use crate::infrastructure::windows::com_scope::ComScope;
use crate::infrastructure::windows::shell_new::extract_background_menu;
use std::path::Path;

#[test]
#[ignore = "requires the installed Zed Shell extension; run alone in a fresh test process"]
fn zed_is_present_on_first_background_extraction() {
    let com = ComScope::sta();
    assert!(com.is_initialized());
    let hwnd = unsafe { windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow() };
    let mut found = Vec::new();
    for opening in 1..=2 {
        let menu = extract_background_menu(hwnd, Path::new(r"C:\")).unwrap();
        let items = menu.items.borrow();
        let labels: Vec<_> = items.iter().map(|item| item.text.as_str()).collect();
        eprintln!("Opening {opening}: {labels:?}");
        found.push(items.iter().any(|item| item.text == "Open with Zed"));
    }
    assert!(
        found[0],
        "Zed absent on first extraction; results: {found:?}"
    );
    assert!(found[1], "Zed absent on second extraction");
}
