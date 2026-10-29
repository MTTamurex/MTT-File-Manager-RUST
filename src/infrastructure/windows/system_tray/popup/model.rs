use super::super::{
    menu::Command,
    model::{LocationKind, Snapshot, Target},
    window_ref::WindowRef,
};
use std::collections::HashSet;

pub(super) enum Action {
    Command(Command),
    Information,
}

pub(super) struct Row {
    pub title: String,
    pub subtitle: String,
    pub sent_at: Option<String>,
    pub icon: Option<LocationKind>,
    pub indent: i32,
    pub height: i32,
    pub expanded: Option<bool>,
    pub action: Action,
}

impl Row {
    pub fn instance(&self) -> Option<WindowRef> {
        match self.action {
            Action::Command(Command::Open(window)) => Some(window),
            _ => None,
        }
    }

    pub fn selectable(&self) -> bool {
        !matches!(self.action, Action::Information)
    }
}

pub(super) struct Model {
    pub windows: Vec<(WindowRef, Snapshot)>,
    pub expanded: HashSet<usize>,
    pub rows: Vec<Row>,
    pub selected: Option<usize>,
    pub scroll: i32,
}

impl Model {
    pub fn new(windows: Vec<(WindowRef, Snapshot)>) -> Self {
        let mut model = Self {
            windows,
            expanded: HashSet::new(),
            rows: Vec::new(),
            selected: None,
            scroll: 0,
        };
        model.rebuild();
        model
    }

    pub(super) fn rebuild(&mut self) {
        self.rows.clear();
        for (index, (window, snapshot)) in self.windows.iter().enumerate() {
            let expandable = snapshot.workspaces.len() > 1
                || snapshot
                    .workspaces
                    .iter()
                    .any(|workspace| workspace.panels.is_some());
            let expanded = expandable && self.expanded.contains(&(window.hwnd.0 as usize));
            self.rows.push(Row {
                title: format!(
                    "{} {}",
                    rust_i18n::t!("tray.window"),
                    crate::infrastructure::windows::app_instances::instance_number(window.hwnd)
                        .unwrap_or(index + 1)
                ),
                subtitle: snapshot.active_path.clone(),
                sent_at: snapshot.hidden_at.clone(),
                icon: Some(snapshot.active_icon),
                indent: 0,
                height: 52,
                expanded: expandable.then_some(expanded),
                action: Action::Command(Command::Open(*window)),
            });
            if !expanded {
                continue;
            }
            for workspace in &snapshot.workspaces {
                // Split workspaces expose their panel tabs directly. Their outer
                // title mirrors the focused panel and would duplicate that entry.
                if workspace.panels.is_none() && snapshot.workspaces.len() > 1 {
                    self.rows.push(Row {
                        title: workspace.tab.path.clone(),
                        subtitle: String::new(),
                        sent_at: None,
                        icon: Some(workspace.tab.icon),
                        indent: 1,
                        height: 30,
                        expanded: None,
                        action: Action::Command(Command::Activate(
                            *window,
                            Target {
                                workspace: workspace.tab.id,
                                panel: None,
                            },
                        )),
                    });
                }
                if let Some(panels) = &workspace.panels {
                    for (right, entries) in panels.iter().enumerate() {
                        self.rows.push(Row {
                            title: if right == 0 {
                                rust_i18n::t!("tray.left_panel")
                            } else {
                                rust_i18n::t!("tray.right_panel")
                            }
                            .into_owned(),
                            subtitle: String::new(),
                            sent_at: None,
                            icon: None,
                            indent: 1,
                            height: 22,
                            expanded: None,
                            action: Action::Information,
                        });
                        for entry in entries {
                            self.rows.push(Row {
                                title: entry.path.clone(),
                                subtitle: String::new(),
                                sent_at: None,
                                icon: Some(entry.icon),
                                indent: 1,
                                height: 28,
                                expanded: None,
                                action: Action::Command(Command::Activate(
                                    *window,
                                    Target {
                                        workspace: workspace.tab.id,
                                        panel: Some((right == 1, entry.id)),
                                    },
                                )),
                            });
                        }
                    }
                }
            }
        }
    }

    pub fn toggle(&mut self, index: usize) {
        let Some(Row {
            action: Action::Command(Command::Open(window)),
            expanded: Some(_),
            ..
        }) = self.rows.get(index)
        else {
            return;
        };
        let key = window.hwnd.0 as usize;
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
        self.rebuild();
        self.selected = Some(index.min(self.rows.len().saturating_sub(1)));
    }

    pub(super) fn remove_window(&mut self, closed: WindowRef) {
        self.windows
            .retain(|(window, _)| *window != closed && window.is_current());
        self.expanded.retain(|id| {
            self.windows
                .iter()
                .any(|(window, _)| window.hwnd.0 as usize == *id)
        });
        self.selected = None;
        self.rebuild();
    }

    pub fn navigate(&mut self, direction: i32) {
        let mut selectable: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.selectable())
            .map(|(index, _)| index)
            .collect();
        // Both global commands remain selectable in the pinned footer.
        selectable.extend([self.rows.len(), self.rows.len() + 1]);
        let position = self
            .selected
            .and_then(|index| selectable.iter().position(|&row| row == index));
        let next = match position {
            Some(position) => {
                (position as i32 + direction).rem_euclid(selectable.len() as i32) as usize
            }
            None if direction < 0 => selectable.len() - 1,
            None => 0,
        };
        self.selected = Some(selectable[next]);
    }

    pub fn content_height(&self) -> i32 {
        self.rows.iter().map(|row| row.height).sum()
    }

    pub fn row_at(&self, y: i32) -> Option<usize> {
        if y < 0 {
            return None;
        }
        let mut bottom = 0;
        self.rows.iter().position(|row| {
            bottom += row.height;
            y < bottom
        })
    }

    pub fn reveal_selection(&mut self, viewport: i32) {
        if let Some(index) = self.selected.filter(|&index| index < self.rows.len()) {
            let top: i32 = self.rows[..index].iter().map(|row| row.height).sum();
            let bottom = top + self.rows[index].height;
            if top < self.scroll || self.rows[index].height >= viewport {
                self.scroll = top;
            } else if bottom > self.scroll + viewport {
                self.scroll = bottom - viewport;
            }
        }
        self.scroll = self
            .scroll
            .clamp(0, (self.content_height() - viewport).max(0));
    }
}

#[cfg(test)]
mod tests;
