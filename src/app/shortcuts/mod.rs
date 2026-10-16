use eframe::egui;
use std::collections::HashMap;

mod action;
mod binding;
pub use action::ShortcutAction;
use action::DEFAULT_BINDINGS;
use binding::{binding_from_event, validate_binding_policy};
pub use binding::{ShortcutBinding, ShortcutTrigger};

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
            let Some(candidate) = prefs
                .get(action.preference_key())
                .and_then(|raw| ShortcutBinding::parse(raw))
            else {
                continue;
            };
            if validate_action_binding(action, candidate).is_err() {
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
                let duplicates: Vec<_> = ShortcutAction::ALL
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
                    .find(|other| !customized[other.index()] && other.default_binding() == binding)
                    .unwrap_or(duplicates[0]);
                for other in duplicates {
                    if other != winner && bindings[other.index()] != other.default_binding() {
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
    pub fn reset(&mut self, action: ShortcutAction) -> Result<(), ShortcutValidationError> {
        self.validate_candidate(action, action.default_binding())?;
        self.bindings[action.index()] = action.default_binding();
        Ok(())
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
        validate_action_binding(action, binding)?;
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
        ShortcutAction::ALL
            .iter()
            .copied()
            .find(|other| *other != action && self.bindings[other.index()] == binding)
    }
    pub fn append_preferences(&self, prefs: &mut Vec<(&'static str, String)>) {
        for action in ShortcutAction::CONFIGURABLE {
            prefs.push((action.preference_key(), self.get(action).serialize()));
        }
    }
    pub fn is_triggered(&self, action: ShortcutAction, ctx: &egui::Context) -> bool {
        let binding = self.get(action);
        ctx.input(|i| {
            i.events
                .iter()
                .any(|event| binding_from_event(event) == Some(binding))
        })
    }
    pub fn consume_triggered(&self, action: ShortcutAction, ctx: &egui::Context) -> bool {
        let binding = self.get(action);
        ctx.input_mut(|input| {
            let mut consumed = false;
            input.events.retain(|event| {
                let matches = binding_from_event(event) == Some(binding);
                consumed |= matches;
                !matches
            });
            if consumed && binding.is_scroll() {
                clear_scroll(input);
            }
            consumed
        })
    }
}

// egui computes scrolling before application event consumption. Suppress the
// visible delta too so a consumed shortcut cannot also scroll the file view.
pub(crate) fn clear_scroll(input: &mut egui::InputState) {
    input.smooth_scroll_delta = egui::Vec2::ZERO;
}

fn validate_action_binding(
    action: ShortcutAction,
    binding: ShortcutBinding,
) -> Result<(), ShortcutValidationError> {
    if binding.is_scroll()
        && !matches!(
            action,
            ShortcutAction::NextViewMode | ShortcutAction::PreviousViewMode
        )
    {
        return Err(ShortcutValidationError::Unsupported);
    }
    validate_binding_policy(binding)
}

pub fn capture_shortcut(ctx: &egui::Context) -> Option<ShortcutCapture> {
    let capture = ctx.input(|i| {
        for event in &i.events {
            if matches!(event, egui::Event::Key { key: egui::Key::Escape, pressed: true, modifiers, .. }
                if !modifiers.ctrl && !modifiers.alt && !modifiers.shift)
            {
                return Some(ShortcutCapture::Cancelled);
            }
            if let Some(binding) = binding_from_event(event) { return Some(ShortcutCapture::Binding(binding)); }
        }
        None
    });
    if let Some(ShortcutCapture::Binding(binding)) = capture {
        if binding.is_scroll() {
            ctx.input_mut(|input| {
                input
                    .events
                    .retain(|event| binding_from_event(event) != Some(binding));
                clear_scroll(input);
            });
        }
    }
    capture
}

#[cfg(test)]
mod tests;
