//! Left sidebar of the disk analyzer: NTFS drives, volume summary, stats.

use crate::app::disk_analysis_model::FileCategory;
use crate::app::disk_analysis_state::DiskAnalysisState;
use crate::infrastructure::windows::formatting::format_size;
use crate::ui::disk_analysis::{category_color, category_label};
use eframe::egui;
use rust_i18n::t;

pub fn render_sidebar(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    ui.add_space(6.0);
    section_header(ui, &t!("disk_analysis.drives"));
    render_drives(state, ui);

    ui.add_space(10.0);
    ui.separator();
    ui.add_space(6.0);
    section_header(ui, &t!("disk_analysis.summary"));
    render_summary(state, ui);
}

/// Volume summary: file/folder counts and used-vs-total space (moved here
/// from the old status bar / top bar).
fn render_summary(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    if let Some(drive) = state
        .drives
        .iter()
        .find(|d| Some(d.letter) == state.drive_letter)
    {
        let used = drive.total_space.saturating_sub(drive.free_space);
        ui.label(
            egui::RichText::new(
                t!(
                    "disk_analysis.used_of",
                    used = format_size(used),
                    total = format_size(drive.total_space)
                )
                .to_string(),
            )
            .strong(),
        );
        ui.add_space(2.0);
    }
    if let Some(model) = state.model.clone() {
        ui.label(
            egui::RichText::new(
                t!("disk_analysis.files_count", count = model.total_files).to_string(),
            )
            .strong(),
        );
        ui.label(
            egui::RichText::new(
                t!("disk_analysis.folders_count", count = model.total_folders).to_string(),
            )
            .strong(),
        );
    }
    ui.add_space(20.0);
    render_usage_pie(state, ui);
}

/// Pie chart of category shares (same data as the status-bar legend).
fn render_usage_pie(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let Some(model) = state.model.clone() else {
        return;
    };
    let total = model.total_allocated_size.max(1);
    let dark = ui.visuals().dark_mode;

    let available = ui.available_width();
    let diameter = available.min(180.0);
    let response =
        ui.allocate_response(egui::vec2(available, diameter + 8.0), egui::Sense::hover());
    let rect = response.rect;
    // The scroll area's scrollbar gutter offsets the content rect, so the
    // raw center reads as shifted right; nudge left to optically center the
    // pie within the visible sidebar width.
    let center = egui::pos2(rect.center().x - 6.0, rect.center().y);
    let radius = diameter / 2.0;

    let pointer = response.hover_pos();
    let pointer_polar = pointer.map(|pos| {
        let v = pos - center;
        (v.length(), v.y.atan2(v.x))
    });

    // Sectors start at the top, clockwise.
    let mut start = -std::f32::consts::FRAC_PI_2;
    let mut hovered: Option<(FileCategory, u64, f64)> = None;
    for category in FileCategory::ALL {
        let bytes = model.category_totals[category.index()];
        if bytes == 0 {
            continue;
        }
        let sweep = (bytes as f32 / total as f32) * std::f32::consts::TAU;
        let end = start + sweep;

        if sweep >= 0.002 {
            ui.painter().add(egui::Shape::convex_polygon(
                sector_points(center, radius, start, sweep),
                category_color(category, dark),
                egui::Stroke::NONE,
            ));
        }
        ui.painter().line_segment(
            [
                center,
                center + radius * egui::Vec2::new(start.cos(), start.sin()),
            ],
            egui::Stroke::new(1.0, ui.visuals().panel_fill),
        );

        if let Some((r, angle)) = pointer_polar {
            if r <= radius {
                let mut a = angle;
                while a < start {
                    a += std::f32::consts::TAU;
                }
                if a < end {
                    hovered = Some((category, bytes, (bytes as f64 / total as f64) * 100.0));
                }
            }
        }
        start = end;
    }

    if let (Some((category, bytes, percent)), Some(pos)) = (hovered, pointer) {
        let screen = ui.ctx().viewport_rect();
        let tooltip_pos = egui::pos2(
            (pos.x + 14.0)
                .min(screen.max.x - 320.0)
                .max(screen.min.x + 4.0),
            (pos.y + 12.0).min(screen.max.y - 60.0),
        );
        egui::Area::new(egui::Id::new("disk_analysis_pie_tooltip"))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .fixed_pos(tooltip_pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(480.0);
                    ui.label(format!(
                        "{} · {} · {:.1}%",
                        category_label(category),
                        format_size(bytes),
                        percent
                    ));
                });
            });
    }
}

/// Sector polygon for the usage pie. Wide sectors use a center fan (convex,
/// miter-safe); thin slices use a 4-point trapezoid so no degenerate
/// micro-edges reach epaint's miter/feather math, which previously emitted
/// runaway spikes across the window.
fn sector_points(center: egui::Pos2, radius: f32, start: f32, sweep: f32) -> Vec<egui::Pos2> {
    if sweep >= 0.35 {
        let steps = ((sweep / 0.09).ceil() as usize).max(2);
        let mut points = Vec::with_capacity(steps + 2);
        points.push(center);
        for i in 0..=steps {
            let a = start + sweep * i as f32 / steps as f32;
            points.push(center + radius * egui::Vec2::new(a.cos(), a.sin()));
        }
        points
    } else {
        let end = start + sweep;
        let inner_radius = 1.5_f32.min(radius * 0.1);
        let (cs, ss) = (start.cos(), start.sin());
        let (ce, se) = (end.cos(), end.sin());
        vec![
            center + radius * egui::Vec2::new(cs, ss),
            center + radius * egui::Vec2::new(ce, se),
            center + inner_radius * egui::Vec2::new(ce, se),
            center + inner_radius * egui::Vec2::new(cs, ss),
        ]
    }
}

