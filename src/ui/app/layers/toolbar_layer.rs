use crate::app::ImageViewerApp;
use eframe::egui;
use rust_i18n::t;

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
                &mut app.show_address_history_menu,
                &mut app.address_bar_focus_request,
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
                if !matches!(
                    act,
                    ToolbarAction::Search(_)
                        | ToolbarAction::ChangeSortMode(_)
                        | ToolbarAction::ToggleSortDescending
                        | ToolbarAction::ToggleViewMode
                        | ToolbarAction::TogglePreviewPanel
                ) {
                    app.show_address_history_menu = false;
                }

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
                        if !app.current_folder_locked {
                            app.view_mode_normal = app.view_mode;
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
                        if !app.current_folder_locked {
                            app.sort_descending_normal = app.sort_descending;
                        }
                        app.sort_items();
                        app.save_preferences();
                    }
                    ToolbarAction::Search(_query) => app.filter_items(),
                    ToolbarAction::Navigate(path) => app.navigate_to(&path),
                    ToolbarAction::StartAddressEdit => {
                        app.navigation_state.path_input = if app.navigation_state.current_path == crate::domain::special_paths::COMPUTER_VIEW_ID {
                            t!("nav.computer").to_string()
                        } else if app.navigation_state.current_path == crate::domain::special_paths::RECYCLE_BIN_VIEW_ID {
                            t!("nav.recycle_bin").to_string()
                        } else {
                            app.navigation_state.current_path.clone()
                        };
                        app.is_address_editing = true;
                        app.show_address_history_menu = false;
                    }
                    ToolbarAction::StartAddressEditWithHistory => {
                        app.navigation_state.path_input = if app.navigation_state.current_path == crate::domain::special_paths::COMPUTER_VIEW_ID {
                            t!("nav.computer").to_string()
                        } else if app.navigation_state.current_path == crate::domain::special_paths::RECYCLE_BIN_VIEW_ID {
                            t!("nav.recycle_bin").to_string()
                        } else {
                            app.navigation_state.current_path.clone()
                        };
                        app.is_address_editing = true;
                        app.show_address_history_menu = true;
                    }
                    ToolbarAction::CommitPathInput(path) => {
                        // Enter used to commit path input must not trigger "open selected"
                        // in list/grid handlers during the same frame.
                        app.suppress_next_enter_open = true;

                        if crate::infrastructure::onedrive::fast_path_exists(std::path::Path::new(
                            &path,
                        )) {
                            app.navigate_to(&path);
                            app.is_address_editing = false;
                        } else {
                            app.notifications.error(format!(
                                "{}",
                                t!("operations.path_not_found", path = path)
                            ));
                            app.navigation_state.path_input = app.navigation_state.current_path.clone();
                            app.is_address_editing = false;
                        }
                    }
                    ToolbarAction::CancelPathInput => {
                        app.is_address_editing = false;
                        app.navigation_state.path_input = app.navigation_state.current_path.clone();
                    }
                    ToolbarAction::SelectAddressHistoryPath(path) => {
                        app.navigate_to(&path);
                        app.is_address_editing = false;
                    }
                    _ => {}
                }
            }
        });
}
