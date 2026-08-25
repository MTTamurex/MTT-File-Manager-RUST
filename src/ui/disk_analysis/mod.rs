//! Disk usage analyzer full view (NTFS volumes, backed by the MTT Search
//! Service USN/MFT index). Layout inspired by modern disk analyzers: header
//! with drive summary and scan action, left sidebar with drives/file types,
//! squarified treemap with drill-down breadcrumbs, and a stats footer.
//!
//! Rendered by the standalone analyzer process (`crate::disk_analyzer`),
//! which runs as its own OS process (same model as the dedicated viewers)
//! so it gets an independent taskbar button and minimize/restore lifecycle.

mod duplicates;
mod efficiency;
mod largest_items;
mod results_panel;
mod sidebar;
pub(crate) mod toolbar;
pub(crate) mod treemap;

pub(crate) use results_panel::{attach_row_menu, paint_cell, HEADER_ROW_H, ROW_HEIGHT};

use crate::app::disk_analysis_model::FileCategory;
use crate::app::disk_analysis_query::SizeMetric;
use crate::app::disk_analysis_state::{DiskAnalysisPhase, DiskAnalysisState};
use crate::infrastructure::windows::formatting::format_size;
use eframe::egui;
use rust_i18n::t;

/// The analyzer viewport keeps the native Windows caption; mirror the app
/// theme onto it via DWM immersive dark mode (same as the dedicated viewers).
#[cfg(target_os = "windows")]
fn sync_native_title_bar(state: &mut DiskAnalysisState, dark: bool) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, IsWindow};

    let cached = state.viewport_hwnd.map(|raw| HWND(raw as *mut _));
    let cached = match cached {
        Some(hwnd) if unsafe { IsWindow(Some(hwnd)) }.as_bool() => Some(hwnd),
        _ => {
            // Window was destroyed (or never found): force re-lookup.
            state.viewport_hwnd = None;
            state.viewport_title_bar_dark = None;
            None
        }
    };
    let hwnd = cached.or_else(|| {
        // Same title-lookup hack the main window uses (eframe exposes no
        // HWND for viewports). Retries each frame until the window exists.
        let title: Vec<u16> = format!("{}\0", t!("disk_analysis.title"))
            .encode_utf16()
            .collect();
        let found = unsafe { FindWindowW(None, PCWSTR(title.as_ptr())) }
            .ok()
            .filter(|h| !h.is_invalid());
        if let Some(h) = found {
            state.viewport_hwnd = Some(h.0 as isize);
        }
        state.viewport_title_bar_dark = None;
        found
    });
    if let Some(hwnd) = hwnd {
        if state.viewport_title_bar_dark != Some(dark) {
            crate::infrastructure::windows::window_corners::apply_dark_title_bar(hwnd, dark);
            state.viewport_title_bar_dark = Some(dark);
        }
    }
}

/// Category palette based on the Okabe-Ito colorblind-safe set: muted but
/// maximally separated in hue/lightness so slices stay distinguishable in
/// both themes (including after the light-mode darkening).
pub fn category_color(category: FileCategory, dark: bool) -> egui::Color32 {
    let (r, g, b) = match category {
        FileCategory::Video => (0, 114, 178),
        FileCategory::Images => (204, 121, 167),
        FileCategory::Audio => (230, 159, 0),
        FileCategory::Archives => (240, 228, 66),
        FileCategory::Code => (86, 180, 233),
        FileCategory::Documents => (0, 158, 110),
        FileCategory::System => (213, 94, 0),
        FileCategory::Other => (150, 150, 150),
    };
    if dark {
        egui::Color32::from_rgb(r, g, b)
    } else {
        // Slightly darker tones keep text readable on light backgrounds.
        egui::Color32::from_rgb(
            (r as f32 * 0.82) as u8,
            (g as f32 * 0.82) as u8,
            (b as f32 * 0.82) as u8,
        )
    }
}

fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let lerp = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    egui::Color32::from_rgba_unmultiplied(
        lerp(a.r(), b.r()),
        lerp(a.g(), b.g()),
        lerp(a.b(), b.b()),
        lerp(a.a(), b.a()),
    )
}

pub(crate) fn analyzer_text_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        ui.visuals().text_color()
    } else {
        ui.visuals().strong_text_color()
    }
}

pub(crate) fn panel_secondary_text_color(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        ui.visuals().weak_text_color()
    } else {
        analyzer_text_color(ui)
    }
}

/// Render one analyzer frame into `ui`. The standalone process wraps this
/// in its root panel; close/Escape are handled by that process.
pub fn render_analyzer_body(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();

    #[cfg(target_os = "windows")]
    sync_native_title_bar(state, ctx.global_style().visuals.dark_mode);

    if state.poll() {
        ctx.request_repaint();
    }
    // Schedule filter/top-K/search jobs whose inputs changed (debounced).
    state.sync_query_jobs();

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
        .show(ui, |ui| {
            if !ui.visuals().dark_mode {
                let color = analyzer_text_color(ui);
                ui.visuals_mut().override_text_color = Some(color);
            }
            render_header(state, ui);
            render_footer(state, ui);
            // Added after the footer so the results panel sits above the
            // category legend.
            results_panel::render_results_panel(state, ui);

            egui::Panel::left(egui::Id::new("disk_analysis_sidebar"))
                .resizable(false)
                .exact_size(250.0)
                .frame(
                    egui::Frame::NONE
                        .fill(ui.visuals().panel_fill)
                        .inner_margin(10.0),
                )
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("disk_analysis_sidebar_scroll")
                        .show(ui, |ui| sidebar::render_sidebar(state, ui));
                });

            egui::CentralPanel::default()
                .frame(
                    egui::Frame::NONE
                        .fill(ui.visuals().panel_fill)
                        .inner_margin(8.0),
                )
                .show(ui, |ui| {
                    toolbar::render_toolbar(state, ui);
                    render_breadcrumb(state, ui);
                    ui.add_space(4.0);
                    render_treemap(state, ui);
                });
        });
    // Toolbar edits happen inside the panel above, so deadlines must be read
    // after rendering. Poll only while a worker is actually outstanding.
    if let Some(delay) = state.query_repaint_after() {
        ctx.request_repaint_after(delay);
    }
    if state.has_background_work() {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

fn render_header(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    egui::Panel::top(egui::Id::new("disk_analysis_header"))
        .show_separator_line(false)
        .frame(
            egui::Frame::NONE
                .fill(ui.visuals().panel_fill)
                .inner_margin(egui::Margin::same(12)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let letter = state.drive_letter;
                if let Some(letter) = letter {
                    ui.label(
                        egui::RichText::new(format!("{letter}:"))
                            .strong()
                            .size(15.0),
                    );
                    let label = state
                        .drives
                        .iter()
                        .find(|d| d.letter == letter)
                        .map(|d| d.label.clone())
                        .unwrap_or_default();
                    if !label.is_empty() {
                        ui.label(label);
                    }
                    ui.label(egui::RichText::new("— NTFS").color(panel_secondary_text_color(ui)));
                } else {
                    ui.label(
                        egui::RichText::new(t!("disk_analysis.title").to_string())
                            .strong()
                            .size(15.0),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let fetching = state.phase == DiskAnalysisPhase::Fetching;
                    // Same refresh icon button as the main app toolbar.
                    if fetching {
                        ui.spinner();
                    } else if letter.is_some()
                        && crate::ui::widgets::icon_button(
                            ui,
                            &mut state.svg_icons,
                            crate::ui::theme::ICON_REFRESH,
                            t!("disk_analysis.rescan").as_ref(),
                            None,
                        )
                        .clicked()
                    {
                        if let Some(letter) = letter {
                            state.request(letter);
                        }
                    }
                    ui.add_space(10.0);

                    // Scan duration, next to the rescan button.
                    if let Some(elapsed) = state.fetch_elapsed {
                        ui.label(
                            egui::RichText::new(
                                t!(
                                    "disk_analysis.scan_time",
                                    secs = format!("{:.2}", elapsed.as_secs_f64())
                                )
                                .to_string(),
                            )
                            .color(panel_secondary_text_color(ui)),
                        );
                    }
                });
            });
            ui.separator();
        });
}

