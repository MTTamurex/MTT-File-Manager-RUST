//! Disk usage analyzer full view (NTFS volumes, backed by the MTT Search
//! Service USN/MFT index). Layout inspired by modern disk analyzers: header
//! with drive summary and scan action, left sidebar with drives/file types,
//! squarified treemap with drill-down breadcrumbs, and a stats footer.

mod sidebar;
mod treemap;

use crate::app::disk_analysis_model::FileCategory;
use crate::app::disk_analysis_state::DiskAnalysisPhase;
use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows::formatting::format_size;
use eframe::egui;
use rust_i18n::t;

/// Muted category palette (dark/light agnostic base colors).
pub fn category_color(category: FileCategory, dark: bool) -> egui::Color32 {
    let (r, g, b) = match category {
        FileCategory::Video => (99, 102, 214),
        FileCategory::Images => (200, 88, 138),
        FileCategory::Audio => (150, 95, 195),
        FileCategory::Archives => (212, 149, 70),
        FileCategory::Code => (56, 152, 140),
        FileCategory::Documents => (120, 168, 80),
        FileCategory::System => (130, 136, 150),
        FileCategory::Other => (158, 158, 166),
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

/// Render the analyzer into its own OS window (same-process immediate
/// viewport). Must be called every frame while `disk_analysis.active`.
pub fn render_disk_analysis_viewport(app: &mut ImageViewerApp, ctx: &egui::Context) {
    if !app.disk_analysis.active {
        return;
    }
    let close_requested = ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("mtt_disk_analyzer"),
        egui::ViewportBuilder::default()
            .with_title(t!("disk_analysis.title").to_string())
            .with_inner_size([1150.0, 720.0])
            .with_min_inner_size([760.0, 480.0]),
        |ui: &mut egui::Ui, _class| {
            let close = ui.input(|i| i.viewport().close_requested());
            render_view_body(app, ui);
            close
        },
    );
    if close_requested {
        app.close_disk_analysis();
    }
}

fn render_view_body(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    if app.disk_analysis.poll() {
        ctx.request_repaint();
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.close_disk_analysis();
        return;
    }

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill))
        .show(ui, |ui| {
            render_header(app, ui);
            render_footer(app, ui);

            egui::Panel::left(egui::Id::new("disk_analysis_sidebar"))
                .resizable(false)
                .exact_size(250.0)
                .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(10.0))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("disk_analysis_sidebar_scroll")
                        .show(ui, |ui| sidebar::render_sidebar(app, ui));
                });

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(8.0))
                .show(ui, |ui| {
                    render_breadcrumb(app, ui);
                    ui.add_space(4.0);
                    render_treemap(app, ui);
                });
        });
}

fn drive_label(app: &ImageViewerApp, letter: char) -> String {
    app.drive_state
        .disks
        .iter()
        .find(|(path, _)| path.chars().next().map(|c| c.to_ascii_uppercase()) == Some(letter))
        .map(|(_, label)| label.clone())
        .unwrap_or_default()
}

