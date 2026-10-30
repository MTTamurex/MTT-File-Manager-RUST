use super::*;

#[test]
fn blocked_deletion_is_visible_and_can_be_dismissed_without_starting_an_operation() {
    let ctx = egui::Context::default();
    let mut state = DiskAnalysisState::new();
    state.duplicates.deletion.status = Some(DeleteStatus::Unsafe);
    let mut button = None;
    for _ in 0..3 {
        let output = ctx.run_ui(egui::RawInput::default(), |ui| render(&mut state, ui.ctx()));
        let mut warning_visible = false;
        for shape in output.shapes {
            if let egui::Shape::Text(text) = shape.shape {
                if text.galley.text() == t!("disk_analysis.dup_delete_unsafe").as_ref() {
                    warning_visible = true;
                }
                if text.galley.text() == t!("disk_analysis.dup_delete_close").as_ref() {
                    button = Some(text.pos + text.galley.size() * 0.5);
                }
            }
        }
        if button.is_some() {
            assert!(warning_visible);
        }
    }
    let pos = button.expect("close button visible");
    for pressed in [true, false] {
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| render(&mut state, ui.ctx()),
        );
    }
    assert!(!state.duplicates.deletion.is_open());
    assert!(!state.duplicates.deletion.is_running());
}

#[test]
fn no_deletion_dialog_without_an_explicit_request_or_result() {
    let ctx = egui::Context::default();
    let mut state = DiskAnalysisState::new();
    let output = ctx.run_ui(egui::RawInput::default(), |ui| render(&mut state, ui.ctx()));
    assert!(!output
        .shapes
        .iter()
        .any(|s| matches!(&s.shape, egui::Shape::Text(_))));
}

fn progress_text(progress: DeleteProgress, running: bool, status: DeleteStatus) -> Vec<String> {
    let ctx = egui::Context::default();
    let mut text = Vec::new();
    for _ in 0..3 {
        text.clear();
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            render_progress(ui, progress, running, Some(status));
        });
        for shape in output.shapes {
            if let egui::Shape::Text(shape) = shape.shape {
                text.push(shape.galley.text().to_owned());
            }
        }
    }
    text
}

#[test]
fn full_read_shows_steps_percentage_bytes_and_pending_windows() {
    let progress = DeleteProgress {
        stage: DeleteStage::TargetContent,
        bytes_read: 2048,
        total_bytes: 4096,
        failure: None,
    };
    let text = progress_text(progress, true, DeleteStatus::Verifying);
    assert!(text.iter().any(|s| s.contains("50%")));
    assert!(text.iter().any(|s| s
        == &t!(
            "disk_analysis.dup_delete_read_bytes",
            read = "2048",
            total = "4096"
        )
        .to_string()));
    for (stage, key) in [
        (DeleteStage::Pair, "disk_analysis.dup_delete_step_done"),
        (
            DeleteStage::TargetContent,
            "disk_analysis.dup_delete_step_running",
        ),
        (
            DeleteStage::Windows,
            "disk_analysis.dup_delete_step_waiting",
        ),
    ] {
        assert!(text.contains(&format!("{} — {}", stage_label(stage), t!(key))));
    }
}

#[test]
fn windows_has_no_invented_percentage_and_earlier_checks_are_complete() {
    let progress = DeleteProgress {
        stage: DeleteStage::Windows,
        ..Default::default()
    };
    let text = progress_text(progress, true, DeleteStatus::Verifying);
    assert!(!text.iter().any(|s| s.contains('%')));
    assert!(text.contains(&format!(
        "{} — {}",
        stage_label(DeleteStage::Revalidate),
        t!("disk_analysis.dup_delete_step_done")
    )));
    assert!(text.contains(&format!(
        "{} — {}",
        stage_label(DeleteStage::Windows),
        t!("disk_analysis.dup_delete_step_running")
    )));
}

#[test]
fn failure_shows_reason_and_does_not_claim_later_steps_ran() {
    let progress = DeleteProgress {
        stage: DeleteStage::Metadata,
        failure: Some(DeleteFailure::SurvivorUnavailableOrChanged),
        ..Default::default()
    };
    let text = progress_text(progress, false, DeleteStatus::Unsafe);
    assert!(text.contains(&t!("disk_analysis.dup_delete_reason_survivor").to_string()));
    assert!(text.contains(&format!(
        "{} — {}",
        stage_label(DeleteStage::Metadata),
        t!("disk_analysis.dup_delete_step_stopped")
    )));
    assert!(text.contains(&format!(
        "{} — {}",
        stage_label(DeleteStage::Windows),
        t!("disk_analysis.dup_delete_step_waiting")
    )));
}

#[test]
fn successful_removal_marks_the_windows_step_complete() {
    let text = progress_text(
        DeleteProgress {
            stage: DeleteStage::Windows,
            ..Default::default()
        },
        false,
        DeleteStatus::Removed,
    );
    for stage in DeleteStage::ALL {
        assert!(text.contains(&format!(
            "{} — {}",
            stage_label(stage),
            t!("disk_analysis.dup_delete_step_done")
        )));
    }
}

#[test]
fn close_button_stays_visible_below_scrollable_checks_in_a_small_window() {
    let ctx = egui::Context::default();
    let mut state = DiskAnalysisState::new();
    state.duplicates.deletion.status = Some(DeleteStatus::Unsafe);
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 300.0));
    let mut close_visible = false;
    for _ in 0..3 {
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(viewport),
                ..Default::default()
            },
            |ui| render(&mut state, ui.ctx()),
        );
        close_visible = output.shapes.iter().any(|shape| {
            if let egui::Shape::Text(text) = &shape.shape {
                text.galley.text() == t!("disk_analysis.dup_delete_close").as_ref()
                    && viewport
                        .contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size()))
            } else {
                false
            }
        });
    }
    assert!(close_visible);
}
