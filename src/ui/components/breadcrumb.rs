use crate::ui::preview_panel::utils::truncate_text_to_fit;
use crate::ui::theme;
use eframe::egui;
use std::cell::RefCell;
use std::path::Component;

// M-3: Cache breadcrumb segments — recomputed only when path changes.
// Each entry is (display_label, navigation_target).
thread_local! {
    static BREADCRUMB_CACHE: RefCell<(String, Vec<(String, String)>)> =
        const { RefCell::new((String::new(), Vec::new())) };
}

/// Returns the pre-split breadcrumb segments for the given path string.
/// On a cache hit (same path + locale) the cached Vec is cloned; on a miss
/// segments are recomputed from `Path::components()` and the cache is updated.
pub fn breadcrumb_segments(current_path: &str) -> Vec<(String, String)> {
    BREADCRUMB_CACHE.with(|cache| {
        let mut c = cache.borrow_mut();
        // Include locale in cache key so breadcrumbs refresh on language change
        let current_locale = rust_i18n::locale().to_string();
        let cache_key = format!("{}|{}", current_path, current_locale);
        if c.0 == cache_key {
            return c.1.clone();
        }
        // cache miss — recompute
        let mut full = std::path::PathBuf::new();
        let path = std::path::Path::new(current_path);
        let components: Vec<_> = path.components().collect();
        let mut segs = Vec::with_capacity(components.len());
        for (i, comp) in components.iter().enumerate() {
            full.push(comp.as_os_str());

            if matches!(comp, Component::RootDir) {
                continue;
            }

            let comp_str = comp.as_os_str().to_string_lossy();
            let display_name = comp_str.trim_end_matches('\\');
            if display_name.is_empty() && i > 0 {
                continue;
            }
            let target = {
                let mut p = full.to_string_lossy().into_owned();
                if p.len() == 2 && p.ends_with(':') {
                    p.push('\\');
                }
                p
            };
            // Use translated name for known special folders (Desktop, Documents, etc.)
            let display = crate::infrastructure::onedrive::special_folder_display_name(&full)
                .unwrap_or_else(|| {
                    if display_name.is_empty() {
                        comp_str.to_string()
                    } else {
                        display_name.to_string()
                    }
                });
            segs.push((display, target));
        }
        c.0 = cache_key;
        c.1 = segs.clone();
        segs
    })
}

/// Builds breadcrumb segments from a `std::path::Path` directly, without
/// touching the thread-local cache. Used by the details panel where the
/// displayed path changes on every selection and caching would not help.
pub fn breadcrumb_segments_for_path(path: &std::path::Path) -> Vec<(String, String)> {
    let mut full = std::path::PathBuf::new();
    let components: Vec<_> = path.components().collect();
    let mut segs = Vec::with_capacity(components.len());
    for (i, comp) in components.iter().enumerate() {
        full.push(comp.as_os_str());

        if matches!(comp, Component::RootDir) {
            continue;
        }

        let comp_str = comp.as_os_str().to_string_lossy();
        let display_name = comp_str.trim_end_matches('\\');
        if display_name.is_empty() && i > 0 {
            continue;
        }
        let target = {
            let mut p = full.to_string_lossy().into_owned();
            if p.len() == 2 && p.ends_with(':') {
                p.push('\\');
            }
            p
        };
        let display = crate::infrastructure::onedrive::special_folder_display_name(&full)
            .unwrap_or_else(|| {
                if display_name.is_empty() {
                    comp_str.to_string()
                } else {
                    display_name.to_string()
                }
            });
        segs.push((display, target));
    }
    segs
}

/// Result of fitting a breadcrumb trail into the available width. Mirrors
/// the layout decision the address bar makes so both surfaces stay in sync.
struct TrailLayout {
    /// Indices of segments to render in the main horizontal trail (in order).
    visible: Vec<usize>,
    /// Total width consumed by `visible`, including separators and the
    /// ellipsis button when present.
    width: f32,
    /// When non-empty, the ellipsis button is rendered between `visible[0]`
    /// and `visible[1]` and the indices in this slice (in path order) are
    /// listed in the overflow popup.
    hidden: Vec<usize>,
    /// True if even the `root + last` baseline doesn't fit — the visible
    /// list is then just the tail of the path.
    fallback_tail: bool,
}

fn measure_text_width(ui: &egui::Ui, text: &str, font_id: &egui::FontId) -> f32 {
    ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(text.to_string(), font_id.clone(), ui.visuals().text_color())
            .size()
            .x
    })
}