fn render_header(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    egui::Panel::top(egui::Id::new("disk_analysis_header"))
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(egui::Margin::same(12)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let letter = app.disk_analysis.drive_letter;
                if let Some(letter) = letter {
                    ui.label(egui::RichText::new(format!("{letter}:")).strong().size(15.0));
                    let label = drive_label(app, letter);
                    if !label.is_empty() {
                        ui.label(label);
                    }
                    ui.label(
                        egui::RichText::new("— NTFS").color(ui.visuals().weak_text_color()),
                    );
                } else {
                    ui.label(egui::RichText::new(t!("disk_analysis.title").to_string()).strong().size(15.0));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let fetching = app.disk_analysis.phase == DiskAnalysisPhase::Fetching;
                    let scan_label = if fetching {
                        t!("disk_analysis.scanning").to_string()
                    } else {
                        t!("disk_analysis.scan").to_string()
                    };
                    let scan_enabled = !fetching && letter.is_some();
                    if ui.add_enabled(scan_enabled, egui::Button::new(scan_label)).clicked() {
                        if let Some(letter) = letter {
                            app.disk_analysis.request(letter);
                        }
                    }
                    ui.add_space(10.0);

                    // Index state + usage summary.
                    match app.disk_analysis.index_state.as_deref() {
                        Some("ready") => {
                            ui.label(egui::RichText::new(t!("disk_analysis.index_ready")).color(ui.visuals().weak_text_color()));
                        }
                        Some("scanning") | Some("not_started") => {
                            ui.label(egui::RichText::new(t!("disk_analysis.index_scanning")).color(ui.visuals().weak_text_color()));
                        }
                        _ => {}
                    }
                    if let Some(letter) = letter {
                        let info = app
                            .drive_state
                            .cached_drive_info(&format!("{letter}:\\"));
                        if let Some(info) = info {
                            let used = info.total_space.saturating_sub(info.free_space);
                            ui.label(
                                egui::RichText::new(
                                    t!(
                                        "disk_analysis.used_of",
                                        used = format_size(used),
                                        total = format_size(info.total_space)
                                    )
                                    .to_string(),
                                )
                                .strong(),
                            );
                        }
                    }
                });
            });
            ui.separator();
        });
}

fn render_footer(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    egui::Panel::bottom(egui::Id::new("disk_analysis_footer"))
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(ui.visuals().panel_fill).inner_margin(egui::Margin::symmetric(12, 6)))
        .show(ui, |ui| {
            ui.separator();
            ui.horizontal(|ui| {
                if let Some(model) = app.disk_analysis.model.clone() {
                    ui.label(
                        egui::RichText::new(
                            t!("disk_analysis.files_count", count = model.total_files).to_string(),
                        )
                        .color(ui.visuals().weak_text_color()),
                    );
                    ui.add_space(16.0);
                    ui.label(
                        egui::RichText::new(
                            t!("disk_analysis.folders_count", count = model.total_folders).to_string(),
                        )
                        .color(ui.visuals().weak_text_color()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(elapsed) = app.disk_analysis.fetch_elapsed {
                            ui.label(
                                egui::RichText::new(
                                    t!("disk_analysis.scan_time", secs = format!("{:.2}", elapsed.as_secs_f64())).to_string(),
                                )
                                .color(ui.visuals().weak_text_color()),
                            );
                        }
                    });
                }
            });
        });
}

fn render_breadcrumb(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    let Some(model) = app.disk_analysis.model.clone() else {
        return;
    };
    let stack = app.disk_analysis.drill_stack.clone();
    let mut truncate_to: Option<usize> = None;
    ui.horizontal(|ui| {
        for (i, idx) in stack.iter().enumerate() {
            let node = &model.nodes[*idx as usize];
            let label = node.name.clone();
            let is_last = i + 1 == stack.len();
            let text = if is_last {
                egui::RichText::new(label).strong()
            } else {
                egui::RichText::new(label).color(ui.visuals().weak_text_color())
            };
            if ui.add(egui::Button::new(text).frame(false)).clicked() && !is_last {
                truncate_to = Some(i + 1);
            }
            if !is_last {
                ui.label(egui::RichText::new("›").color(ui.visuals().weak_text_color()));
            }
        }
    });
    if let Some(len) = truncate_to {
        app.disk_analysis.drill_stack.truncate(len);
        app.disk_analysis.hovered = None;
        ui.ctx().request_repaint();
    }
}

