use crate::app::shortcuts::{ShortcutAction, ShortcutBinding, ShortcutBindings};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Command {
    NewTab,
    CloseTab,
    ReopenTab,
    Back,
    Forward,
    Up,
    Refresh,
    Address,
    Computer,
    RecycleBin,
    Grid,
    Details,
    Columns,
    Miller,
    NextView,
    PreviousView,
    Preview,
    DualPanel,
    GlobalSearch,
    SelectAll,
    ClearSelection,
    InvertSelection,
    CreateFolder,
    Rename,
    Properties,
    Settings,
    Organizer,
}

pub(crate) const COMMANDS: &[Command] = &[
    Command::NewTab,
    Command::CloseTab,
    Command::ReopenTab,
    Command::Back,
    Command::Forward,
    Command::Up,
    Command::Refresh,
    Command::Address,
    Command::Computer,
    Command::RecycleBin,
    Command::Grid,
    Command::Details,
    Command::Columns,
    Command::Miller,
    Command::NextView,
    Command::PreviousView,
    Command::Preview,
    Command::DualPanel,
    Command::GlobalSearch,
    Command::SelectAll,
    Command::ClearSelection,
    Command::InvertSelection,
    Command::CreateFolder,
    Command::Rename,
    Command::Properties,
    Command::Settings,
    Command::Organizer,
];

impl Command {
    pub fn binding(self, shortcuts: &ShortcutBindings) -> Option<ShortcutBinding> {
        self.shortcut()
            .map(|action| shortcuts.get(action))
            .or_else(|| {
                (self == Self::DualPanel).then(|| ShortcutBinding::ctrl_shift(eframe::egui::Key::D))
            })
    }

    pub fn label_key(self) -> &'static str {
        match self {
            Self::NewTab => "command_palette.new_tab",
            Self::CloseTab => "command_palette.close_tab",
            Self::ReopenTab => "command_palette.reopen_tab",
            Self::Back => "command_palette.back",
            Self::Forward => "command_palette.forward",
            Self::Up => "command_palette.up",
            Self::Refresh => "command_palette.refresh",
            Self::Address => "command_palette.address",
            Self::Computer => "command_palette.computer",
            Self::RecycleBin => "command_palette.recycle_bin",
            Self::Grid => "command_palette.grid",
            Self::Details => "command_palette.details",
            Self::Columns => "command_palette.columns",
            Self::Miller => "command_palette.miller",
            Self::NextView => "command_palette.next_view",
            Self::PreviousView => "command_palette.previous_view",
            Self::Preview => "command_palette.preview",
            Self::DualPanel => "command_palette.dual_panel",
            Self::GlobalSearch => "command_palette.global_search",
            Self::SelectAll => "command_palette.select_all",
            Self::ClearSelection => "command_palette.clear_selection",
            Self::InvertSelection => "command_palette.invert_selection",
            Self::CreateFolder => "command_palette.create_folder",
            Self::Rename => "command_palette.rename",
            Self::Properties => "command_palette.properties",
            Self::Settings => "command_palette.settings",
            Self::Organizer => "command_palette.organizer",
        }
    }

    pub fn category_key(self) -> &'static str {
        match self {
            Self::NewTab | Self::CloseTab | Self::ReopenTab => "command_palette.tabs",
            Self::Back | Self::Forward | Self::Up | Self::Refresh | Self::Address => {
                "command_palette.navigation"
            }
            Self::Computer | Self::RecycleBin => "command_palette.locations",
            Self::Grid
            | Self::Details
            | Self::Columns
            | Self::Miller
            | Self::NextView
            | Self::PreviousView
            | Self::Preview
            | Self::DualPanel => "command_palette.views",
            Self::GlobalSearch => "command_palette.search",
            Self::SelectAll | Self::ClearSelection | Self::InvertSelection => {
                "command_palette.selection"
            }
            Self::CreateFolder | Self::Rename | Self::Properties => "command_palette.files",
            Self::Settings | Self::Organizer => "command_palette.tools",
        }
    }

    pub fn shortcut(self) -> Option<ShortcutAction> {
        Some(match self {
            Self::NewTab => ShortcutAction::NewTab,
            Self::CloseTab => ShortcutAction::CloseTab,
            Self::ReopenTab => ShortcutAction::ReopenClosedTab,
            Self::Back => ShortcutAction::GoBack,
            Self::Forward => ShortcutAction::GoForward,
            Self::Up => ShortcutAction::GoUp,
            Self::Refresh => ShortcutAction::Refresh,
            Self::Address => ShortcutAction::FocusAddressBar,
            Self::GlobalSearch => ShortcutAction::GlobalSearch,
            Self::SelectAll => ShortcutAction::SelectAll,
            Self::NextView => ShortcutAction::NextViewMode,
            Self::PreviousView => ShortcutAction::PreviousViewMode,
            Self::CreateFolder => ShortcutAction::CreateFolder,
            Self::Rename => ShortcutAction::Rename,
            Self::Properties => ShortcutAction::Properties,
            _ => return None,
        })
    }
}
