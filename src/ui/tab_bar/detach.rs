//! Native cursor sampling keeps release detection working outside the viewport.
use eframe::egui;
use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetWindowRect};

#[derive(Clone, Copy)]
pub(crate) struct NativePointer {
    pub outside: bool,
    pub down: bool,
    pub screen_px: egui::Pos2,
    pub owner: Option<isize>,
}

pub(crate) fn sample_pointer(ctx: &egui::Context, hwnd: Option<HWND>) {
    let id = egui::Id::new("tab_detach_native_pointer");
    ctx.data_mut(|data| data.remove::<NativePointer>(id));
    if let Some(pointer) = hwnd.and_then(sample_native_pointer) {
        ctx.data_mut(|data| data.insert_temp(id, pointer));
    }
}

pub(crate) fn sample_native_pointer(hwnd: HWND) -> Option<NativePointer> {
    let mut cursor = POINT::default();
    let mut rect = RECT::default();
    if unsafe { GetCursorPos(&mut cursor).is_ok() && GetWindowRect(hwnd, &mut rect).is_ok() } {
        Some(NativePointer {
            outside: cursor.x < rect.left
                || cursor.x >= rect.right
                || cursor.y < rect.top
                || cursor.y >= rect.bottom,
            down: unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0,
            screen_px: egui::pos2(cursor.x as f32, cursor.y as f32),
            owner: Some(hwnd.0 as isize),
        })
    } else {
        None
    }
}

#[cfg(test)]
pub(crate) fn set_test_pointer(ctx: &egui::Context, outside: bool, down: bool) {
    set_test_pointer_at(ctx, outside, down, egui::Pos2::ZERO);
}

#[cfg(test)]
pub(crate) fn set_test_pointer_at(
    ctx: &egui::Context,
    outside: bool,
    down: bool,
    screen_px: egui::Pos2,
) {
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new("tab_detach_native_pointer"),
            NativePointer {
                outside,
                down,
                screen_px,
                owner: None,
            },
        )
    });
}

pub(crate) fn pointer(ctx: &egui::Context) -> Option<NativePointer> {
    let viewport = ctx.viewport_rect();
    let scale = ctx.pixels_per_point();
    ctx.data(|data| data.get_temp::<NativePointer>(egui::Id::new("tab_detach_native_pointer")))
        .or_else(|| {
            ctx.input(|input| {
                let pos = input.pointer.interact_pos()?;
                let origin = input.viewport().inner_rect?.min;
                Some(NativePointer {
                    outside: !viewport.contains(pos),
                    down: input.pointer.primary_down(),
                    screen_px: (origin + pos.to_vec2()) * scale,
                    owner: None,
                })
            })
        })
}

pub(crate) fn released_outside(ctx: &egui::Context) -> bool {
    if ctx.input(|input| input.key_pressed(egui::Key::Escape) || !input.focused) {
        return false;
    }
    if let Some(pointer) =
        ctx.data(|data| data.get_temp::<NativePointer>(egui::Id::new("tab_detach_native_pointer")))
    {
        return pointer.outside && !pointer.down;
    }
    let viewport = ctx.viewport_rect();
    ctx.input(|input| {
        input.pointer.primary_released()
            && input
                .pointer
                .interact_pos()
                .is_some_and(|pos| !viewport.contains(pos))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_release_requires_both_outside_position_and_button_up() {
        let ctx = egui::Context::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            for (outside, down, expected) in [
                (false, false, false),
                (false, true, false),
                (true, true, false),
                (true, false, true),
            ] {
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new("tab_detach_native_pointer"),
                        NativePointer {
                            outside,
                            down,
                            screen_px: egui::Pos2::ZERO,
                            owner: None,
                        },
                    )
                });
                assert_eq!(released_outside(ui.ctx()), expected);
            }
        });
    }
}