fn render_treemap(app: &mut ImageViewerApp, ui: &mut egui::Ui) {
    let phase = app.disk_analysis.phase;
    let model = app.disk_analysis.model.clone();

    let Some(model) = model else {
        render_center_state(app, ui, phase);
        return;
    };

    if phase == DiskAnalysisPhase::Fetching {
        // Subtle refresh indicator while a new snapshot is in flight.
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.spinner();
            ui.label(
                egui::RichText::new(t!("disk_analysis.loading").to_string())
                    .color(ui.visuals().weak_text_color()),
            );
        });
    }

    let current = *app.disk_analysis.drill_stack.last().unwrap_or(&model.root);
    let avail = ui.available_rect_before_wrap();
    if avail.width() < 16.0 || avail.height() < 16.0 {
        return;
    }
    let (rect, resp) = ui.allocate_exact_size(avail.size(), egui::Sense::click());
    let area = rect.shrink(4.0);
    let placed = treemap::layout(&model, current, area);

    let dark = ui.visuals().dark_mode;
    let panel = ui.visuals().panel_fill;
    let text_color = ui.visuals().text_color();
    let weak_color = ui.visuals().weak_text_color();
    let painter = ui.painter().with_clip_rect(rect);

    for p in &placed {
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
                draw_header_texts(&painter, header, &node.name, node.subtree_size, text_color);
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
    let pointer = resp.hover_pos();
    let hovered = pointer
        .and_then(|pos| treemap::hit_test(&placed, pos))
        .map(|p| p.idx);
    app.disk_analysis.hovered = hovered;
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
    if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            if let Some(p) = treemap::hit_test(&placed, pos) {
                // Reparse points are leaves: never drill into them.
                if p.is_dir && !model.nodes[p.idx as usize].is_reparse {
                    app.disk_analysis.drill_stack.push(p.idx);
                    app.disk_analysis.hovered = None;
                }
            }
        }
    }

    // Tooltip for the hovered node.
    if let (Some(h), Some(pos)) = (hovered, pointer) {
        let node = &model.nodes[h as usize];
        let own = if node.is_dir { node.subtree_size } else { node.size };
        let parent_size = model.nodes[node.parent as usize].subtree_size.max(1);
        let percent = (own as f64 / parent_size as f64) * 100.0;
        // Clamp so the tooltip stays inside the window and gets a usable width.
        let screen = ui.ctx().viewport_rect();
        let tooltip_pos = egui::pos2(
            (pos.x + 14.0).min(screen.max.x - 500.0).max(screen.min.x + 4.0),
            (pos.y + 12.0).min(screen.max.y - 120.0),
        );
        egui::Area::new(egui::Id::new("disk_analysis_tooltip"))
            .order(egui::Order::Tooltip)
            .fixed_pos(tooltip_pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(480.0);
                    ui.add(
                        egui::Label::new(egui::RichText::new(model.path_of(h)).strong())
                            .wrap_mode(egui::TextWrapMode::Truncate),
                    );
                    ui.label(format!("{} · {:.1}%", format_size(own), percent));
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

/// Paint a treemap frame header without ever overlapping the name and the
/// size: the size wins, the name is truncated to the remaining space, and on
/// very narrow frames no text is drawn at all (tooltips cover that case).
fn draw_header_texts(
    painter: &egui::Painter,
    header: egui::Rect,
    name: &str,
    subtree_size: u64,
    text_color: egui::Color32,
) {
    const PAD: f32 = 6.0;
    const GAP: f32 = 8.0;
    let font_id = egui::FontId::proportional(12.0);
    let size_text = format_size(subtree_size);
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

fn render_center_state(app: &mut ImageViewerApp, ui: &mut egui::Ui, phase: DiskAnalysisPhase) {
    ui.centered_and_justified(|ui| {
        match phase {
            DiskAnalysisPhase::Failed => {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(t!("disk_analysis.failed").to_string()).strong(),
                    );
                    if let Some(error) = app.disk_analysis.error.clone() {
                        ui.label(
                            egui::RichText::new(error).color(ui.visuals().weak_text_color()),
                        );
                    }
                    ui.add_space(8.0);
                    if ui.button(t!("disk_analysis.retry").to_string()).clicked() {
                        if let Some(letter) = app.disk_analysis.drive_letter {
                            app.disk_analysis.request(letter);
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
        }
    });
}
