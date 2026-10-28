use super::*;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::WindowsAndMessaging::{GetMenuItemID, GetMenuItemInfoW};

#[test]
fn native_location_icons_do_not_change_commands_or_submenu_ownership() {
    let text = MenuText::new(HWND::default(), POINT::default());
    let icons = MenuIcons::new(text.icon_size());
    let root = Menu::new(&text, &icons).unwrap();
    let child = Menu::new(&text, &icons).unwrap();
    let mut commands = Vec::new();
    child.location_command(
        &mut commands,
        Command::ExitAll,
        "Example",
        LocationKind::Tag,
    );
    assert_eq!(commands.len(), 1);
    assert_eq!(unsafe { GetMenuItemID(child.handle, 0) }, 1);
    let bitmap = |menu, position| {
        let mut info = MENUITEMINFOW {
            cbSize: size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_BITMAP,
            ..Default::default()
        };
        unsafe {
            GetMenuItemInfoW(menu, position, true, &mut info).unwrap();
        }
        info.hbmpItem
    };
    for kind in LocationKind::ALL {
        assert!(icons.get(kind).is_some());
    }
    assert_eq!(
        bitmap(child.handle, 0),
        icons.get(LocationKind::Tag).unwrap()
    );
    root.submenu(child, "C:\\Documents", Some(LocationKind::Folder));
    assert_eq!(
        bitmap(root.handle, 0),
        icons.get(LocationKind::Folder).unwrap()
    );
    assert_eq!(unsafe { GetMenuItemCount(Some(root.handle)) }, 1);
}
