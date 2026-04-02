//! Status bar rendering for the file manager.
//!
//! This module contains the rendering logic for the application status bar.
//!
//! PERFORMANCE: RAM usage (kernel syscall) and VRAM estimation (O(n) texture iteration)
//! are cached with 1-second TTL to avoid per-frame overhead.

use crate::domain::file_entry::{FoldersPosition, SortMode, ViewMode};
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::theme;
use crate::ui::widgets;
use eframe::egui;
use lru::LruCache;
use rust_i18n::t;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Cached RAM usage value (atomic for cheap per-frame read)
static CACHED_RAM_BYTES: AtomicU64 = AtomicU64::new(0);
/// Last time RAM was queried (ms since epoch, stored as u64)
static CACHED_RAM_TIMESTAMP: AtomicU64 = AtomicU64::new(0);
/// Cached VRAM estimation in bytes
static CACHED_VRAM_BYTES: AtomicU64 = AtomicU64::new(0);
/// Last time VRAM was calculated
static CACHED_VRAM_TIMESTAMP: AtomicU64 = AtomicU64::new(0);

// --- Kernel resource monitoring (GDI Objects, USER Objects, Handle Count) ---
// These metrics make resource leaks visible at runtime.
// Previously, COM refcount leaks and thread-pool accumulation were invisible
// because Task Manager doesn't show per-process GDI/USER/handle counts by default.
static CACHED_GDI_OBJECTS: AtomicU64 = AtomicU64::new(0);
static CACHED_USER_OBJECTS: AtomicU64 = AtomicU64::new(0);
static CACHED_HANDLE_COUNT: AtomicU64 = AtomicU64::new(0);
static CACHED_THREAD_COUNT: AtomicU64 = AtomicU64::new(0);
static CACHED_PEAK_THREAD_COUNT: AtomicU64 = AtomicU64::new(0);
static LAST_THREAD_WARN_TIMESTAMP: AtomicU64 = AtomicU64::new(0);
static LAST_THREAD_WARN_COUNT: AtomicU64 = AtomicU64::new(0);
static LAST_THREAD_CRITICAL_TIMESTAMP: AtomicU64 = AtomicU64::new(0);
static THREAD_CRITICAL_STREAK: AtomicU64 = AtomicU64::new(0);
/// Set by the UI thread so the background kernel monitor can suppress false
/// thread-count warnings during video playback.
static VIDEO_PREVIEW_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// TTL for RAM/VRAM cache (1 second)
const STATUS_CACHE_TTL_MS: u64 = 1000;
/// Interval for the background kernel metrics thread. CreateToolhelp32Snapshot
/// enumerates ALL system threads and can take 40-150ms, so it must never run on
/// the UI thread. 5 seconds is sufficient for leak detection.
const KERNEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
/// Minimum interval between repeated HIGH thread warnings for stable counts.
const THREAD_WARN_COOLDOWN_MS: u64 = 30_000;
/// Number of consecutive 1s samples above critical threshold before escalating.
const THREAD_CRITICAL_STREAK_MIN: u64 = 6;

fn should_emit_thread_critical(
    now_ms: u64,
    thread_count: u32,
    critical_threshold: u32,
    stalled_onedrive_workers: usize,
) -> bool {
    // If OneDrive workers are stalled, escalate immediately.
    if stalled_onedrive_workers > 0 {
        LAST_THREAD_CRITICAL_TIMESTAMP.store(now_ms, Ordering::Relaxed);
        THREAD_CRITICAL_STREAK.store(1, Ordering::Relaxed);
        return true;
    }

    // Very large overshoot is suspicious even if transient.
    if thread_count >= critical_threshold.saturating_add(25) {
        LAST_THREAD_CRITICAL_TIMESTAMP.store(now_ms, Ordering::Relaxed);
        THREAD_CRITICAL_STREAK.store(1, Ordering::Relaxed);
        return true;
    }

    let last_ts = LAST_THREAD_CRITICAL_TIMESTAMP.load(Ordering::Relaxed);
    let streak = if now_ms.saturating_sub(last_ts) <= STATUS_CACHE_TTL_MS + 500 {
        THREAD_CRITICAL_STREAK
            .load(Ordering::Relaxed)
            .saturating_add(1)
    } else {
        1
    };

    LAST_THREAD_CRITICAL_TIMESTAMP.store(now_ms, Ordering::Relaxed);
    THREAD_CRITICAL_STREAK.store(streak, Ordering::Relaxed);
    streak >= THREAD_CRITICAL_STREAK_MIN
}

