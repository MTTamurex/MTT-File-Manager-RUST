use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

#[inline]
pub fn is_virtual_key_down(virtual_key: i32) -> bool {
    unsafe { (GetAsyncKeyState(virtual_key) as u16 & 0x8000) != 0 }
}
