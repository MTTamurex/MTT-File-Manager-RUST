use super::ShortcutValidationError;
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShortcutTrigger {
    Key(egui::Key),
    ScrollUp,
    ScrollDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ShortcutBinding {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub trigger: ShortcutTrigger,
}

impl ShortcutBinding {
    pub const fn new(ctrl: bool, alt: bool, shift: bool, key: egui::Key) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            trigger: ShortcutTrigger::Key(key),
        }
    }
    pub const fn scroll(ctrl: bool, alt: bool, shift: bool, up: bool) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            trigger: if up {
                ShortcutTrigger::ScrollUp
            } else {
                ShortcutTrigger::ScrollDown
            },
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
    pub fn is_scroll(self) -> bool {
        !matches!(self.trigger, ShortcutTrigger::Key(_))
    }
    pub fn parse(value: &str) -> Option<Self> {
        let mut ctrl = false;
        let mut alt = false;
        let mut shift = false;
        let mut trigger = None;
        for token in value
            .split('+')
            .map(str::trim)
            .filter(|token| !token.is_empty())
        {
            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => ctrl = true,
                "alt" => alt = true,
                "shift" => shift = true,
                name => {
                    if trigger.is_some() {
                        return None;
                    }
                    trigger = Some(match name {
                        "scrollup" => ShortcutTrigger::ScrollUp,
                        "scrolldown" => ShortcutTrigger::ScrollDown,
                        "pgup" => ShortcutTrigger::Key(egui::Key::PageUp),
                        "pgdn" => ShortcutTrigger::Key(egui::Key::PageDown),
                        "del" => ShortcutTrigger::Key(egui::Key::Delete),
                        _ => ShortcutTrigger::Key(
                            egui::Key::ALL
                                .iter()
                                .copied()
                                .find(|key| key.name().eq_ignore_ascii_case(token))?,
                        ),
                    });
                }
            }
        }
        Some(Self {
            ctrl,
            alt,
            shift,
            trigger: trigger?,
        })
    }
    pub fn serialize(self) -> String {
        self.to_string()
    }
    pub fn is_alt_enter(self) -> bool {
        self == Self::alt(egui::Key::Enter)
    }
    pub fn is_ctrl_v(self) -> bool {
        self == Self::ctrl(egui::Key::V)
    }
    pub fn is_shift_delete(self) -> bool {
        self == Self::shift(egui::Key::Delete)
    }
}

impl std::fmt::Display for ShortcutBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ctrl {
            write!(f, "Ctrl+")?;
        }
        if self.alt {
            write!(f, "Alt+")?;
        }
        if self.shift {
            write!(f, "Shift+")?;
        }
        f.write_str(match self.trigger {
            ShortcutTrigger::Key(key) => key.name(),
            ShortcutTrigger::ScrollUp => "ScrollUp",
            ShortcutTrigger::ScrollDown => "ScrollDown",
        })
    }
}

pub(super) fn binding_from_event(
    event: &egui::Event,
    modifiers: egui::Modifiers,
) -> Option<ShortcutBinding> {
    match event {
        egui::Event::Copy => Some(ShortcutBinding::ctrl(egui::Key::C)),
        // On Windows egui-winit emits Cut instead of a key-down event for
        // Shift+Delete. File commands must retain its distinct shortcut identity.
        // Do not rewrite the event itself: text edits still need native Cut semantics.
        egui::Event::Cut if cfg!(target_os = "windows") && modifiers.shift && !modifiers.ctrl => {
            Some(ShortcutBinding::from_modifiers(
                egui::Key::Delete,
                modifiers,
            ))
        }
        egui::Event::Cut => Some(ShortcutBinding::ctrl(egui::Key::X)),
        egui::Event::Paste(_) => Some(ShortcutBinding::ctrl(egui::Key::V)),
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => Some(ShortcutBinding::from_modifiers(*key, *modifiers)),
        egui::Event::MouseWheel {
            delta, modifiers, ..
        } if delta.y != 0.0 => Some(ShortcutBinding::scroll(
            modifiers.ctrl,
            modifiers.alt,
            modifiers.shift,
            delta.y > 0.0,
        )),
        _ => None,
    }
}

pub(super) fn validate_binding_policy(
    binding: ShortcutBinding,
) -> Result<(), ShortcutValidationError> {
    let ShortcutTrigger::Key(key) = binding.trigger else {
        if !binding.ctrl && !binding.alt && !binding.shift {
            return Err(ShortcutValidationError::Unsupported);
        }
        // Reserved for grid thumbnail sizing.
        if binding.ctrl && binding.shift && !binding.alt {
            return Err(ShortcutValidationError::Reserved);
        }
        return Ok(());
    };
    if !is_supported_key(key)
        || (!binding.ctrl && !binding.alt && !binding.shift && !is_allowed_unmodified_key(key))
    {
        return Err(ShortcutValidationError::Unsupported);
    }
    if (binding.alt
        && !binding.ctrl
        && ((!binding.shift && matches!(key, egui::Key::Tab | egui::Key::Space))
            || key == egui::Key::F4))
        || (binding.ctrl && !binding.alt && key == egui::Key::Escape)
        || (binding.ctrl && binding.alt && !binding.shift && key == egui::Key::Delete)
    {
        return Err(ShortcutValidationError::Reserved);
    }
    Ok(())
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
            | egui::Key::Backspace
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
            | egui::Key::PageUp
            | egui::Key::PageDown
            | egui::Key::ArrowLeft
            | egui::Key::ArrowRight
            | egui::Key::ArrowUp
            | egui::Key::ArrowDown
    ) || is_allowed_unmodified_key(key)
}
