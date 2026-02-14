use crate::app::ImageViewerApp;
use eframe::egui;

#[cfg(debug_assertions)]
macro_rules! debug_log {
    ($($arg:tt)*) => { log::debug!($($arg)*) }
}

#[cfg(not(debug_assertions))]
macro_rules! debug_log {
    ($($arg:tt)*) => {};
}

pub(crate) fn render_toolbar_layer(app: &mut ImageViewerApp, ctx: &egui::Context) {
    egui::TopBottomPanel::top("nav_bar")
        .show_separator_line(true)
        .exact_height(46.0)
        .frame(egui::Frame {
            fill: if ctx.style().visuals.dark_mode {
                egui::Color32::from_rgb(45, 45, 45)
            } else {
                egui::Color32::from_rgb(243, 243, 243)
            },
            inner_margin: egui::Margin {
                left: 8,
                right: 8,
                top: 7,
                bottom: 7,
            },
            ..Default::default()
        })
        .show(ctx, |ui| {
            use crate::domain::file_entry::ViewMode;
            use crate::ui::toolbar::{render_toolbar, ToolbarAction};

            let action = render_toolbar(
                ui,
                &app.navigation_state.current_path,
                &mut app.navigation_state.path_input,
                &mut app.is_address_editing,
                &mut app.search_query,
                &app.navigation_state.navigation,
                app.view_mode,
                app.sort_mode,
                app.sort_descending,
                &mut app.thumbnail_size,
                app.show_preview_panel,
                app.renaming_state.is_some(),
                app.cache_manager.computer_icon.as_ref(),
                &mut app.svg_icon_manager,
            );

            if let Some(act) = action {
                match act {
                    ToolbarAction::GoBack => app.go_back(),
                    ToolbarAction::GoForward => app.go_forward(),
                    ToolbarAction::GoUp => app.go_up_one_level(),
                    ToolbarAction::Refresh => app.trigger_manual_refresh(),
                    ToolbarAction::CreateFolder => app.create_new_folder(),
                    ToolbarAction::NavigateToComputer => app.navigate_to_computer(),
                    ToolbarAction::NavigateToRecycleBin => app.navigate_to_recycle_bin(),
                    ToolbarAction::ToggleViewMode => {
                        if app.view_mode == ViewMode::List {
                            app.view_mode = ViewMode::Grid;
                        } else {
                            app.view_mode = ViewMode::List;
                        }
                        debug_log!(
                            "[VIEW-MODE] Toolbar toggle -> {:?} (tab={})",
                            app.view_mode,
                            app.tab_manager.active_tab
                        );
                    }
                    ToolbarAction::TogglePreviewPanel => {
                        app.show_preview_panel = !app.show_preview_panel;
                        app.update_video_visibility();
                    }
                    ToolbarAction::ChangeSortMode(mode) => {
                        app.sort_mode = mode;
                        if app.navigation_state.is_computer_view {
                            app.sort_mode_computer = mode;
                        } else {
                            app.sort_mode_normal = mode;
                        }
                        app.sort_items();
                        app.save_preferences();
                    }
                    ToolbarAction::ToggleSortDescending => {
                        app.sort_descending = !app.sort_descending;
                        app.sort_items();
                        app.save_preferences();
                    }
                    ToolbarAction::Search(_query) => app.filter_items(),
                    ToolbarAction::Navigate(path) => app.navigate_to(&path),
                    ToolbarAction::StartAddressEdit => {
                        app.navigation_state.path_input = app.navigation_state.current_path.clone();
                        app.is_address_editing = true;
                    }
                    ToolbarAction::CommitPathInput(path) => {
                        if crate::infrastructure::onedrive::fast_path_exists(std::path::Path::new(
                            &path,
                        )) {
                            app.navigate_to(&path);
                            app.is_address_editing = false;
                        } else {
                            app.navigation_state.path_input = app.navigation_state.current_path.clone();
                            app.is_address_editing = false;
                        }
                    }
                    ToolbarAction::CancelPathInput => {
                        app.is_address_editing = false;
                        app.navigation_state.path_input = app.navigation_state.current_path.clone();
                    }
                    _ => {}
                }
            }
        });
}
