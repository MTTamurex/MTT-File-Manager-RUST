use eframe::egui;

pub(super) fn centered_modal_area(viewport: egui::Rect) -> egui::Area {
    // Anchor using the measured size: translated filters can exceed the preferred width.
    egui::Area::new(egui::Id::new("global_search_modal"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .constrain_to(viewport)
        .order(egui::Order::Foreground)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_stays_centered_when_content_exceeds_preferred_width() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
        for width in [640.0, 950.0] {
            let mut actual = egui::Rect::NOTHING;
            // Area sizing is measured on the first pass and positioned on the next.
            for _ in 0..3 {
                let _ = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(viewport),
                        ..Default::default()
                    },
                    |ui| {
                        actual = centered_modal_area(viewport)
                            .show(ui.ctx(), |ui| {
                                ui.allocate_exact_size(
                                    egui::vec2(width, 600.0),
                                    egui::Sense::hover(),
                                );
                            })
                            .response
                            .rect;
                    },
                );
            }
            assert!((actual.width() - width).abs() < 1.0);
            assert!((actual.center().x - viewport.center().x).abs() < 1.0);
            assert!((actual.center().y - viewport.center().y).abs() < 1.0);
        }
    }
}
