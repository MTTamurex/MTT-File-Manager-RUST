use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SWAPBUTTON};

#[inline]
pub fn is_virtual_key_down(virtual_key: i32) -> bool {
    unsafe { (GetAsyncKeyState(virtual_key) as u16 & 0x8000) != 0 }
}

#[inline]
pub fn is_primary_mouse_button_down() -> bool {
    let buttons_swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
    is_virtual_key_down(primary_mouse_virtual_key(buttons_swapped))
}

#[inline]
fn primary_mouse_virtual_key(buttons_swapped: bool) -> i32 {
    if buttons_swapped {
        VK_RBUTTON.0 as i32
    } else {
        VK_LBUTTON.0 as i32
    }
}

#[cfg(test)]
mod tests {
    use super::primary_mouse_virtual_key;
    use windows::Win32::UI::Input::KeyboardAndMouse::{VK_LBUTTON, VK_RBUTTON};

    #[test]
    fn primary_mouse_key_respects_swapped_buttons() {
        assert_eq!(primary_mouse_virtual_key(false), VK_LBUTTON.0 as i32);
        assert_eq!(primary_mouse_virtual_key(true), VK_RBUTTON.0 as i32);
    }
}