fn render_footer(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    egui::Panel::bottom(egui::Id::new("disk_analysis_footer"))
        .show_separator_line(false)
        .frame(
            egui::Frame::NONE
                .fill(ui.visuals().panel_fill)
                .inner_margin(egui::Margin::symmetric(12, 6)),
        )
        .show(ui, |ui| {
            ui.separator();
            ui.horizontal(|ui| {
                let Some(model) = state.model.clone() else {
                    return;
                };
                // File-type legend (moved here from the sidebar), following
                // the active metric.
                let dark = ui.visuals().dark_mode;
                let totals = model.category_totals.slice_for(state.metric);
                let total = totals.iter().sum::<u64>().max(1);
                for category in FileCategory::ALL {
                    let bytes = totals[category.index()];
                    if bytes == 0 {
                        continue;
                    }
                    let percent = (bytes as f64 / total as f64) * 100.0;
                    let (dot_rect, _) =
                        ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        dot_rect.center(),
                        4.0,
                        category_color(category, dark),
                    );
                    ui.add_space(4.0);
                    ui.label(category_label(category));
                    ui.label(
                        egui::RichText::new(format!("{percent:.0}%"))
                            .color(panel_secondary_text_color(ui)),
                    );
                    ui.add_space(12.0);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "{}: {}",
                            t!("disk_analysis.deepest_path"),
                            model.deepest_path
                        ))
                        .color(panel_secondary_text_color(ui)),
                    );
                });
            });
        });
}

pub fn category_label(category: FileCategory) -> String {
    let key = match category {
        FileCategory::Video => "disk_analysis.category_video",
        FileCategory::Images => "disk_analysis.category_images",
        FileCategory::Audio => "disk_analysis.category_audio",
        FileCategory::Archives => "disk_analysis.category_archives",
        FileCategory::Code => "disk_analysis.category_code",
        FileCategory::Documents => "disk_analysis.category_documents",
        FileCategory::System => "disk_analysis.category_system",
        FileCategory::Other => "disk_analysis.category_other",
    };
    t!(key).to_string()
}

fn render_breadcrumb(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let Some(model) = state.model.clone() else {
        return;
    };
    let stack = state.drill_stack.clone();
    // Segment target encodes the stack position so a click truncates the
    // drill trail exactly like the main app's address bar navigation.
    let segments: Vec<(String, String)> = stack
        .iter()
        .enumerate()
        .map(|(i, &idx)| (model.nodes[idx as usize].name.clone(), i.to_string()))
        .collect();
    if let Some(target) = crate::ui::components::breadcrumb::render_breadcrumb_trail(ui, &segments)
    {
        if let Ok(pos) = target.parse::<usize>() {
            if pos + 1 < stack.len() {
                state.drill_stack.truncate(pos + 1);
                state.hovered = None;
                ui.ctx().request_repaint();
            }
        }
    }
}