/// Computes which segments fit, which go into the overflow popup, and what
/// width the resulting trail will occupy. This is the same algorithm the
/// address bar uses (root + (ellipsis) + tail that fit, falling back to a
/// pure tail when even the root+last pair is too wide).
///
/// The returned `width` is the **rendered** width — sum of item widths plus
/// the `item_spacing` gap that `ui.horizontal_wrapped` inserts between every
/// pair of consecutive items. Without this, the trail ends up a few pixels
/// wider than the budget and wraps to a second line.
fn compute_trail_layout(
    ui: &egui::Ui,
    segments: &[(String, String)],
) -> TrailLayout {
    let total = segments.len();
    if total == 0 {
        return TrailLayout {
            visible: Vec::new(),
            width: 0.0,
            hidden: Vec::new(),
            fallback_tail: false,
        };
    }

    let font_id = egui::FontId::proportional(11.0);
    let button_padding = ui.spacing().button_padding.x * 2.0;
    let chevron_width = measure_text_width(ui, "›", &font_id);
    let ellipsis_width = measure_text_width(ui, "…", &font_id) + button_padding;
    // Must match `ui.spacing_mut().item_spacing.x = 2.0` set in the renderer.
    let item_spacing = 2.0;
    let available_width = ui.available_width();

    let seg_widths: Vec<f32> = segments
        .iter()
        .map(|(display, _)| measure_text_width(ui, display, &font_id) + button_padding)
        .collect();

    // Total rendered width when nothing is hidden: N segments + (N-1) chevrons
    // + (N-1) item_spacing gaps between them.
    let total_width: f32 = seg_widths.iter().sum::<f32>()
        + chevron_width * total.saturating_sub(1) as f32
        + item_spacing * total.saturating_sub(1) as f32;

    if total_width <= available_width {
        return TrailLayout {
            visible: (0..total).collect(),
            width: total_width,
            hidden: Vec::new(),
            fallback_tail: false,
        };
    }

    // Truncated layout: root + (ellipsis) + tail that fits.
    let root_width = seg_widths[0];
    let last_width = seg_widths[total - 1];

    // Initial: [root] › [last]  → 2 item_spacing gaps.
    let mut needed = root_width + item_spacing + chevron_width + item_spacing + last_width;
    let mut show_ellipsis = false;

    if total > 2 {
        // With ellipsis: [root] › [⋯] › [last]  → 4 item_spacing gaps.
        // The two extra gaps bracket the inserted ellipsis+chevron pair.
        let with_ellipsis =
            needed + item_spacing + ellipsis_width + item_spacing + chevron_width;
        if with_ellipsis <= available_width {
            needed = with_ellipsis;
            show_ellipsis = true;
        }
    }

    let mut visible_end_count = 1;
    if needed <= available_width && total > 2 {
        for i in (1..total - 1).rev() {
            // Each appended tail segment brings one chevron and two more
            // item_spacing gaps (one on each side of the chevron).
            let extra = chevron_width + item_spacing + seg_widths[i] + item_spacing;
            if needed + extra <= available_width {
                needed += extra;
                visible_end_count += 1;
            } else {
                break;
            }
        }
    }

    if needed > available_width && total > 1 {
        // Even root + last doesn't fit — show as many tail segments as
        // possible without root or ellipsis.
        let mut used = 0.0;
        let mut vis = 0;
        for i in (0..total).rev() {
            let extra = if vis == 0 {
                seg_widths[i]
            } else {
                chevron_width + item_spacing + seg_widths[i] + item_spacing
            };
            if used + extra <= available_width {
                used += extra;
                vis += 1;
            } else {
                break;
            }
        }
        let start = total - vis;
        return TrailLayout {
            visible: (start..total).collect(),
            width: used,
            hidden: (0..start).collect(),
            fallback_tail: true,
        };
    }

    let last_visible_start = total - visible_end_count;
    // Visible trail is always: root, then either the ellipsis+tail or just
    // the contiguous tail starting at `last_visible_start`.
    let visible: Vec<usize> = std::iter::once(0)
        .chain(last_visible_start..total)
        .collect();
    let hidden: Vec<usize> = if show_ellipsis && last_visible_start > 1 {
        (1..last_visible_start).collect()
    } else {
        Vec::new()
    };

    TrailLayout {
        visible,
        width: needed,
        hidden,
        fallback_tail: false,
    }
}