fn should_emit_thread_warning(now_ms: u64, thread_count: u32) -> bool {
    let last_ts = LAST_THREAD_WARN_TIMESTAMP.load(Ordering::Relaxed);
    let last_count = LAST_THREAD_WARN_COUNT.load(Ordering::Relaxed) as u32;

    // Always emit when count grows meaningfully (>= +4 threads).
    if thread_count >= last_count.saturating_add(4) {
        LAST_THREAD_WARN_TIMESTAMP.store(now_ms, Ordering::Relaxed);
        LAST_THREAD_WARN_COUNT.store(thread_count as u64, Ordering::Relaxed);
        return true;
    }

    // For stable/slightly changing counts, rate-limit warning spam.
    if now_ms.saturating_sub(last_ts) >= THREAD_WARN_COOLDOWN_MS {
        LAST_THREAD_WARN_TIMESTAMP.store(now_ms, Ordering::Relaxed);
        LAST_THREAD_WARN_COUNT.store(thread_count as u64, Ordering::Relaxed);
        return true;
    }

    false
}

/// Returns cached RAM usage, refreshing only after TTL expires.
fn get_ram_usage_cached(allow_refresh: bool) -> Option<u64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last = CACHED_RAM_TIMESTAMP.load(Ordering::Relaxed);
    if allow_refresh && now.saturating_sub(last) > STATUS_CACHE_TTL_MS {
        if let Some(ram) = get_ram_usage() {
            CACHED_RAM_BYTES.store(ram, Ordering::Relaxed);
            CACHED_RAM_TIMESTAMP.store(now, Ordering::Relaxed);
            return Some(ram);
        }
    }
    let cached = CACHED_RAM_BYTES.load(Ordering::Relaxed);
    if cached > 0 {
        Some(cached)
    } else {
        None
    }
}

/// Returns cached VRAM estimation, refreshing only after TTL expires.
fn get_vram_usage_cached(
    texture_cache: &LruCache<PathBuf, egui::TextureHandle>,
    allow_refresh: bool,
) -> usize {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last = CACHED_VRAM_TIMESTAMP.load(Ordering::Relaxed);
    if allow_refresh && now.saturating_sub(last) > STATUS_CACHE_TTL_MS {
        let vram: usize = texture_cache
            .iter()
            .map(|(_, tex)| {
                let size = tex.size();
                size[0] * size[1] * 4
            })
            .sum();
        CACHED_VRAM_BYTES.store(vram as u64, Ordering::Relaxed);
        CACHED_VRAM_TIMESTAMP.store(now, Ordering::Relaxed);
        vram
    } else {
        CACHED_VRAM_BYTES.load(Ordering::Relaxed) as usize
    }
}

/// Kernel resource metrics for runtime leak detection.
struct KernelResourceMetrics {
    gdi_objects: u32,
    user_objects: u32,
    handle_count: u32,
    thread_count: u32,
    peak_thread_count: u32,
}

/// Returns cached kernel resource metrics. The actual system queries run on a
/// dedicated background thread (spawned once) so the UI thread never blocks on
/// `CreateToolhelp32Snapshot` or `GetGuiResources`.
fn get_kernel_resources_cached(_allow_refresh: bool, video_preview_active: bool) -> KernelResourceMetrics {
    // Communicate video state to the background thread via atomic.
    VIDEO_PREVIEW_ACTIVE.store(video_preview_active, Ordering::Relaxed);

    // Ensure the background poller is running (spawned exactly once).
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        std::thread::Builder::new()
            .name("kernel-metrics".into())
            .spawn(kernel_metrics_poller)
            .ok();
    });

    KernelResourceMetrics {
        gdi_objects: CACHED_GDI_OBJECTS.load(Ordering::Relaxed) as u32,
        user_objects: CACHED_USER_OBJECTS.load(Ordering::Relaxed) as u32,
        handle_count: CACHED_HANDLE_COUNT.load(Ordering::Relaxed) as u32,
        thread_count: CACHED_THREAD_COUNT.load(Ordering::Relaxed) as u32,
        peak_thread_count: CACHED_PEAK_THREAD_COUNT.load(Ordering::Relaxed) as u32,
    }
}

