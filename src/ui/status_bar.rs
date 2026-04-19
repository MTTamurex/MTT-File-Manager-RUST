//! Status bar rendering for the file manager.
//!
//! This module contains the rendering logic for the application status bar.
//! Low-level thread/resource monitoring stays in the background, but those
//! process metrics are intentionally not exposed in the UI.

use crate::domain::file_entry::ViewMode;
use crate::ui::svg_icons::SvgIconManager;
use crate::ui::theme;
use crate::ui::widgets;
use eframe::egui;
use rust_i18n::t;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// Shared timing constant for the background monitor.
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

/// Kernel resource metrics for runtime leak detection.
struct KernelResourceMetrics {
    gdi_objects: u32,
    user_objects: u32,
    handle_count: u32,
    thread_count: u32,
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
    let metrics = crate::infrastructure::windows::get_current_process_kernel_resources();

    KernelResourceMetrics {
        gdi_objects: metrics.gdi_objects,
        user_objects: metrics.user_objects,
        handle_count: metrics.handle_count,
        thread_count: metrics.thread_count,
    }
}

/// Status bar action that needs to be handled by the caller
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StatusBarAction {
    /// Open application settings
    OpenSettings,
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
    is_computer_view: bool,
    is_recycle_bin_view: bool,
    bulk_scan_active: bool,
    show_hidden_files: &mut bool,
    view_mode: ViewMode,
    thumbnail_size: &mut f32,
    video_preview_active: bool,
) -> StatusBarAction {
    let mut action = StatusBarAction::None;

    ui.scope(|ui| {
        let hover_color = if ui.visuals().dark_mode {
            theme::color_dark_hover()
        } else {
            theme::color_hover()
        };
        let is_dark = ui.visuals().dark_mode;
        let selection_stroke = egui::Stroke::new(0.0, theme::selection_text_color(is_dark));

        ui.visuals_mut().selection.bg_fill = theme::selection_color(is_dark);
        ui.visuals_mut().selection.stroke = selection_stroke;
        ui.visuals_mut().override_text_color = Some(theme::text_color(is_dark));
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

        // Keep the background thread monitor alive without exposing those
        // low-level process metrics in the status bar.
        let _ = get_kernel_resources_cached(false, video_preview_active);

        ui.horizontal(|ui| {
            // === LEFTMOST: Settings button ===
            if widgets::toggle_icon_button_sized(
                ui,
                svg_manager,
                "settings",
                false,
                &t!("status_bar.settings"),
                theme::ICON_SIZE_SM,
                1.0,
                0.75,
            )
            .clicked()
            {
                action = StatusBarAction::OpenSettings;
            }

            // === BULK THUMBNAIL SCAN button ===
            if !is_computer_view {
                ui.scope(|ui| {
                    if bulk_scan_active {
                        ui.disable();
                    }

                    if widgets::toggle_icon_button_sized(
                        ui,
                        svg_manager,
                        "image",
                        bulk_scan_active,
                        &t!("status_bar.bulk_thumbnails"),
                        theme::ICON_SIZE_SM,
                        1.0,
                        0.75,
                    )
                    .clicked()
                    {
                        action = StatusBarAction::BulkThumbnailScan;
                    }
                });
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
                }); // end Frame

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::Frame::NONE
                    .inner_margin(egui::Margin { left: 0, right: 0, top: 0, bottom: 2 })
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.scope(|ui| {
                            let is_dark = ui.visuals().dark_mode;
                            let slider_track = if is_dark {
                                egui::Color32::from_gray(78)
                            } else {
                                egui::Color32::from_gray(228)
                            };
                            let slider_track_stroke = if is_dark {
                                egui::Stroke::new(1.0, egui::Color32::from_gray(98))
                            } else {
                                egui::Stroke::new(1.0, egui::Color32::from_gray(214))
                            };
                            let slider_fill = if is_dark {
                                egui::Color32::from_rgb(36, 170, 231)
                            } else {
                                egui::Color32::from_rgb(33, 176, 236)
                            };
                            let slider_knob = if is_dark {
                                egui::Color32::from_gray(220)
                            } else {
                                egui::Color32::from_gray(224)
                            };
                            let slider_knob_stroke = if is_dark {
                                egui::Stroke::new(1.0, egui::Color32::from_gray(150))
                            } else {
                                egui::Stroke::new(1.0, egui::Color32::from_gray(190))
                            };
                            let slider_knob_hover = if is_dark {
                                egui::Color32::from_gray(232)
                            } else {
                                egui::Color32::from_gray(234)
                            };

                            ui.visuals_mut().selection.bg_fill = slider_fill;

                            ui.visuals_mut().widgets.noninteractive.bg_fill = slider_track;
                            ui.visuals_mut().widgets.noninteractive.weak_bg_fill = slider_track;
                            ui.visuals_mut().widgets.noninteractive.fg_stroke = slider_knob_stroke;
                            ui.visuals_mut().widgets.noninteractive.bg_stroke = slider_track_stroke;

                            ui.visuals_mut().widgets.inactive.bg_fill = slider_track;
                            ui.visuals_mut().widgets.inactive.weak_bg_fill = slider_track;
                            ui.visuals_mut().widgets.inactive.fg_stroke = slider_knob_stroke;
                            ui.visuals_mut().widgets.inactive.bg_stroke = slider_track_stroke;

                            ui.visuals_mut().widgets.hovered.bg_fill = slider_knob_hover;
                            ui.visuals_mut().widgets.hovered.weak_bg_fill = slider_track;
                            ui.visuals_mut().widgets.hovered.fg_stroke = slider_knob_stroke;
                            ui.visuals_mut().widgets.hovered.bg_stroke = slider_track_stroke;

                            ui.visuals_mut().widgets.active.bg_fill = slider_knob;
                            ui.visuals_mut().widgets.active.weak_bg_fill = slider_track;
                            ui.visuals_mut().widgets.active.fg_stroke = slider_knob_stroke;
                            ui.visuals_mut().widgets.active.bg_stroke = slider_track_stroke;

                            if matches!(view_mode, ViewMode::List) {
                                ui.disable();
                            }

                            ui.label(t!("status_bar.thumbnail_size").to_string());
                            ui.spacing_mut().slider_width = 56.0;
                            ui.spacing_mut().interact_size.y = 14.0;
                            ui.add(
                                egui::Slider::new(thumbnail_size, theme::THUMBNAIL_MIN..=256.0)
                                    .show_value(false),
                            );
                        });
                    });
            });
        });
    });

    action
}