/// Renders a clickable breadcrumb trail. Every visible segment — including
/// the last one — is rendered as a button. The last segment is styled in
/// bold to mark the current location, but remains clickable so callers can
/// navigate to that folder (useful for files shown inside a Tag view).
///
/// When the trail doesn't fit, the same root + "…" + tail strategy the
/// address bar uses is applied, and the ellipsis button opens an overflow
/// popup listing the hidden segments. Returns the `target_path` of the
/// clicked segment, or `None`.
pub fn render_breadcrumb_trail(
    ui: &mut egui::Ui,
    segments: &[(String, String)],
) -> Option<String> {
    if segments.is_empty() {
        return None;
    }

    let layout = compute_trail_layout(ui, segments);
    let mut action: Option<String> = None;
    let total = segments.len();
    let text_color = theme::text_color(ui.visuals().dark_mode);
    let chevron_color = ui.visuals().weak_text_color();

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;

        if layout.fallback_tail {
            render_visible_tail(ui, segments, &layout, text_color, chevron_color, &mut action);
            return;
        }

        // Root segment is always first in `visible` when not falling back.
        let root_idx = layout.visible[0];
        let root_label = egui::RichText::new(&segments[root_idx].0)
            .color(text_color)
            .size(11.0);
        if breadcrumb_button_with_label(ui, root_label) {
            action = Some(segments[root_idx].1.clone());
        }

        let has_hidden_popup = !layout.hidden.is_empty() && layout.visible.len() >= 2;
        if has_hidden_popup {
            ui.label(egui::RichText::new("›").color(chevron_color).size(11.0));
            if let Some(target) =
                render_overflow_popup(ui, segments, &layout.hidden, text_color)
            {
                action = Some(target);
            }
            ui.label(egui::RichText::new("›").color(chevron_color).size(11.0));
        }

        // Render the remaining visible segments (the tail). A chevron goes
        // before each one except the first one when a popup sits between
        // root and tail (the popup block already emits its own closing
        // chevron).
        for (pos, &seg_idx) in layout.visible.iter().enumerate().skip(1) {
            let skip_chevron = pos == 1 && has_hidden_popup;
            if !skip_chevron {
                ui.label(egui::RichText::new("›").color(chevron_color).size(11.0));
            }
            let is_last_overall = seg_idx + 1 == total;
            let label = if is_last_overall {
                egui::RichText::new(&segments[seg_idx].0)
                    .strong()
                    .color(text_color)
                    .size(11.0)
            } else {
                egui::RichText::new(&segments[seg_idx].0)
                    .color(text_color)
                    .size(11.0)
            };
            if breadcrumb_button_with_label(ui, label) {
                action = Some(segments[seg_idx].1.clone());
            }
        }
    });

    action
}

fn render_visible_tail(
    ui: &mut egui::Ui,
    segments: &[(String, String)],
    layout: &TrailLayout,
    text_color: egui::Color32,
    chevron_color: egui::Color32,
    action: &mut Option<String>,
) {
    let total = segments.len();
    for (pos, &seg_idx) in layout.visible.iter().enumerate() {
        if pos > 0 {
            ui.label(egui::RichText::new("›").color(chevron_color).size(11.0));
        }
        let is_last_overall = seg_idx + 1 == total;
        let label = if is_last_overall {
            egui::RichText::new(&segments[seg_idx].0)
                .strong()
                .color(text_color)
                .size(11.0)
        } else {
            egui::RichText::new(&segments[seg_idx].0)
                .color(text_color)
                .size(11.0)
        };
        if breadcrumb_button_with_label(ui, label) {
            *action = Some(segments[seg_idx].1.clone());
        }
    }
}

