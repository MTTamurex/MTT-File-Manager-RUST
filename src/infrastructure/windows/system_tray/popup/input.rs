use super::super::menu::Command;
use super::{
    layout::{FOOTER, HEADER},
    model::Action,
    State,
};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::Input::KeyboardAndMouse::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    Body,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Hit {
    pub index: usize,
    pub part: Part,
}

impl State {
    pub(super) fn pointer_move(&mut self, position: POINT) {
        let hit = self.mouse_hit(position);
        self.model.selected = hit.map(|hit| hit.index);
        self.hot_close = hit
            .filter(|hit| hit.part == Part::Close)
            .map(|hit| hit.index);
    }

    pub(super) fn mouse_hit(&self, position: POINT) -> Option<Hit> {
        let index = self.hit(position)?;
        let close = position.x >= self.layout.width - 36
            && self
                .model
                .rows
                .get(index)
                .is_some_and(|row| row.instance().is_some());
        Some(Hit {
            index,
            part: if close { Part::Close } else { Part::Body },
        })
    }

    pub(super) fn click(&mut self, hit: Hit) {
        match hit.part {
            Part::Body => self.activate(
                hit.index,
                self.model
                    .rows
                    .get(hit.index)
                    .is_some_and(|row| row.expanded.is_some()),
            ),
            Part::Close => self.close_instance(hit.index),
        }
    }

    fn close_instance(&mut self, index: usize) {
        if let Some(window) = self.model.rows.get(index).and_then(|row| row.instance()) {
            self.command = Some(Command::Exit(window));
            self.done = true;
        }
    }

    pub(super) fn hit(&self, point: POINT) -> Option<usize> {
        if point.x < 0
            || point.x >= self.layout.width
            || point.y < HEADER
            || point.y >= self.layout.height
        {
            return None;
        }
        if point.y >= self.layout.height - FOOTER {
            return Some(self.model.rows.len());
        }
        self.model
            .row_at(point.y - HEADER + self.model.scroll)
            .filter(|&index| self.model.rows[index].selectable())
    }

    pub(super) fn activate(&mut self, index: usize, expand: bool) {
        if index == self.model.rows.len() {
            self.command = Some(Command::ExitAll);
            self.done = true;
        } else if expand
            && self
                .model
                .rows
                .get(index)
                .is_some_and(|row| row.expanded.is_some())
        {
            self.model.toggle(index);
            self.resize = true;
        } else if let Some(Action::Command(command)) =
            self.model.rows.get(index).map(|row| &row.action)
        {
            self.command = Some(command.clone());
            self.done = true;
        }
    }

    pub(super) fn key(&mut self, key: VIRTUAL_KEY) {
        self.pressed = None;
        self.hot_close = None;
        match key {
            VK_ESCAPE => self.done = true,
            VK_UP | VK_DOWN | VK_TAB => {
                let backwards = key == VK_UP
                    || (key == VK_TAB && unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0);
                self.model.navigate(if backwards { -1 } else { 1 });
            }
            VK_HOME => {
                self.model.selected = None;
                self.model.navigate(1);
            }
            VK_END => self.model.selected = Some(self.model.rows.len()),
            VK_DELETE => {
                if let Some(index) = self.model.selected {
                    self.close_instance(index);
                }
            }
            VK_RETURN | VK_SPACE => {
                if let Some(index) = self.model.selected {
                    self.click(Hit {
                        index,
                        part: Part::Body,
                    });
                }
            }
            VK_LEFT | VK_RIGHT => {
                if let Some(index) = self.model.selected {
                    if self.model.rows.get(index).and_then(|row| row.expanded)
                        == Some(key == VK_LEFT)
                    {
                        self.activate(index, true);
                    } else if key == VK_LEFT
                        && index < self.model.rows.len()
                        && self.model.rows[index].indent > 0
                    {
                        if let Some(parent) = self.model.rows[..index]
                            .iter()
                            .rposition(|row| row.expanded == Some(true))
                        {
                            self.activate(parent, true);
                        }
                    }
                }
            }
            _ => {}
        }
        self.model.reveal_selection(self.layout.viewport());
    }

    pub(super) fn wheel(&mut self, delta: i32) {
        self.pressed = None;
        self.hot_close = None;
        self.model.scroll = (self.model.scroll - delta * 48 / 120).clamp(
            0,
            (self.model.content_height() - self.layout.viewport()).max(0),
        );
        self.model.selected = None;
    }
}
