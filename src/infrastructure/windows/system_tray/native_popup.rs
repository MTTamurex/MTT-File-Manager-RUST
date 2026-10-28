use super::{
    menu::Command,
    menu_icons::MenuIcons,
    menu_text::MenuText,
    model::{menu_label, LocationKind},
    window_ref::WindowRef,
};
use std::mem::size_of;
use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetMenuItemCount, SetMenuItemInfoW, HMENU,
    MENUITEMINFOW, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MIIM_BITMAP,
};

pub(super) struct Menu<'a> {
    pub handle: HMENU,
    text: &'a MenuText,
    icons: &'a MenuIcons,
}

impl<'a> Menu<'a> {
    pub(super) fn new(text: &'a MenuText, icons: &'a MenuIcons) -> Option<Self> {
        unsafe {
            CreatePopupMenu().ok().map(|handle| Self {
                handle,
                text,
                icons,
            })
        }
    }

    fn label(&self, label: &str) -> Vec<u16> {
        menu_label(&self.text.fit(label))
            .encode_utf16()
            .chain(Some(0))
            .collect()
    }

    fn item(&self, id: usize, label: &str) -> bool {
        let label = self.label(label);
        unsafe { AppendMenuW(self.handle, MF_STRING, id, PCWSTR(label.as_ptr())).is_ok() }
    }

    pub(super) fn information(&self, label: &str) {
        let label = self.label(label);
        unsafe {
            let _ = AppendMenuW(
                self.handle,
                MF_STRING | MF_GRAYED,
                0,
                PCWSTR(label.as_ptr()),
            );
        }
    }

    pub(super) fn separator(&self) {
        unsafe {
            let _ = AppendMenuW(self.handle, MF_SEPARATOR, 0, PCWSTR::null());
        }
    }

    fn attach_icon(&self, kind: LocationKind) {
        let Some(bitmap) = self.icons.get(kind) else {
            return;
        };
        let position = unsafe { GetMenuItemCount(Some(self.handle)) } - 1;
        if position < 0 {
            return;
        }
        let info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_BITMAP,
            hbmpItem: bitmap,
            ..Default::default()
        };
        unsafe {
            let _ = SetMenuItemInfoW(self.handle, position as u32, true, &info);
        }
    }

    pub(super) fn submenu(&self, child: Self, label: &str, icon: Option<LocationKind>) {
        let label = self.label(label);
        if unsafe {
            AppendMenuW(
                self.handle,
                MF_POPUP,
                child.handle.0 as usize,
                PCWSTR(label.as_ptr()),
            )
            .is_ok()
        } {
            if let Some(icon) = icon {
                self.attach_icon(icon);
            }
            // The parent owns the attached native menu. Its bitmaps remain
            // borrowed from MenuIcons until the whole hierarchy is destroyed.
            std::mem::forget(child);
        }
    }

    pub(super) fn command(
        &self,
        commands: &mut Vec<Command>,
        command: Command,
        label: &str,
    ) -> bool {
        if !self.item(commands.len() + 1, label) {
            return false;
        }
        commands.push(command);
        true
    }

    pub(super) fn location_command(
        &self,
        commands: &mut Vec<Command>,
        command: Command,
        label: &str,
        icon: LocationKind,
    ) {
        if self.command(commands, command, label) {
            self.attach_icon(icon);
        }
    }

    pub(super) fn window_exit(
        &self,
        commands: &mut Vec<Command>,
        window: WindowRef,
        window_count: usize,
    ) {
        if window_count > 1 {
            self.separator();
            self.command(
                commands,
                Command::Exit(window),
                &rust_i18n::t!("tray.exit_window"),
            );
        }
    }
}

impl Drop for Menu<'_> {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.handle);
        }
    }
}

#[cfg(test)]
mod tests;