/// Renders the "…" overflow button and its popup. The popup lists the
/// hidden segments in path order. Returns the `target_path` of the segment
/// the user clicked in the popup, or `None`.
fn render_overflow_popup(
    ui: &mut egui::Ui,
    segments: &[(String, String)],
    hidden_indices: &[usize],
    text_color: egui::Color32,
) -> Option<String> {
    let popup_id = egui::Id::new("details_breadcrumb_overflow_popup");
    let mut show_overflow = ui
        .ctx()
        .memory(|m| m.data.get_temp::<bool>(popup_id).unwrap_or(false));

    let font_id = egui::FontId::proportional(11.0);
    let ellipsis_resp = ui
        .scope(|ui| {
            let hover_color = if ui.visuals().dark_mode {
                theme::color_dark_hover()
            } else {
                theme::color_hover()
            };

            ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
            ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;

            ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
            ui.visuals_mut().widgets.hovered.weak_bg_fill = hover_color;
            ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;

            ui.visuals_mut().widgets.active.bg_fill = if ui.visuals().dark_mode {
                egui::Color32::from_gray(70)
            } else {
                egui::Color32::from_gray(210)
            };
            ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;

            ui.button(egui::RichText::new("…").color(text_color))
        })
        .inner;

    if ellipsis_resp.clicked() {
        show_overflow = !show_overflow;
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp(popup_id, show_overflow));
    }

    if !show_overflow {
        return None;
    }

    let mut selected_path: Option<String> = None;
    let popup_response = egui::Area::new(popup_id)
        .order(egui::Order::Foreground)
        .fixed_pos(egui::pos2(
            ellipsis_resp.rect.left(),
            ellipsis_resp.rect.bottom() + 2.0,
        ))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(180.0);
                ui.spacing_mut().item_spacing.y = 0.0;

                for &idx in hidden_indices {
                    let (display, actual_path) = segments[idx].clone();
                    let item_size = egui::vec2(400.0, 28.0);
                    let (item_rect, response) =
                        ui.allocate_exact_size(item_size, egui::Sense::click());
                    let visuals = ui.style().interact(&response);

                    if response.hovered() || response.highlighted() {
                        ui.painter().rect_filled(
                            item_rect,
                            visuals.corner_radius,
                            visuals.weak_bg_fill,
                        );
                    }

                    let text_rect = item_rect.shrink2(egui::vec2(8.0, 0.0));
                    let truncated = truncate_text_to_fit(
                        &display,
                        text_rect.width(),
                        &font_id,
                        ui,
                    );
                    ui.painter().text(
                        egui::pos2(text_rect.left(), item_rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        &truncated,
                        font_id.clone(),
                        text_color,
                    );
                    if truncated != display {
                        let _ = response.clone().on_hover_text(display);
                    }

                    if response.clicked() {
                        selected_path = Some(actual_path);
                    }
                }
            });
        });

    if let Some(path) = selected_path {
        ui.ctx()
            .memory_mut(|m| m.data.insert_temp::<bool>(popup_id, false));
        return Some(path);
    }

    // Auto-close when the user clicks anywhere outside the button and popup.
    if ui.ctx().input(|i| i.pointer.any_pressed()) {
        if let Some(pointer_pos) = ui.ctx().input(|i| i.pointer.press_origin()) {
            let clicked_ellipsis = ellipsis_resp.rect.contains(pointer_pos);
            let clicked_popup = popup_response.response.rect.contains(pointer_pos);
            if !clicked_ellipsis && !clicked_popup {
                ui.ctx()
                    .memory_mut(|m| m.data.insert_temp::<bool>(popup_id, false));
            }
        }
    }

    None
}

/// Estimates the natural (shrink-fit) width of the breadcrumb trail in
/// pixels, matching what `render_breadcrumb_trail` would consume. Uses the
/// same layout algorithm as the renderer so the centering math
/// (`(available_width - natural_width) / 2`) stays accurate when the trail
/// is shrunk to fit a narrow panel.
pub fn measure_breadcrumb_trail(ui: &egui::Ui, segments: &[(String, String)]) -> f32 {
    if segments.is_empty() {
        return 0.0;
    }
    compute_trail_layout(ui, segments).width
}

/// Renders a single clickable breadcrumb segment using the project's
/// breadcrumb button visuals (transparent background, hover effect). The
/// caller passes a pre-built `RichText` so it can apply extra styling such
/// as `strong()` for the "current" segment. Returns `true` if the button was
/// clicked this frame.
fn breadcrumb_button_with_label(ui: &mut egui::Ui, label: egui::RichText) -> bool {
    let hover_color = if ui.visuals().dark_mode {
        theme::color_dark_hover()
    } else {
        theme::color_hover()
    };

    ui.visuals_mut().widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
    ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
    ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;

    ui.visuals_mut().widgets.hovered.bg_fill = hover_color;
    ui.visuals_mut().widgets.hovered.weak_bg_fill = hover_color;
    ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;

    ui.visuals_mut().widgets.active.bg_fill = if ui.visuals().dark_mode {
        egui::Color32::from_gray(70)
    } else {
        egui::Color32::from_gray(210)
    };
    ui.visuals_mut().widgets.active.bg_stroke = egui::Stroke::NONE;

    ui.button(label).clicked()
}
