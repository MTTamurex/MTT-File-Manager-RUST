use eframe::egui;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum ShortcutAction {
    NewTab = 0,
    CloseTab,
    NextTab,
    PreviousTab,
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
}

impl ShortcutAction {
    pub const COUNT: usize = 16;
    pub const ALL: [Self; Self::COUNT] = [
        Self::NewTab,
        Self::CloseTab,
        Self::NextTab,
        Self::PreviousTab,
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
    ];
    pub const CONFIGURABLE_COUNT: usize = 8;
    pub const CONFIGURABLE: [Self; Self::CONFIGURABLE_COUNT] = [
        Self::NewTab,
        Self::CloseTab,
        Self::NextTab,
        Self::PreviousTab,
        Self::Refresh,
        Self::FocusAddressBar,
        Self::GlobalSearch,
        Self::PreviewSelected,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn preference_key(self) -> &'static str {
        match self {
            Self::NewTab => "shortcut_new_tab",
            Self::CloseTab => "shortcut_close_tab",
            Self::NextTab => "shortcut_next_tab",
            Self::PreviousTab => "shortcut_previous_tab",
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
        }
    }

    pub const fn translation_key(self) -> &'static str {
        match self {
            Self::NewTab => "settings.shortcut_new_tab",
            Self::CloseTab => "settings.shortcut_close_tab",
            Self::NextTab => "settings.shortcut_next_tab",
            Self::PreviousTab => "settings.shortcut_previous_tab",
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
        }
    }

    pub fn default_binding(self) -> ShortcutBinding {
        DEFAULT_BINDINGS[self.index()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShortcutBinding {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: egui::Key,
}

impl ShortcutBinding {
    pub const fn new(ctrl: bool, alt: bool, shift: bool, key: egui::Key) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            key,
        }
    }

    pub const fn plain(key: egui::Key) -> Self {
        Self::new(false, false, false, key)
    }

    pub const fn ctrl(key: egui::Key) -> Self {
        Self::new(true, false, false, key)
    }

    pub const fn ctrl_shift(key: egui::Key) -> Self {
        Self::new(true, false, true, key)
    }

    pub const fn alt(key: egui::Key) -> Self {
        Self::new(false, true, false, key)
    }

    pub const fn shift(key: egui::Key) -> Self {
        Self::new(false, false, true, key)
    }

    pub fn from_modifiers(key: egui::Key, modifiers: egui::Modifiers) -> Self {
        Self::new(modifiers.ctrl, modifiers.alt, modifiers.shift, key)
    }

    pub fn parse(value: &str) -> Option<Self> {
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut key = None;

        for token in value.split('+').map(str::trim).filter(|token| !token.is_empty()) {
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                _ => {
                    if key.is_some() {
                        return None;
                    }
                    key = parse_key_token(token);
                }
            }
        }

        Some(Self::new(ctrl, alt, shift, key?))
    }

    pub fn serialize(self) -> String {
        self.to_string()
    }

    pub fn is_alt_enter(self) -> bool {
        self.alt && !self.ctrl && !self.shift && self.key == egui::Key::Enter
    }

    pub fn is_ctrl_v(self) -> bool {
        self.ctrl && !self.alt && !self.shift && self.key == egui::Key::V
    }

    pub fn is_shift_delete(self) -> bool {
        self.shift && !self.ctrl && !self.alt && self.key == egui::Key::Delete
    }
}

