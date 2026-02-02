/// State for inline expandable menus in video controls
#[derive(Default, Clone, PartialEq)]
pub enum ExpandedMenu {
    #[default]
    None,
    AudioTracks,
    Subtitles,
}

#[derive(Default, Clone)]
pub struct VideoControlsState {
    pub expanded_menu: ExpandedMenu,
}

impl VideoControlsState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle_audio_menu(&mut self) {
        self.expanded_menu = if self.expanded_menu == ExpandedMenu::AudioTracks {
            ExpandedMenu::None
        } else {
            ExpandedMenu::AudioTracks
        };
    }

    pub fn toggle_subtitle_menu(&mut self) {
        self.expanded_menu = if self.expanded_menu == ExpandedMenu::Subtitles {
            ExpandedMenu::None
        } else {
            ExpandedMenu::Subtitles
        };
    }

    pub fn close_menus(&mut self) {
        self.expanded_menu = ExpandedMenu::None;
    }

    pub fn is_audio_menu_open(&self) -> bool {
        self.expanded_menu == ExpandedMenu::AudioTracks
    }

    pub fn is_subtitle_menu_open(&self) -> bool {
        self.expanded_menu == ExpandedMenu::Subtitles
    }
}
