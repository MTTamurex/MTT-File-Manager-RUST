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

/// TTL for RAM/VRAM cache (1 second)
const STATUS_CACHE_TTL_MS: u64 = 1000;

/// Returns cached RAM usage, refreshing only after TTL expires.
fn get_ram_usage_cached() -> Option<u64> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last = CACHED_RAM_TIMESTAMP.load(Ordering::Relaxed);
    if now.saturating_sub(last) > STATUS_CACHE_TTL_MS {
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
fn get_vram_usage_cached(texture_cache: &LruCache<PathBuf, egui::TextureHandle>) -> usize {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last = CACHED_VRAM_TIMESTAMP.load(Ordering::Relaxed);
    if now.saturating_sub(last) > STATUS_CACHE_TTL_MS {
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

/// Status bar action that needs to be handled by the caller
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StatusBarAction {
    /// Sort mode or direction changed
    SortChanged,
    /// View mode changed
    ViewModeChanged,
    /// Open virtual drive settings
    OpenVirtualDriveSettings,
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
            if ui
                .button(egui::RichText::new("⚙").color(egui::Color32::BLACK))
                .on_hover_text("Configurar otimização de drives virtuais")
                .clicked()
            {
                action = StatusBarAction::OpenVirtualDriveSettings;
            }

            // === BULK THUMBNAIL SCAN button ===
            if let Some((done, total)) = bulk_progress {
                ui.spinner();
                ui.label(
                    egui::RichText::new(format!("{}/{}", done, total))
                        .color(egui::Color32::BLACK)
                        .small()
                );
            } else if !is_computer_view
                && ui
                    .button(egui::RichText::new("🖼").color(egui::Color32::BLACK))
                    .on_hover_text("Gerar thumbnails para todas as subpastas")
                    .clicked()
            {
                action = StatusBarAction::BulkThumbnailScan;
            }

            ui.add(egui::Separator::default().grow(6.0));

            // === SHOW HIDDEN FILES TOGGLE ===
            {
                let should_disable_show_hidden = is_computer_view || is_recycle_bin_view;
                let tooltip = if *show_hidden_files {
                    "Esconder itens ocultos"
                } else {
                    "Exibir itens ocultos"
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
                        tooltip,
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
                ui.spinner();
                ui.label("Carregando...");
            } else {
                let item_text = if total_items == 1 {
                    "1 item".to_string()
                } else {
                    format!("{} itens", total_items)
                };
                ui.label(item_text);
            }

            ui.separator();

            // === CENTER: View mode (disabled when folder is locked) ===
            ui.scope(|ui| {
                if folder_locked { ui.disable(); }
                ui.label("Modo:");
                if ui
                    .selectable_label(*view_mode == ViewMode::Grid, "Grade")
                    .clicked()
                {
                    *view_mode = ViewMode::Grid;
                    action = StatusBarAction::ViewModeChanged;
                }
                if ui
                    .selectable_label(*view_mode == ViewMode::List, "Lista")
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
                ui.label("Ordenar:");

                // PERFORMANCE: Static arrays instead of Vec allocation per frame
                let sort_modes: &[(SortMode, &str)] = if is_computer_view {
                    &[
                        (SortMode::Name, "Nome"),
                        (SortMode::DriveTotalSpace, "Espaço Total"),
                        (SortMode::DriveFreeSpace, "Espaço Livre"),
                    ]
                } else {
                    &[
                        (SortMode::Name, "Nome"),
                        (SortMode::Date, "Data"),
                        (SortMode::Size, "Tamanho"),
                    ]
                };

                for &(mode, label) in sort_modes {
                    if ui.selectable_label(*sort_mode == mode, label).clicked() {
                        if *sort_mode == mode {
                            *sort_descending = !*sort_descending;
                        } else {
                            *sort_mode = mode;
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
                ui.label("Pastas:");
                if ui
                    .selectable_label(*folders_position == FoldersPosition::First, "Início")
                    .on_hover_text("Pastas sempre no topo")
                    .clicked()
                {
                    *folders_position = FoldersPosition::First;
                    action = StatusBarAction::SortChanged;
                }
                if ui
                    .selectable_label(*folders_position == FoldersPosition::Last, "Fim")
                    .on_hover_text("Pastas no final da lista")
                    .clicked()
                {
                    *folders_position = FoldersPosition::Last;
                    action = StatusBarAction::SortChanged;
                }
                if ui
                    .selectable_label(*folders_position == FoldersPosition::Mixed, "Misto")
                    .on_hover_text("Pastas misturadas com arquivos")
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

                // RAM usage (cached with 1s TTL — avoids kernel syscall every frame)
                if let Some(ram_usage) = get_ram_usage_cached() {
                    ui.label(format!("RAM: {}", format_size(ram_usage)));
                }

                // VRAM estimation (cached with 1s TTL — avoids O(n) texture iteration every frame)
                let vram_usage = get_vram_usage_cached(texture_cache);

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