impl std::fmt::Display for ShortcutBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::with_capacity(4);
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(key_display_name(self.key));
        write!(f, "{}", parts.join("+"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutValidationError {
    Conflict(ShortcutAction),
    Reserved,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutCapture {
    Cancelled,
    Binding(ShortcutBinding),
}

#[derive(Clone, Debug, Default)]
pub struct ShortcutEditorState {
    pub capturing_action: Option<ShortcutAction>,
    pub message: Option<ShortcutValidationError>,
}

impl ShortcutEditorState {
    pub fn is_capturing(&self) -> bool {
        self.capturing_action.is_some()
    }

    pub fn begin_capture(&mut self, action: ShortcutAction) {
        self.capturing_action = Some(action);
        self.message = None;
    }

    pub fn clear(&mut self) {
        self.capturing_action = None;
        self.message = None;
    }
}

#[derive(Clone, Debug)]
pub struct ShortcutBindings {
    bindings: [ShortcutBinding; ShortcutAction::COUNT],
}

impl Default for ShortcutBindings {
    fn default() -> Self {
        Self {
            bindings: DEFAULT_BINDINGS,
        }
    }
}

impl ShortcutBindings {
    pub fn from_preferences(prefs: &HashMap<String, String>) -> Self {
        let mut bindings = DEFAULT_BINDINGS;
        let mut customized = [false; ShortcutAction::COUNT];

        for action in ShortcutAction::CONFIGURABLE {
            let Some(raw) = prefs.get(action.preference_key()) else {
                continue;
            };
            let Some(candidate) = ShortcutBinding::parse(raw) else {
                continue;
            };
            if validate_binding_policy(candidate).is_err() {
                continue;
            }
            bindings[action.index()] = candidate;
            customized[action.index()] = candidate != action.default_binding();
        }

        let mut changed = true;
        let mut iterations = 0;
        while changed && iterations < ShortcutAction::COUNT {
            iterations += 1;
            changed = false;

            for action in ShortcutAction::ALL {
                let binding = bindings[action.index()];
                let duplicates: Vec<ShortcutAction> = ShortcutAction::ALL
                    .iter()
                    .copied()
                    .filter(|other| bindings[other.index()] == binding)
                    .collect();

                if duplicates.len() <= 1 {
                    continue;
                }

                let winner = duplicates
                    .iter()
                    .copied()
                    .find(|other| {
                        !customized[other.index()] && other.default_binding() == binding
                    })
                    .unwrap_or(duplicates[0]);

                for other in duplicates {
                    if other == winner {
                        continue;
                    }
                    if bindings[other.index()] != other.default_binding() {
                        bindings[other.index()] = other.default_binding();
                        customized[other.index()] = false;
                        changed = true;
                    }
                }
            }
        }

        Self { bindings }
    }

    pub fn get(&self, action: ShortcutAction) -> ShortcutBinding {
        self.bindings[action.index()]
    }

    pub fn label(&self, action: ShortcutAction) -> String {
        self.get(action).to_string()
    }

    pub fn is_default(&self, action: ShortcutAction) -> bool {
        self.get(action) == action.default_binding()
    }

    pub fn any_customized(&self) -> bool {
        ShortcutAction::CONFIGURABLE
            .iter()
            .copied()
            .any(|action| !self.is_default(action))
    }

    pub fn reset(&mut self, action: ShortcutAction) {
        self.bindings[action.index()] = action.default_binding();
    }

    pub fn reset_all(&mut self) {
        for action in ShortcutAction::CONFIGURABLE {
            self.bindings[action.index()] = action.default_binding();
        }
    }

    pub fn set(&mut self, action: ShortcutAction, binding: ShortcutBinding) {
        self.bindings[action.index()] = binding;
    }

    pub fn validate_candidate(
        &self,
        action: ShortcutAction,
        binding: ShortcutBinding,
    ) -> Result<(), ShortcutValidationError> {
        validate_binding_policy(binding)?;
        if let Some(other) = self.find_conflict(action, binding) {
            return Err(ShortcutValidationError::Conflict(other));
        }
        Ok(())
    }

    pub fn find_conflict(
        &self,
        action: ShortcutAction,
        binding: ShortcutBinding,
    ) -> Option<ShortcutAction> {
        ShortcutAction::ALL.iter().copied().find(|other| {
            *other != action && self.bindings[other.index()] == binding
        })
    }

    pub fn append_preferences(&self, prefs: &mut Vec<(&'static str, String)>) {
        for action in ShortcutAction::CONFIGURABLE {
            prefs.push((action.preference_key(), self.get(action).serialize()));
        }
    }

    pub fn is_triggered(&self, action: ShortcutAction, ctx: &egui::Context) -> bool {
        let binding = self.get(action);
        ctx.input(|i| i.events.iter().any(|event| binding_from_event(event) == Some(binding)))
    }
}

pub fn capture_shortcut(ctx: &egui::Context) -> Option<ShortcutCapture> {
    ctx.input(|i| {
        for event in &i.events {
            match event {
                egui::Event::Key {
                    key,
                    pressed,
                    modifiers,
                    ..
                } if *pressed && *key == egui::Key::Escape && !modifiers.ctrl && !modifiers.alt && !modifiers.shift => {
                    return Some(ShortcutCapture::Cancelled);
                }
                _ => {
                    if let Some(binding) = binding_from_event(event) {
                        return Some(ShortcutCapture::Binding(binding));
                    }
                }
            }
        }
        None
    })
}

const DEFAULT_BINDINGS: [ShortcutBinding; ShortcutAction::COUNT] = [
    ShortcutBinding::ctrl(egui::Key::T),
    ShortcutBinding::ctrl(egui::Key::W),
    ShortcutBinding::ctrl(egui::Key::Tab),
    ShortcutBinding::ctrl_shift(egui::Key::Tab),
    ShortcutBinding::ctrl(egui::Key::C),
    ShortcutBinding::ctrl(egui::Key::X),
    ShortcutBinding::ctrl(egui::Key::V),
    ShortcutBinding::plain(egui::Key::F2),
    ShortcutBinding::plain(egui::Key::Delete),
    ShortcutBinding::shift(egui::Key::Delete),
    ShortcutBinding::plain(egui::Key::F5),
    ShortcutBinding::ctrl(egui::Key::L),
    ShortcutBinding::ctrl_shift(egui::Key::F),
    ShortcutBinding::alt(egui::Key::Enter),
    ShortcutBinding::ctrl_shift(egui::Key::N),
    ShortcutBinding::plain(egui::Key::Space),
];

fn binding_from_event(event: &egui::Event) -> Option<ShortcutBinding> {
    match event {
        egui::Event::Copy => Some(ShortcutBinding::ctrl(egui::Key::C)),
        egui::Event::Cut => Some(ShortcutBinding::ctrl(egui::Key::X)),
        egui::Event::Paste(_) => Some(ShortcutBinding::ctrl(egui::Key::V)),
        egui::Event::Key {
            key,
            pressed,
            modifiers,
            ..
        } if *pressed => Some(ShortcutBinding::from_modifiers(*key, *modifiers)),
        _ => None,
    }
}

fn validate_binding_policy(binding: ShortcutBinding) -> Result<(), ShortcutValidationError> {
    if !is_supported_key(binding.key) {
        return Err(ShortcutValidationError::Unsupported);
    }

    if !binding.ctrl && !binding.alt && !binding.shift && !is_allowed_unmodified_key(binding.key)
    {
        return Err(ShortcutValidationError::Unsupported);
    }

    if is_reserved_binding(binding) {
        return Err(ShortcutValidationError::Reserved);
    }

    Ok(())
}

fn is_reserved_binding(binding: ShortcutBinding) -> bool {
    matches!(binding, b if b.alt && !b.ctrl && !b.shift && b.key == egui::Key::Tab)
        || matches!(binding, b if b.alt && !b.ctrl && b.key == egui::Key::F4)
        || matches!(binding, b if b.alt && !b.ctrl && !b.shift && b.key == egui::Key::Space)
        || matches!(binding, b if b.ctrl && !b.alt && !b.shift && b.key == egui::Key::Escape)
        || matches!(binding, b if b.ctrl && !b.alt && b.shift && b.key == egui::Key::Escape)
        || matches!(binding, b if b.ctrl && b.alt && !b.shift && b.key == egui::Key::Delete)
}

fn is_allowed_unmodified_key(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::F1
            | egui::Key::F2
            | egui::Key::F3
            | egui::Key::F4
            | egui::Key::F5
            | egui::Key::F6
            | egui::Key::F7
            | egui::Key::F8
            | egui::Key::F9
            | egui::Key::F10
            | egui::Key::F11
            | egui::Key::F12
            | egui::Key::Delete
            | egui::Key::Space
    )
}

fn is_supported_key(key: egui::Key) -> bool {
    matches!(
        key,
        egui::Key::A
            | egui::Key::B
            | egui::Key::C
            | egui::Key::D
            | egui::Key::E
            | egui::Key::F
            | egui::Key::G
            | egui::Key::H
            | egui::Key::I
            | egui::Key::J
            | egui::Key::K
            | egui::Key::L
            | egui::Key::M
            | egui::Key::N
            | egui::Key::O
            | egui::Key::P
            | egui::Key::Q
            | egui::Key::R
            | egui::Key::S
            | egui::Key::T
            | egui::Key::U
            | egui::Key::V
            | egui::Key::W
            | egui::Key::X
            | egui::Key::Y
            | egui::Key::Z
            | egui::Key::Num0
            | egui::Key::Num1
            | egui::Key::Num2
            | egui::Key::Num3
            | egui::Key::Num4
            | egui::Key::Num5
            | egui::Key::Num6
            | egui::Key::Num7
            | egui::Key::Num8
            | egui::Key::Num9
            | egui::Key::Tab
            | egui::Key::Enter
            | egui::Key::Space
            | egui::Key::Delete
            | egui::Key::F1
            | egui::Key::F2
            | egui::Key::F3
            | egui::Key::F4
            | egui::Key::F5
            | egui::Key::F6
            | egui::Key::F7
            | egui::Key::F8
            | egui::Key::F9
            | egui::Key::F10
            | egui::Key::F11
            | egui::Key::F12
    )
}

fn parse_key_token(token: &str) -> Option<egui::Key> {
    match token.to_ascii_lowercase().as_str() {
        "a" => Some(egui::Key::A),
        "b" => Some(egui::Key::B),
        "c" => Some(egui::Key::C),
        "d" => Some(egui::Key::D),
        "e" => Some(egui::Key::E),
        "f" => Some(egui::Key::F),
        "g" => Some(egui::Key::G),
        "h" => Some(egui::Key::H),
        "i" => Some(egui::Key::I),
        "j" => Some(egui::Key::J),
        "k" => Some(egui::Key::K),
        "l" => Some(egui::Key::L),
        "m" => Some(egui::Key::M),
        "n" => Some(egui::Key::N),
        "o" => Some(egui::Key::O),
        "p" => Some(egui::Key::P),
        "q" => Some(egui::Key::Q),
        "r" => Some(egui::Key::R),
        "s" => Some(egui::Key::S),
        "t" => Some(egui::Key::T),
        "u" => Some(egui::Key::U),
        "v" => Some(egui::Key::V),
        "w" => Some(egui::Key::W),
        "x" => Some(egui::Key::X),
        "y" => Some(egui::Key::Y),
        "z" => Some(egui::Key::Z),
        "0" => Some(egui::Key::Num0),
        "1" => Some(egui::Key::Num1),
        "2" => Some(egui::Key::Num2),
        "3" => Some(egui::Key::Num3),
        "4" => Some(egui::Key::Num4),
        "5" => Some(egui::Key::Num5),
        "6" => Some(egui::Key::Num6),
        "7" => Some(egui::Key::Num7),
        "8" => Some(egui::Key::Num8),
        "9" => Some(egui::Key::Num9),
        "tab" => Some(egui::Key::Tab),
        "enter" => Some(egui::Key::Enter),
        "space" => Some(egui::Key::Space),
        "delete" | "del" => Some(egui::Key::Delete),
        "f1" => Some(egui::Key::F1),
        "f2" => Some(egui::Key::F2),
        "f3" => Some(egui::Key::F3),
        "f4" => Some(egui::Key::F4),
        "f5" => Some(egui::Key::F5),
        "f6" => Some(egui::Key::F6),
        "f7" => Some(egui::Key::F7),
        "f8" => Some(egui::Key::F8),
        "f9" => Some(egui::Key::F9),
        "f10" => Some(egui::Key::F10),
        "f11" => Some(egui::Key::F11),
        "f12" => Some(egui::Key::F12),
        _ => None,
    }
}

fn key_display_name(key: egui::Key) -> &'static str {
    match key {
        egui::Key::A => "A",
        egui::Key::B => "B",
        egui::Key::C => "C",
        egui::Key::D => "D",
        egui::Key::E => "E",
        egui::Key::F => "F",
        egui::Key::G => "G",
        egui::Key::H => "H",
        egui::Key::I => "I",
        egui::Key::J => "J",
        egui::Key::K => "K",
        egui::Key::L => "L",
        egui::Key::M => "M",
        egui::Key::N => "N",
        egui::Key::O => "O",
        egui::Key::P => "P",
        egui::Key::Q => "Q",
        egui::Key::R => "R",
        egui::Key::S => "S",
        egui::Key::T => "T",
        egui::Key::U => "U",
        egui::Key::V => "V",
        egui::Key::W => "W",
        egui::Key::X => "X",
        egui::Key::Y => "Y",
        egui::Key::Z => "Z",
        egui::Key::Num0 => "0",
        egui::Key::Num1 => "1",
        egui::Key::Num2 => "2",
        egui::Key::Num3 => "3",
        egui::Key::Num4 => "4",
        egui::Key::Num5 => "5",
        egui::Key::Num6 => "6",
        egui::Key::Num7 => "7",
        egui::Key::Num8 => "8",
        egui::Key::Num9 => "9",
        egui::Key::Tab => "Tab",
        egui::Key::Enter => "Enter",
        egui::Key::Space => "Space",
        egui::Key::Delete => "Delete",
        egui::Key::F1 => "F1",
        egui::Key::F2 => "F2",
        egui::Key::F3 => "F3",
        egui::Key::F4 => "F4",
        egui::Key::F5 => "F5",
        egui::Key::F6 => "F6",
        egui::Key::F7 => "F7",
        egui::Key::F8 => "F8",
        egui::Key::F9 => "F9",
        egui::Key::F10 => "F10",
        egui::Key::F11 => "F11",
        egui::Key::F12 => "F12",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_serializes_default_bindings() {
        for action in ShortcutAction::ALL {
            let binding = action.default_binding();
            let serialized = binding.serialize();
            assert_eq!(ShortcutBinding::parse(&serialized), Some(binding));
        }
    }

    #[test]
    fn rejects_reserved_windows_shortcuts() {
        assert_eq!(
            validate_binding_policy(ShortcutBinding::alt(egui::Key::F4)),
            Err(ShortcutValidationError::Reserved)
        );
        assert_eq!(
            validate_binding_policy(ShortcutBinding::new(true, true, false, egui::Key::Delete)),
            Err(ShortcutValidationError::Reserved)
        );
    }

    #[test]
    fn keeps_valid_swaps_from_preferences() {
        let mut prefs = HashMap::new();
        prefs.insert("shortcut_new_tab".to_string(), "Ctrl+W".to_string());
        prefs.insert("shortcut_close_tab".to_string(), "Ctrl+T".to_string());

        let bindings = ShortcutBindings::from_preferences(&prefs);

        assert_eq!(
            bindings.get(ShortcutAction::NewTab),
            ShortcutBinding::ctrl(egui::Key::W)
        );
        assert_eq!(
            bindings.get(ShortcutAction::CloseTab),
            ShortcutBinding::ctrl(egui::Key::T)
        );
    }

    #[test]
    fn duplicate_preference_falls_back_to_default_owner() {
        let mut prefs = HashMap::new();
        prefs.insert("shortcut_new_tab".to_string(), "Ctrl+W".to_string());

        let bindings = ShortcutBindings::from_preferences(&prefs);

        assert_eq!(
            bindings.get(ShortcutAction::CloseTab),
            ShortcutBinding::ctrl(egui::Key::W)
        );
        assert_eq!(
            bindings.get(ShortcutAction::NewTab),
            ShortcutBinding::ctrl(egui::Key::T)
        );
    }

    #[test]
    fn ignores_preferences_for_fixed_file_shortcuts() {
        let mut prefs = HashMap::new();
        prefs.insert("shortcut_copy".to_string(), "Ctrl+Q".to_string());
        prefs.insert("shortcut_delete".to_string(), "F8".to_string());

        let bindings = ShortcutBindings::from_preferences(&prefs);

        assert_eq!(
            bindings.get(ShortcutAction::Copy),
            ShortcutBinding::ctrl(egui::Key::C)
        );
        assert_eq!(
            bindings.get(ShortcutAction::Delete),
            ShortcutBinding::plain(egui::Key::Delete)
        );
    }
}