/// All sidebar text shares the same color and tone as the Summary
/// used-vs-total line: strong weight in the theme's default text color.
fn paint_sidebar_text(
    ui: &egui::Ui,
    pos: egui::Pos2,
    right_aligned: bool,
    text: String,
    size: f32,
) {
    let galley = egui::WidgetText::from(egui::RichText::new(text).size(size).strong()).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let mut pos = pos;
    if right_aligned {
        pos.x -= galley.size().x;
    }
    ui.painter().galley(pos, galley, ui.visuals().text_color());
}

fn section_header(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).size(11.0).strong());
    ui.add_space(4.0);
}

fn render_drives(state: &mut DiskAnalysisState, ui: &mut egui::Ui) {
    let active_letter = state.drive_letter;
    let drives = state.drives.clone();

    for drive in drives {
        if !drive.file_system.eq_ignore_ascii_case("NTFS") {
            continue;
        }
        let used = drive.total_space.saturating_sub(drive.free_space);
        let fraction = if drive.total_space > 0 {
            used as f32 / drive.total_space as f32
        } else {
            0.0
        };
        let is_active = active_letter == Some(drive.letter);

        let response =
            ui.allocate_response(egui::vec2(ui.available_width(), 52.0), egui::Sense::click());
        let rect = response.rect;
        // Border-only feedback, matching the main app's GRID mode:
        // solid accent outline when selected, light accent outline on hover.
        let accent = crate::ui::theme::COLOR_ACCENT;
        if is_active {
            let stroke_width = if response.hovered() { 2.5 } else { 2.0 };
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                6.0,
                egui::Stroke::new(stroke_width, accent),
                egui::StrokeKind::Inside,
            );
        } else if response.hovered() {
            ui.painter().rect_stroke(
                rect.shrink(1.0),
                6.0,
                egui::Stroke::new(1.0, accent.gamma_multiply(0.35)),
                egui::StrokeKind::Inside,
            );
        }

        let text_rect = rect.shrink2(egui::vec2(8.0, 6.0));
        paint_sidebar_text(
            ui,
            text_rect.left_top(),
            false,
            format!("{}:  {}", drive.letter, drive.label),
            13.0,
        );
        paint_sidebar_text(
            ui,
            text_rect.right_top(),
            true,
            format!("{:.0}%", fraction * 100.0),
            12.0,
        );
        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(text_rect.min.x, text_rect.min.y + 20.0),
            egui::vec2(text_rect.width(), 4.0),
        );
        ui.painter().rect_filled(
            bar_rect,
            2.0,
            crate::ui::theme::drive_usage_background_color(ui.visuals().dark_mode),
        );
        ui.painter().rect_filled(
            egui::Rect::from_min_size(
                bar_rect.min,
                egui::vec2(
                    bar_rect.width() * fraction.clamp(0.0, 1.0),
                    bar_rect.height(),
                ),
            ),
            2.0,
            crate::ui::theme::drive_usage_color(fraction),
        );
        paint_sidebar_text(
            ui,
            egui::pos2(text_rect.min.x, bar_rect.max.y + 3.0),
            false,
            t!(
                "disk_analysis.used_of",
                used = format_size(used),
                total = format_size(drive.total_space)
            )
            .to_string(),
            11.0,
        );

        if response.clicked() && !is_active {
            state.request(drive.letter);
            ui.ctx().request_repaint();
        }
        ui.add_space(2.0);
    }
}

#[cfg(test)]
mod pie_tessellation_tests {
    use super::sector_points;
    use eframe::egui;

    /// The sector polygons must never tessellate into vertices outside the
    /// pie disc (the old degenerate construction emitted feather spikes
    /// thousands of points long).
    #[test]
    fn sector_tessellation_stays_within_disc() {
        let center = egui::pos2(500.0, 500.0);
        let radius = 90.0;
        let mut tessellator = egui::epaint::Tessellator::new(
            1.0,
            egui::epaint::TessellationOptions::default(),
            [1024, 1024],
            vec![],
        );
        for sweep in [0.005_f32, 0.02, 0.3, 0.36, 1.0, 4.6] {
            let points = sector_points(center, radius, -std::f32::consts::FRAC_PI_2, sweep);
            let mut mesh = egui::epaint::Mesh::default();
            tessellator.tessellate_shape(
                egui::Shape::convex_polygon(points, egui::Color32::RED, egui::Stroke::NONE),
                &mut mesh,
            );
            let max_dist = mesh
                .vertices
                .iter()
                .map(|v| (v.pos - center).length())
                .fold(0.0_f32, f32::max);
            assert!(
                max_dist <= radius + 2.0,
                "sweep {sweep}: tessellated vertex escaped the disc (max_dist={max_dist})"
            );
        }
    }
}
