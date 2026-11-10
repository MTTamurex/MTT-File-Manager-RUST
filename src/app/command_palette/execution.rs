use super::{
    availability::{disabled_reason, CommandContext},
    catalog::Command,
    file_actions,
};
use crate::app::{navigation_state::SettingsSection, ImageViewerApp};
use crate::domain::file_entry::ViewMode;
use eframe::egui;

impl ImageViewerApp {
    /// Called after rendering, with the palette closed. Never dispatch through stale menu targets.
    pub(crate) fn execute_palette_command(&mut self, command: Command, ctx: &egui::Context) {
        if self.command_palette.active
            || self.command_palette_blocked()
            || crate::app::operations::tabs::transfer_pending(ctx)
        {
            return;
        }
        let context = CommandContext::from_app(self);
        if let Some(reason) = disabled_reason(command, &context) {
            self.notifications
                .warning(rust_i18n::t!(reason).to_string());
            return;
        }
        match command {
            Command::NewTab => self.create_default_tab(),
            Command::CloseTab => self.close_main_tab(self.tab_manager.active_tab),
            Command::ReopenTab => self.reopen_closed_main_tab(),
            Command::Back => self.go_back(),
            Command::Forward => self.go_forward(),
            Command::Up => self.go_up_one_level(),
            Command::Refresh => self.trigger_manual_refresh(),
            Command::Address => {
                let display =
                    self.tag_view_display_name_for_path(&self.navigation_state.current_path);
                self.navigation_state.path_input = crate::ui::address_bar::editable_path(
                    &self.navigation_state.current_path,
                    display.as_deref(),
                );
                self.is_address_editing = true;
                self.show_address_history_menu = false;
                self.address_bar_focus_request = true;
            }
            Command::Grid => {
                self.change_view_mode(ViewMode::Grid);
            }
            Command::Details => {
                self.change_view_mode(ViewMode::List);
            }
            Command::Columns => {
                self.change_view_mode(ViewMode::ColumnList);
            }
            Command::Miller => {
                self.change_view_mode(ViewMode::Miller);
            }
            Command::NextView => {
                self.change_view_mode(self.view_mode.cycle(true));
            }
            Command::PreviousView => {
                self.change_view_mode(self.view_mode.cycle(false));
            }
            Command::Preview => self.toggle_preview_panel(),
            Command::Sidebar => self.toggle_left_sidebar(),
            Command::DualPanel => self.dual_panel_toggle(),
            Command::GlobalSearch => self.toggle_global_search(),
            Command::SelectAll => {
                self.select_all_current_items();
            }
            Command::ClearSelection => self.clear_file_view_selection(),
            Command::InvertSelection => {
                self.invert_current_items_selection();
            }
            Command::CreateFolder => self.create_new_folder(),
            Command::Rename => {
                if self.multi_selection.len() > 1 {
                    self.begin_batch_rename();
                } else if let Some(path) = self.selected_file.as_ref().map(|item| item.path.clone())
                {
                    self.begin_rename_path(&path);
                }
            }
            Command::Properties => {
                let paths = if self.multi_selection.is_empty() {
                    self.selected_file
                        .iter()
                        .map(|item| item.path.clone())
                        .collect()
                } else {
                    self.multi_selection.iter().cloned().collect()
                };
                self.show_properties_for_paths(paths);
            }
            Command::Settings => self.open_settings(SettingsSection::General),
            Command::PreviewSelected => {
                self.trigger_selected_preview_overlay_action();
            }
            Command::Copy | Command::Cut | Command::Paste | Command::Delete => {
                file_actions::execute(self, command)
            }
            Command::DiskAnalysis => {
                if let Some(letter) = context.disk_analysis_drive {
                    self.open_disk_analysis(letter);
                }
            }
        }
        ctx.request_repaint();
    }
}