fn render_treemap(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let phase = state.phase;
    let model = state.model.clone();

    let Some(model) = model else {
        render_center_state(state, ui, phase);
        return;
    };

    if phase == DiskAnalysisPhase::Fetching {
        // Subtle refresh indicator while a new snapshot is in flight.
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.spinner();
            ui.label(
                egui::RichText::new(t!("disk_analysis.loading").to_string())
                    .color(panel_secondary_text_color(ui)),
            );
        });
    }

    let current = *state.drill_stack.last().unwrap_or(&model.root);
    let avail = ui.available_rect_before_wrap();
    if avail.width() < 16.0 || avail.height() < 16.0 {
        return;
    }
    let (rect, resp) = ui.allocate_exact_size(avail.size(), egui::Sense::click());
    let area = rect.shrink(4.0);

    if state.filter_is_updating() {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            ui.centered_and_justified(|ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(t!("disk_analysis.filter_updating").to_string());
                });
            });
        });
        state.hovered = None;
        return;
    }

    // Filtered weights apply only while they exist; the metric drives the
    // plain layout otherwise.
    let weights = state.active_weights.clone();
    let weights_ref = weights.as_ref();

    // Explicit empty state instead of an area-less void when the whole
    // subtree weighs zero under the active metric/filter combination.
    let root_value = treemap::tile_value(&model, current, state.metric, weights_ref);
    if root_value == 0 {
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new(t!("disk_analysis.empty_weight_state").to_string())
                    .color(panel_secondary_text_color(ui)),
            );
        });
        state.hovered = None;
        return;
    }

    let placed = state
        .treemap_cache
        .get(&model, current, area, state.metric, weights_ref);

    let dark = ui.visuals().dark_mode;
    let panel = ui.visuals().panel_fill;
    let text_color = analyzer_text_color(ui);
    let weak_color = panel_secondary_text_color(ui);
    let painter = ui.painter().with_clip_rect(rect);

    for p in placed.iter() {
        let node = &model.nodes[p.idx as usize];
        let base = category_color(node.category, dark);
        if p.is_dir {
            // Sharp corners: rounded fills leave 1px holes at tile junctions
            // that expose the panel fill as white specks.
            painter.rect_filled(p.rect, 0.0, mix(panel, base, 0.16));
            if p.header {
                let header = egui::Rect::from_min_size(
                    p.rect.min,
                    egui::vec2(p.rect.width(), treemap::HEADER_HEIGHT),
                );
                painter.rect_filled(header, 0.0, mix(panel, base, 0.34));
                draw_header_texts(
                    &painter,
                    header,
                    &node.name,
                    treemap::tile_value(&model, p.idx, state.metric, weights_ref),
                    state.metric,
                    text_color,
                );
            }
            painter.rect_stroke(
                p.rect.shrink(0.5),
                0.0,
                egui::Stroke::new(1.0, panel),
                egui::StrokeKind::Inside,
            );
        } else {
            painter.rect_filled(p.rect, 0.0, mix(panel, base, 0.72));
            // Same-category siblings would otherwise merge into one flat
            // block; the separator stroke keeps every tile visible.
            painter.rect_stroke(
                p.rect.shrink(0.5),
                0.0,
                egui::Stroke::new(1.0, panel),
                egui::StrokeKind::Inside,
            );
        }
    }

    // Hover + drill-down interaction.
    // Geometric hover check instead of `resp.hover_pos()`: when the clamped
    // tooltip covers the pointer (near window edges) its layer steals the
    // hover from this response, hiding the tooltip next frame and making it
    // flicker. The raw pointer position keeps the hover stable.
    let pointer = ui
        .input(|i| i.pointer.hover_pos())
        .filter(|p| rect.contains(*p));
    let hovered = pointer
        .and_then(|pos| treemap::hit_test(&placed, pos))
        .map(|p| p.idx);
    state.hovered = hovered;
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if let Some(h) = hovered {
        if let Some(p) = placed.iter().find(|p| p.idx == h) {
            painter.rect_stroke(
                p.rect.shrink(0.5),
                0.0,
                egui::Stroke::new(1.5, ui.visuals().strong_text_color()),
                egui::StrokeKind::Inside,
            );
        }
    }
    // Persistent selection highlight: black + yellow remain visible over every
    // category color, while the center marker locates even very small tiles.
    if let Some(sel) = state.selected {
        if let Some(p) = placed.iter().find(|p| p.idx == sel) {
            let selected_fill = egui::Color32::from_rgba_unmultiplied(255, 230, 0, 45);
            let selected_yellow = egui::Color32::from_rgb(255, 230, 0);
            painter.rect_filled(p.rect.shrink(0.5), 0.0, selected_fill);
            painter.rect_stroke(
                p.rect.shrink(0.5),
                0.0,
                egui::Stroke::new(5.0, egui::Color32::BLACK),
                egui::StrokeKind::Inside,
            );
            if p.rect.width() > 3.0 && p.rect.height() > 3.0 {
                painter.rect_stroke(
                    p.rect.shrink(1.5),
                    0.0,
                    egui::Stroke::new(2.0, selected_yellow),
                    egui::StrokeKind::Inside,
                );
            }
            let marker_radius = (p.rect.width().min(p.rect.height()) * 0.2).clamp(3.0, 6.0);
            painter.circle_filled(p.rect.center(), marker_radius, egui::Color32::BLACK);
            painter.circle_filled(p.rect.center(), marker_radius * 0.5, selected_yellow);
        }
    }
    if resp.clicked() {
        state.context_menu = None;
        if let Some(pos) = resp.interact_pointer_pos() {
            if let Some(p) = treemap::hit_test(&placed, pos) {
                // Reparse points are leaves: never drill into them.
                if p.is_dir && !model.nodes[p.idx as usize].is_reparse {
                    // Full ancestor chain so the breadcrumb shows the whole
                    // path even when clicking a nested frame directly.
                    state.drill_stack = model.chain_to(p.idx);
                    state.hovered = None;
                    state.selected = None;
                } else if !p.is_dir {
                    state.selected = Some(p.idx);
                }
            }
        }
    }

    // Right-click a tile: remember the target for the context menu below.
    if resp.secondary_clicked() {
        state.context_menu = resp
            .interact_pointer_pos()
            .and_then(|pos| treemap::hit_test(&placed, pos))
            .map(|p| p.idx);
    }

    // Tooltip for the hovered node (hidden while the context menu is open).
    if state.context_menu.is_none() {
        if let (Some(h), Some(pos)) = (hovered, pointer) {
            let node = &model.nodes[h as usize];
            let logical = if node.is_dir {
                node.subtree_size
            } else {
                node.size
            };
            let allocated = if node.is_dir {
                node.subtree_allocated_size
            } else {
                node.allocated_size
            };
            // Percentages follow the active weight basis so they always match
            // the rendered areas.
            let value = treemap::tile_value(&model, h, state.metric, weights_ref);
            let parent_value =
                treemap::tile_value(&model, node.parent, state.metric, weights_ref).max(1);
            let percent = (value as f64 / parent_value as f64) * 100.0;
            // Clamp so the tooltip stays inside the window and gets a usable width.
            let screen = ui.ctx().viewport_rect();
            let tooltip_pos = egui::pos2(
                (pos.x + 14.0)
                    .min(screen.max.x - 500.0)
                    .max(screen.min.x + 4.0),
                (pos.y + 12.0).min(screen.max.y - 120.0),
            );
            egui::Area::new(egui::Id::new("disk_analysis_tooltip"))
                .order(egui::Order::Tooltip)
                .interactable(false)
                .fixed_pos(tooltip_pos)
                .show(ui.ctx(), |ui| {
                    // Non-interactive tooltip: clicks pass through to the treemap.
                    ui.style_mut().interaction.selectable_labels = false;
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(480.0);
                        ui.add(
                            egui::Label::new(egui::RichText::new(&node.name).strong())
                                .wrap_mode(egui::TextWrapMode::Wrap),
                        );
                        ui.label(format!(
                            "{}: {} · {}: {} · {:.1}%",
                            t!("disk_analysis.logical_size"),
                            format_size(logical),
                            t!("disk_analysis.allocated_size"),
                            format_size(allocated),
                            percent
                        ));
                        if node.is_dir {
                            ui.label(
                                egui::RichText::new(t!("disk_analysis.drill_hint").to_string())
                                    .color(weak_color),
                            );
                        }
                    });
                });
        }
    }

    // Context menu for the right-clicked tile (open folder in the main app).
    // Opens at the pointer position; only when a tile was actually hit.
    let context_menu_id = egui::Id::new("disk_analysis_tile_context");
    let open_cmd = if resp.secondary_clicked() && state.context_menu.is_some() {
        Some(egui::SetOpenCommand::Bool(true))
    } else if resp.clicked() {
        Some(egui::SetOpenCommand::Bool(false))
    } else {
        None
    };
    let menu = egui::Popup::context_menu(&resp)
        .id(context_menu_id)
        .open_memory(open_cmd);
    if let Some(menu_resp) = menu.show(|ui| {
        ui.button(t!("disk_analysis.open_in_main_app").to_string())
            .clicked()
    }) {
        if menu_resp.inner {
            if let Some(menu_idx) = state.context_menu {
                let target = model.path_of(menu_idx);
                let is_dir = model.nodes[menu_idx as usize].is_dir;
                crate::disk_analyzer::open_in_main::open_path_in_main_app(&target, is_dir);
            }
            egui::Popup::close_id(ui.ctx(), context_menu_id);
        }
    }
    // Keep state in sync when egui closes the popup (outside click, etc.)
    // so the tooltip suppression below is lifted.
    if state.context_menu.is_some() && !egui::Popup::is_id_open(ui.ctx(), context_menu_id) {
        state.context_menu = None;
    }
}