/// Background thread that periodically queries kernel resource metrics.
/// Runs forever with a sleep interval, keeping the expensive syscalls off the UI thread.
fn kernel_metrics_poller() {
    loop {
        std::thread::sleep(KERNEL_POLL_INTERVAL);

        let metrics = get_kernel_resources();
        CACHED_GDI_OBJECTS.store(metrics.gdi_objects as u64, Ordering::Relaxed);
        CACHED_USER_OBJECTS.store(metrics.user_objects as u64, Ordering::Relaxed);
        CACHED_HANDLE_COUNT.store(metrics.handle_count as u64, Ordering::Relaxed);
        CACHED_THREAD_COUNT.store(metrics.thread_count as u64, Ordering::Relaxed);

        let prev_peak = CACHED_PEAK_THREAD_COUNT.load(Ordering::Relaxed) as u32;
        if metrics.thread_count > prev_peak {
            CACHED_PEAK_THREAD_COUNT.store(metrics.thread_count as u64, Ordering::Relaxed);
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let cpu_count = std::thread::available_parallelism()
            .map(|n| n.get() as u32)
            .unwrap_or(4);
        let expected_baseline = cpu_count * 2 + cpu_count.min(16) + cpu_count.min(8) + cpu_count.min(6) + 28;
        let warn_threshold = expected_baseline + 20;
        let critical_threshold = expected_baseline + 50;
        let stalled = crate::infrastructure::onedrive::get_stalled_io_workers();
        let video_active = VIDEO_PREVIEW_ACTIVE.load(Ordering::Relaxed);

        if metrics.thread_count >= critical_threshold {
            if is_video_preview_thread_spike_expected(video_active, stalled) {
                THREAD_CRITICAL_STREAK.store(0, Ordering::Relaxed);
            } else if should_emit_thread_critical(now, metrics.thread_count, critical_threshold, stalled)
            {
                log::error!(
                    "[THREAD MONITOR] CRITICAL: {} threads (baseline: ~{}, peak: {}, stalled OneDrive I/O: {}). \
                     Possible sustained thread leak causing system-wide stalls.",
                    metrics.thread_count,
                    expected_baseline,
                    prev_peak.max(metrics.thread_count),
                    stalled
                );
                LAST_THREAD_WARN_TIMESTAMP.store(now, Ordering::Relaxed);
                LAST_THREAD_WARN_COUNT.store(metrics.thread_count as u64, Ordering::Relaxed);
            } else if should_emit_thread_warning(now, metrics.thread_count) {
                log::warn!(
                    "[THREAD MONITOR] HIGH transient thread count: {} (baseline: ~{}, peak: {}, stalled OneDrive I/O: {})",
                    metrics.thread_count,
                    expected_baseline,
                    prev_peak.max(metrics.thread_count),
                    stalled
                );
            }
        } else if metrics.thread_count >= warn_threshold {
            THREAD_CRITICAL_STREAK.store(0, Ordering::Relaxed);
            if !is_video_preview_thread_spike_expected(video_active, stalled)
                && should_emit_thread_warning(now, metrics.thread_count)
            {
                log::warn!(
                    "[THREAD MONITOR] HIGH thread count: {} (baseline: ~{}, peak: {}, stalled OneDrive I/O: {})",
                    metrics.thread_count, expected_baseline,
                    prev_peak.max(metrics.thread_count), stalled
                );
            }
        } else {
            THREAD_CRITICAL_STREAK.store(0, Ordering::Relaxed);
        }
    }
}

fn is_video_preview_thread_spike_expected(video_preview_active: bool, stalled_onedrive_workers: usize) -> bool {
    video_preview_active && stalled_onedrive_workers == 0
}

/// Queries GDI Objects, USER Objects, and Handle Count for the current process.
fn get_kernel_resources() -> KernelResourceMetrics {
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Threading::GetCurrentProcess;

    // GetGuiResources is not exposed in windows 0.61 via WindowsAndMessaging,
    // so we link directly against user32.dll.
    const GR_GDIOBJECTS: u32 = 0;
    const GR_USEROBJECTS: u32 = 1;

    extern "system" {
        fn GetGuiResources(hprocess: *mut core::ffi::c_void, uiflags: u32) -> u32;
    }

    unsafe {
        let process = GetCurrentProcess();
        let handle_ptr = process.0 as *mut core::ffi::c_void;
        let gdi = GetGuiResources(handle_ptr, GR_GDIOBJECTS);
        let user = GetGuiResources(handle_ptr, GR_USEROBJECTS);

        let mut handles: u32 = 0;
        let process_handle = HANDLE(process.0);
        let _ = windows::Win32::System::Threading::GetProcessHandleCount(
            process_handle,
            &mut handles,
        );

        // Thread count via CreateToolhelp32Snapshot — the only reliable way
        // to count OS threads, since detached Rust threads (JoinHandle dropped)
        // are invisible to GetProcessHandleCount but still consume kernel
        // thread objects and can block the cloud filter driver.
        let thread_count = count_process_threads();

        KernelResourceMetrics {
            gdi_objects: gdi,
            user_objects: user,
            handle_count: handles,
            thread_count,
            peak_thread_count: 0, // filled by caller
        }
    }
}

/// Counts live OS threads for the current process using Toolhelp32Snapshot.
/// This is the only reliable way to detect detached/leaked threads that are
/// invisible to Rust's `JoinHandle` tracking and `GetProcessHandleCount`.
fn count_process_threads() -> u32 {
    // The `Win32_System_Diagnostics` feature is not enabled in our windows crate,
    // so we link directly against kernel32 for the Toolhelp32 API.
    #[repr(C)]
    #[allow(non_snake_case)]
    struct THREADENTRY32 {
        dwSize: u32,
        cntUsage: u32,
        th32ThreadID: u32,
        th32OwnerProcessID: u32,
        tpBasePri: i32,
        tpDeltaPri: i32,
        dwFlags: u32,
    }

    const TH32CS_SNAPTHREAD: u32 = 0x00000004;

    extern "system" {
        fn CreateToolhelp32Snapshot(dwflags: u32, th32processid: u32) -> isize;
        fn Thread32First(hsnapshot: isize, lpte: *mut THREADENTRY32) -> i32;
        fn Thread32Next(hsnapshot: isize, lpte: *mut THREADENTRY32) -> i32;
    }

    unsafe {
        let pid = std::process::id();
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == -1 {
            return 0;
        }

        let mut entry = std::mem::zeroed::<THREADENTRY32>();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;

        let mut count: u32 = 0;
        if Thread32First(snapshot, &mut entry) != 0 {
            loop {
                if entry.th32OwnerProcessID == pid {
                    count += 1;
                }
                entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
                if Thread32Next(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }

        windows::Win32::Foundation::CloseHandle(
            windows::Win32::Foundation::HANDLE(snapshot as *mut core::ffi::c_void),
        ).ok();
        count
    }
}

/// Status bar action that needs to be handled by the caller
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StatusBarAction {
    /// Sort mode or direction changed
    SortChanged,
    /// View mode changed
    ViewModeChanged,
    /// Open virtual drive settings
    OpenVirtualDriveSettings,
    /// Open language settings
    OpenLanguageSettings,
    /// Start bulk thumbnail extraction for current folder and subfolders
    BulkThumbnailScan,
    /// Show/hide hidden files toggled
    ShowHiddenChanged,
    /// No action
    None,
}

/// Renders the application status bar.
/// Returns an action that needs to be handled by the caller.
#[allow(clippy::too_many_arguments)]
pub fn render_status_bar(
    ui: &mut egui::Ui,
    svg_manager: &mut SvgIconManager,
    is_loading_folder: &mut bool,
    total_items: usize,
    view_mode: &mut ViewMode,
    sort_mode: &mut SortMode,
    sort_descending: &mut bool,
    folders_position: &mut FoldersPosition,
    texture_cache: &LruCache<PathBuf, egui::TextureHandle>,
    _frame_time_avg_ms: f32,
    _frame_time_peak_ms: f32,
    _fps_avg: f32,
    _upload_budget_ms: f32,
    is_computer_view: bool,
    is_recycle_bin_view: bool,
    bulk_progress: Option<(usize, usize)>,
    folder_locked: bool,
    show_hidden_files: &mut bool,
    allow_system_refresh: bool,
    video_preview_active: bool,
) -> StatusBarAction {
    let mut action = StatusBarAction::None;

    ui.scope(|ui| {
        let hover_color = if ui.visuals().dark_mode {
            theme::color_dark_hover()
        } else {
            theme::color_hover()
        };
        let selection_stroke = egui::Stroke::new(0.0, theme::COLOR_SELECTION_TEXT);

        ui.visuals_mut().selection.bg_fill = theme::COLOR_SELECTION;
        ui.visuals_mut().selection.stroke = selection_stroke;
        ui.visuals_mut().override_text_color = Some(egui::Color32::BLACK);
        ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
        ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
        ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
        ui.visuals_mut().widgets.inactive.fg_stroke = egui::Stroke::NONE;
        ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
        ui.visuals_mut().widgets.hovered.weak_bg_fill = hover_color;
        ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;
        ui.visuals_mut().widgets.hovered.fg_stroke = egui::Stroke::NONE;
        ui.visuals_mut().widgets.active.bg_fill = hover_color;
        ui.visuals_mut().widgets.active.weak_bg_fill = hover_color;
        ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;
        ui.visuals_mut().widgets.active.fg_stroke = egui::Stroke::NONE;

        ui.horizontal(|ui| {
            // === LEFTMOST: Virtual drive settings button ===
            if widgets::toggle_icon_button_sized(
                ui,
                svg_manager,
                "settings",
                false,
                &t!("status_bar.vdrive_settings"),
                theme::ICON_SIZE_SM,
                1.0,
                0.75,
            )
            .clicked()
            {
                action = StatusBarAction::OpenVirtualDriveSettings;
            }

            // === LANGUAGE SETTINGS button ===
            if widgets::toggle_icon_button_sized(
                ui,
                svg_manager,
                "languages",
                false,
                &t!("settings.language"),
                theme::ICON_SIZE_SM,
                1.0,
                0.75,
            )
            .clicked()
            {
                action = StatusBarAction::OpenLanguageSettings;
            }

            // === BULK THUMBNAIL SCAN button ===
            if let Some((done, total)) = bulk_progress {
                ui.label(
                    egui::RichText::new(t!("status_bar.processing", done = done, total = total))
                        .color(egui::Color32::BLACK)
                        .small()
                );
            } else if !is_computer_view
                && widgets::toggle_icon_button_sized(
                    ui,
                    svg_manager,
                    "image",
                    false,
                    &t!("status_bar.bulk_thumbnails"),
                    theme::ICON_SIZE_SM,
                    1.0,
                    0.75,
                )
                .clicked()
            {
                action = StatusBarAction::BulkThumbnailScan;
            }

            ui.add(egui::Separator::default().grow(6.0));

            // === SHOW HIDDEN FILES TOGGLE ===
            {
                let should_disable_show_hidden = is_computer_view || is_recycle_bin_view;
                let tooltip = if *show_hidden_files {
                    t!("status_bar.hidden_hide")
                } else {
                    t!("status_bar.hidden_show")
                };
                ui.scope(|ui| {
                    if should_disable_show_hidden {
                        ui.disable();
                    }

                    if widgets::toggle_icon_button_sized(
                        ui,
                        svg_manager,
                        "eye",
                        *show_hidden_files,
                        &tooltip,
                        theme::ICON_SIZE_MD - 2.0,
                        2.0,
                        -1.0,
                    )
                    .clicked()
                    {
                        *show_hidden_files = !*show_hidden_files;
                        action = StatusBarAction::ShowHiddenChanged;
                    }
                });
            }

            ui.separator();

            // Wrap text items in a Frame with asymmetric bottom margin.
            // This shifts content UP by ~0.5px without changing the row height
            // (because buttons/eye are taller, so the Frame never becomes the
            // tallest element → no coupling / vicious circle).
            egui::Frame::NONE
                .inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 2 })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {

            // === LEFT SIDE: Item count and loading status ===
            if *is_loading_folder {
                ui.label(t!("status_bar.loading").to_string());
            } else {
                let item_text = if total_items == 1 {
                    t!("status_bar.item_one").to_string()
                } else {
                    t!("status_bar.item_many", count = total_items).to_string()
                };
                ui.label(item_text);
            }

            ui.separator();

            // === CENTER: View mode (disabled when folder is locked) ===
            ui.scope(|ui| {
                if folder_locked { ui.disable(); }
                ui.label(t!("status_bar.mode").to_string());
                if ui
                    .selectable_label(*view_mode == ViewMode::Grid, t!("status_bar.grid").to_string())
                    .clicked()
                {
                    *view_mode = ViewMode::Grid;
                    action = StatusBarAction::ViewModeChanged;
                }
                if ui
                    .selectable_label(*view_mode == ViewMode::List, t!("status_bar.list").to_string())
                    .clicked()
                {
                    *view_mode = ViewMode::List;
                    action = StatusBarAction::ViewModeChanged;
                }
            });

            ui.separator();

            // === CENTER-RIGHT: Sort controls (disabled when folder is locked) ===
            ui.scope(|ui| {
                if folder_locked { ui.disable(); }
                ui.label(t!("status_bar.sort").to_string());

                // PERFORMANCE: Build sort mode pairs per frame (i18n requires runtime lookup)
                let sort_modes: Vec<(SortMode, String)> = if is_computer_view {
                    vec![
                        (SortMode::Name, t!("status_bar.sort_name").to_string()),
                        (SortMode::DriveTotalSpace, t!("status_bar.sort_total_space").to_string()),
                        (SortMode::DriveFreeSpace, t!("status_bar.sort_free_space").to_string()),
                    ]
                } else {
                    vec![
                        (SortMode::Name, t!("status_bar.sort_name").to_string()),
                        (SortMode::Date, t!("status_bar.sort_date").to_string()),
                        (SortMode::Size, t!("status_bar.sort_size").to_string()),
                    ]
                };

                for (mode, label) in &sort_modes {
                    if ui.selectable_label(*sort_mode == *mode, label.as_str()).clicked() {
                        if *sort_mode == *mode {
                            *sort_descending = !*sort_descending;
                        } else {
                            *sort_mode = *mode;
                            *sort_descending = false;
                        }
                        action = StatusBarAction::SortChanged;
                    }
                }

                // Sort direction indicator
                let arrow = if *sort_descending { "↓" } else { "↑" };
                ui.label(arrow);
            });

            ui.separator();

            ui.scope(|ui| {
                if folder_locked { ui.disable(); }
                ui.label(t!("status_bar.folders").to_string());
                if ui
                    .selectable_label(*folders_position == FoldersPosition::First, t!("status_bar.folders_first").to_string())
                    .on_hover_text(t!("status_bar.folders_first_hint"))
                    .clicked()
                {
                    *folders_position = FoldersPosition::First;
                    action = StatusBarAction::SortChanged;
                }
                if ui
                    .selectable_label(*folders_position == FoldersPosition::Last, t!("status_bar.folders_last").to_string())
                    .on_hover_text(t!("status_bar.folders_last_hint"))
                    .clicked()
                {
                    *folders_position = FoldersPosition::Last;
                    action = StatusBarAction::SortChanged;
                }
                if ui
                    .selectable_label(*folders_position == FoldersPosition::Mixed, t!("status_bar.folders_mixed").to_string())
                    .on_hover_text(t!("status_bar.folders_mixed_hint"))
                    .clicked()
                {
                    *folders_position = FoldersPosition::Mixed;
                    action = StatusBarAction::SortChanged;
                }
            });

                    }); // end text items horizontal
                }); // end Frame

            ui.separator();

            // === RIGHT SIDE: System info (push to right with available space) ===
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label("MTT File Manager");
                ui.separator();

                // Kernel resource monitoring (cached with 1s TTL)
                // These metrics expose handle/GDI/USER leaks at runtime.
                let km = get_kernel_resources_cached(allow_system_refresh, video_preview_active);
                ui.label(&*t!("status_bar_system.threads", count = km.thread_count, peak = km.peak_thread_count));
                ui.label(&*t!("status_bar_system.handles", count = km.handle_count));
                ui.label(&*t!("status_bar_system.gdi", count = km.gdi_objects));
                ui.label(&*t!("status_bar_system.user", count = km.user_objects));
                ui.separator();

                // RAM usage (cached with 1s TTL — avoids kernel syscall every frame)
                if let Some(ram_usage) = get_ram_usage_cached(allow_system_refresh) {
                    ui.label(format!("RAM: {}", format_size(ram_usage)));
                }

                // VRAM estimation (cached with 1s TTL — avoids O(n) texture iteration every frame)
                let vram_usage = get_vram_usage_cached(texture_cache, allow_system_refresh);

                ui.label(format!(
                    "VRAM: {:.1} MB",
                    vram_usage as f64 / 1024.0 / 1024.0
                ));
            });
        });
    });

    action
}

/// Gets the current process RAM usage (RSS/Working Set).
fn get_ram_usage() -> Option<u64> {
    use windows::{
        Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Win32::System::Threading::GetCurrentProcess,
    };

    unsafe {
        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        if K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
        .as_bool()
        {
            Some(counters.WorkingSetSize as u64)
        } else {
            None
        }
    }
}

/// Formats size in bytes to human readable string
fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];

    if bytes == 0 {
        return "0 B".to_string();
    }

    let base = 1024_f64;
    let bytes_f64 = bytes as f64;
    let exponent = (bytes_f64.log10() / base.log10()).floor() as i32;
    let clamped_exponent = exponent.clamp(0, 5);
    let unit_index = clamped_exponent as usize;
    let divisor = base.powi(clamped_exponent);

    format!("{:.1} {}", bytes_f64 / divisor, UNITS[unit_index])
}
