use super::ShortcutBinding;
use eframe::egui::Key;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum ShortcutAction {
    NewTab = 0,
    CloseTab,
    NextTab,
    PreviousTab,
    MoveTabLeft,
    MoveTabRight,
    Copy,
    Cut,
    Paste,
    Rename,
    Delete,
    DeletePermanently,
    Refresh,
    FocusAddressBar,
    GlobalSearch,
    Properties,
    CreateFolder,
    PreviewSelected,
    SelectAll,
    NextViewMode,
    PreviousViewMode,
    GoBack,
    GoForward,
    GoUp,
    ReopenClosedTab,
}

impl ShortcutAction {
    pub const COUNT: usize = 25;
    pub const ALL: [Self; Self::COUNT] = [
        Self::NewTab,
        Self::CloseTab,
        Self::NextTab,
        Self::PreviousTab,
        Self::MoveTabLeft,
        Self::MoveTabRight,
        Self::Copy,
        Self::Cut,
        Self::Paste,
        Self::Rename,
        Self::Delete,
        Self::DeletePermanently,
        Self::Refresh,
        Self::FocusAddressBar,
        Self::GlobalSearch,
        Self::Properties,
        Self::CreateFolder,
        Self::PreviewSelected,
        Self::SelectAll,
        Self::NextViewMode,
        Self::PreviousViewMode,
        Self::GoBack,
        Self::GoForward,
        Self::GoUp,
        Self::ReopenClosedTab,
    ];
    pub const CONFIGURABLE_COUNT: usize = 16;
    pub const CONFIGURABLE: [Self; Self::CONFIGURABLE_COUNT] = [
        Self::NewTab,
        Self::CloseTab,
        Self::ReopenClosedTab,
        Self::NextTab,
        Self::PreviousTab,
        Self::MoveTabLeft,
        Self::MoveTabRight,
        Self::Refresh,
        Self::FocusAddressBar,
        Self::GlobalSearch,
        Self::PreviewSelected,
        Self::NextViewMode,
        Self::PreviousViewMode,
        Self::GoBack,
        Self::GoForward,
        Self::GoUp,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn preference_key(self) -> &'static str {
        match self {
            Self::NewTab => "shortcut_new_tab",
            Self::CloseTab => "shortcut_close_tab",
            Self::ReopenClosedTab => "shortcut_reopen_closed_tab",
            Self::NextTab => "shortcut_next_tab",
            Self::PreviousTab => "shortcut_previous_tab",
            Self::MoveTabLeft => "shortcut_move_tab_left",
            Self::MoveTabRight => "shortcut_move_tab_right",
            Self::Copy => "shortcut_copy",
            Self::Cut => "shortcut_cut",
            Self::Paste => "shortcut_paste",
            Self::Rename => "shortcut_rename",
            Self::Delete => "shortcut_delete",
            Self::DeletePermanently => "shortcut_delete_permanently",
            Self::Refresh => "shortcut_refresh",
            Self::FocusAddressBar => "shortcut_focus_address_bar",
            Self::GlobalSearch => "shortcut_global_search",
            Self::Properties => "shortcut_properties",
            Self::CreateFolder => "shortcut_create_folder",
            Self::PreviewSelected => "shortcut_preview_selected",
            Self::SelectAll => "shortcut_select_all",
            Self::NextViewMode => "shortcut_next_view_mode",
            Self::PreviousViewMode => "shortcut_previous_view_mode",
            Self::GoBack => "shortcut_go_back",
            Self::GoForward => "shortcut_go_forward",
            Self::GoUp => "shortcut_go_up",
        }
    }

    pub const fn translation_key(self) -> &'static str {
        match self {
            Self::NewTab => "settings.shortcut_new_tab",
            Self::CloseTab => "settings.shortcut_close_tab",
            Self::ReopenClosedTab => "settings.shortcut_reopen_closed_tab",
            Self::NextTab => "settings.shortcut_next_tab",
            Self::PreviousTab => "settings.shortcut_previous_tab",
            Self::MoveTabLeft => "settings.shortcut_move_tab_left",
            Self::MoveTabRight => "settings.shortcut_move_tab_right",
            Self::Copy => "settings.shortcut_copy",
            Self::Cut => "settings.shortcut_cut",
            Self::Paste => "settings.shortcut_paste",
            Self::Rename => "settings.shortcut_rename",
            Self::Delete => "settings.shortcut_delete",
            Self::DeletePermanently => "settings.shortcut_delete_permanently",
            Self::Refresh => "settings.shortcut_refresh",
            Self::FocusAddressBar => "settings.shortcut_focus_address_bar",
            Self::GlobalSearch => "settings.shortcut_global_search",
            Self::Properties => "settings.shortcut_properties",
            Self::CreateFolder => "settings.shortcut_create_folder",
            Self::PreviewSelected => "settings.shortcut_preview_selected",
            Self::SelectAll => "settings.shortcut_select_all",
            Self::NextViewMode => "settings.shortcut_next_view_mode",
            Self::PreviousViewMode => "settings.shortcut_previous_view_mode",
            Self::GoBack => "settings.shortcut_go_back",
            Self::GoForward => "settings.shortcut_go_forward",
            Self::GoUp => "settings.shortcut_go_up",
        }
    }

    pub fn default_binding(self) -> ShortcutBinding {
        DEFAULT_BINDINGS[self.index()]
    }
}

pub(super) const DEFAULT_BINDINGS: [ShortcutBinding; ShortcutAction::COUNT] = [
    ShortcutBinding::ctrl(Key::T),
    ShortcutBinding::ctrl(Key::W),
    ShortcutBinding::ctrl(Key::Tab),
    ShortcutBinding::ctrl_shift(Key::Tab),
    ShortcutBinding::ctrl_shift(Key::PageUp),
    ShortcutBinding::ctrl_shift(Key::PageDown),
    ShortcutBinding::ctrl(Key::C),
    ShortcutBinding::ctrl(Key::X),
    ShortcutBinding::ctrl(Key::V),
    ShortcutBinding::plain(Key::F2),
    ShortcutBinding::plain(Key::Delete),
    ShortcutBinding::shift(Key::Delete),
    ShortcutBinding::plain(Key::F5),
    ShortcutBinding::ctrl(Key::L),
    ShortcutBinding::ctrl_shift(Key::F),
    ShortcutBinding::alt(Key::Enter),
    ShortcutBinding::ctrl_shift(Key::N),
    ShortcutBinding::plain(Key::Space),
    ShortcutBinding::ctrl(Key::A),
    ShortcutBinding::scroll(true, false, false, false),
    ShortcutBinding::scroll(true, false, false, true),
    ShortcutBinding::alt(Key::ArrowLeft),
    ShortcutBinding::alt(Key::ArrowRight),
    ShortcutBinding::alt(Key::ArrowUp),
    ShortcutBinding::ctrl_shift(Key::T),
];