/// Paint a treemap frame header without ever overlapping the name and the
/// size: the size wins, the name is truncated to the remaining space, and on
/// very narrow frames no text is drawn at all (tooltips cover that case).
fn draw_header_texts(
    painter: &egui::Painter,
    header: egui::Rect,
    name: &str,
    subtree_size: u64,
    metric: SizeMetric,
    text_color: egui::Color32,
) {
    const PAD: f32 = 6.0;
    const GAP: f32 = 8.0;
    let font_id = egui::FontId::proportional(12.0);
    let size_text = format_metric_value(metric, subtree_size);
    let size_galley = painter.layout_no_wrap(size_text.clone(), font_id.clone(), text_color);
    let size_w = size_galley.size().x;
    let avail = header.width() - PAD * 2.0;
    if avail <= 0.0 || size_w + 24.0 > avail {
        return; // too narrow: tooltips are enough
    }

    let name_avail = avail - size_w - GAP;
    if name_avail >= 24.0 {
        let name_galley = painter.layout_no_wrap(name.to_string(), font_id.clone(), text_color);
        let display = if name_galley.size().x <= name_avail {
            name.to_string()
        } else {
            let chars = name.chars().count().max(1);
            let per_char = name_galley.size().x / chars as f32;
            let max_chars = ((name_avail - 8.0) / per_char).floor() as usize;
            if max_chars >= 2 {
                let mut s: String = name.chars().take(max_chars.saturating_sub(1)).collect();
                s.push('…');
                s
            } else {
                String::new()
            }
        };
        if !display.is_empty() {
            painter.text(
                header.left_center() + egui::vec2(PAD, 0.0),
                egui::Align2::LEFT_CENTER,
                display,
                font_id.clone(),
                text_color,
            );
        }
    }
    painter.text(
        header.right_center() - egui::vec2(PAD, 0.0),
        egui::Align2::RIGHT_CENTER,
        size_text,
        font_id,
        text_color,
    );
}

pub(crate) fn format_metric_value(metric: SizeMetric, value: u64) -> String {
    match metric {
        SizeMetric::FileCount => value.to_string(),
        SizeMetric::Allocated | SizeMetric::Logical => format_size(value),
    }
}

fn render_center_state(state: &mut DiskAnalysisState, ui: &mut egui::Ui, phase: DiskAnalysisPhase) {
    ui.centered_and_justified(|ui| match phase {
        DiskAnalysisPhase::Failed => {
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new(t!("disk_analysis.failed").to_string()).strong());
                if let Some(error) = state.error.clone() {
                    ui.label(egui::RichText::new(error).color(panel_secondary_text_color(ui)));
                }
                ui.add_space(8.0);
                if ui.button(t!("disk_analysis.retry").to_string()).clicked() {
                    if let Some(letter) = state.drive_letter {
                        state.request(letter);
                    }
                }
            });
        }
        _ => {
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.label(t!("disk_analysis.loading").to_string());
            });
        }
    });
}
