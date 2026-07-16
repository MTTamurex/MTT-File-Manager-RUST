use crate::app::global_search_state::GlobalSearchInteractionTarget;
use crate::app::state::ImageViewerApp;
use crate::ui::views::common::should_start_item_drag;
use eframe::egui;

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_selection_and_context_menu(
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    response: &egui::Response,
    source_index: usize,
    ordered_indices: &[usize],
    row_rect: egui::Rect,
    full_path: &str,
    is_directory: bool,
) {
    if response.clicked() {
        let modifiers = ctx.input(|input| input.modifiers);
        if modifiers.shift {
            app.global_search
                .select_result_range(ordered_indices, source_index);
        } else if modifiers.ctrl {
            app.global_search.toggle_result_selection(source_index);
        } else {
            app.global_search.select_single_result(source_index);
        }
        return;
    }

    if !response.secondary_clicked() {
        return;
    }

    if !app.global_search.selected_indices.contains(&source_index) {
        app.global_search.select_single_result(source_index);
    } else {
        app.global_search.selected_index = Some(source_index);
        app.global_search.interaction_target = GlobalSearchInteractionTarget::Results;
    }

    let primary_path = std::path::PathBuf::from(full_path);
    let mut selected_paths = vec![primary_path.clone()];
    selected_paths.extend(
        ordered_indices
            .iter()
            .filter(|idx| app.global_search.selected_indices.contains(idx))
            .filter_map(|idx| app.global_search.results.get(*idx))
            .map(|result| std::path::PathBuf::from(&result.full_path))
            .filter(|path| path != &primary_path),
    );
    let pointer_pos = ctx.pointer_latest_pos().unwrap_or(row_rect.center());
    app.context_menu.open_for_global_search(
        pointer_pos,
        ctx.screen_rect().right(),
        selected_paths.clone(),
        is_directory,
    );
    app.populate_context_menu(ctx, &selected_paths, false, None);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn maybe_start_drag(
    app: &mut ImageViewerApp,
    ctx: &egui::Context,
    response: &egui::Response,
    source_index: usize,
    ordered_indices: &[usize],
    is_directory: bool,
    is_renaming: bool,
    folder_button_rect: egui::Rect,
    open_button_rect: egui::Rect,
    icon: Option<egui::TextureHandle>,
) {
    let (press_origin, pointer_pos, primary_down) = ctx.input(|input| {
        (
            input.pointer.press_origin(),
            input.pointer.interact_pos(),
            input.pointer.button_down(egui::PointerButton::Primary),
        )
    });
    let pressed_action_button = press_origin.is_some_and(|origin| {
        folder_button_rect.contains(origin) || open_button_rect.contains(origin)
    });
    if is_renaming
        || pressed_action_button
        || !should_start_item_drag(
            response.drag_started(),
            response.dragged(),
            response.is_pointer_button_down_on() && primary_down,
            press_origin,
            pointer_pos,
        )
    {
        return;
    }

    if !app.global_search.selected_indices.contains(&source_index) {
        app.global_search.select_single_result(source_index);
    }
    let paths = ordered_indices
        .iter()
        .filter(|idx| app.global_search.selected_indices.contains(idx))
        .filter_map(|idx| app.global_search.results.get(*idx))
        .map(|result| std::path::PathBuf::from(&result.full_path))
        .collect();
    app.begin_global_search_drag(paths, is_directory, icon);
}
