//! Hidden native windows verify notification ordering without requiring a GPU.

use std::time::{Duration, Instant};

use rwh_06::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetWindowLongW, KillTimer, PostMessageW, SendMessageW, SetTimer, SetWindowPos,
    GWL_EXSTYLE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER, WM_CLOSE, WM_ENTERSIZEMOVE,
    WM_EXITSIZEMOVE, WM_SIZE, WS_EX_NOREDIRECTIONBITMAP,
};

use crate::application::ApplicationHandler;
use crate::dpi::{PhysicalPosition, PhysicalSize};
use crate::event::WindowEvent;
use crate::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use crate::platform::run_on_demand::EventLoopExtRunOnDemand;
use crate::platform::windows::{
    with_direct_composition_window, EventLoopBuilderExtWindows, WindowExtWindows,
};
use crate::window::{Window, WindowId};

unsafe extern "system" fn resize_timer(hwnd: HWND, _: u32, id: usize, _: u32) {
    unsafe {
        KillTimer(hwnd, id);
        SendMessageW(hwnd, WM_ENTERSIZEMOVE, 0, 0);
        SetWindowPos(
            hwnd,
            0,
            0,
            0,
            640,
            480,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER,
        );
        SetWindowPos(
            hwnd,
            0,
            0,
            0,
            480,
            320,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOZORDER,
        );
        SetWindowPos(hwnd, 0, 50, 50, 500, 360, SWP_NOACTIVATE | SWP_NOZORDER);
        SendMessageW(hwnd, WM_EXITSIZEMOVE, 0, 0);
        PostMessageW(hwnd, WM_CLOSE, 0, 0);
    }
}

fn hwnd(window: &Window) -> HWND {
    match window.window_handle().unwrap().as_raw() {
        RawWindowHandle::Win32(handle) => handle.hwnd.get(),
        _ => unreachable!(),
    }
}

struct Fixture {
    windows: Vec<(Window, bool)>,
    early_opted_in: usize,
    early_disabled: usize,
    inconsistent_sizes: usize,
    buffered_injected: bool,
    last_prospective: Option<(PhysicalSize<u32>, PhysicalPosition<i32>)>,
    closed: usize,
    started: Instant,
}

impl ApplicationHandler for Fixture {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // A normal window created after the composition scope also proves
        // that failed-attempt/fallback creation cannot inherit its style.
        for enabled in [false, true, false] {
            let create = || {
                event_loop
                    .create_window(
                        Window::default_attributes()
                            .with_visible(false)
                            .with_inner_size(PhysicalSize::new(320, 240)),
                    )
                    .unwrap()
            };
            let window = if enabled {
                with_direct_composition_window(create)
            } else {
                create()
            };
            assert_eq!(
                unsafe { GetWindowLongW(hwnd(&window), GWL_EXSTYLE) } as u32
                    & WS_EX_NOREDIRECTIONBITMAP
                    != 0,
                enabled
            );
            window.set_present_before_resize(enabled);
            assert_ne!(
                unsafe { SetTimer(hwnd(&window), 1, 10, Some(resize_timer)) },
                0
            );
            self.windows.push((window, enabled));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some((window, enabled)) = self.windows.iter().find(|(window, _)| window.id() == id)
        else {
            return;
        };
        match event {
            WindowEvent::Resized(size) => {
                let mut native: RECT = unsafe { std::mem::zeroed() };
                assert_ne!(unsafe { GetClientRect(hwnd(window), &mut native) }, 0);
                let actual = PhysicalSize::new(
                    (native.right - native.left) as u32,
                    (native.bottom - native.top) as u32,
                );
                if actual != size {
                    if *enabled {
                        self.early_opted_in += 1;
                        self.last_prospective = Some((size, window.inner_position().unwrap()));
                    } else {
                        self.early_disabled += 1;
                    }
                    if window.inner_size() != size {
                        self.inconsistent_sizes += 1;
                    }
                    if *enabled && !self.buffered_injected {
                        self.buffered_injected = true;
                        // Nested events must be buffered until prospective
                        // geometry is cleared after the early frame callback.
                        let packed = (actual.width | (actual.height << 16)) as isize;
                        unsafe {
                            SendMessageW(hwnd(window), WM_SIZE, 0, packed);
                        }
                    }
                } else if window.inner_size() != size {
                    self.inconsistent_sizes += 1;
                }
            }
            WindowEvent::CloseRequested => {
                self.closed += 1;
                if self.closed == 3 {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let deadline = self.started + Duration::from_secs(5);
        if Instant::now() >= deadline {
            event_loop.exit();
        } else {
            event_loop.set_control_flow(ControlFlow::WaitUntil(deadline));
        }
    }
}

#[test]
fn early_callback_precedes_native_resize_only_for_opted_in_windows() {
    let mut event_loop = EventLoop::builder().with_any_thread(true).build().unwrap();
    let mut fixture = Fixture {
        windows: Vec::new(),
        early_opted_in: 0,
        early_disabled: 0,
        inconsistent_sizes: 0,
        buffered_injected: false,
        last_prospective: None,
        closed: 0,
        started: Instant::now(),
    };
    event_loop.run_app_on_demand(&mut fixture).unwrap();
    assert_eq!(fixture.closed, 3, "native resize test timed out");
    assert!(
        fixture.early_opted_in >= 3,
        "no frame opportunity before native resize"
    );
    assert_eq!(fixture.early_disabled, 0);
    assert_eq!(fixture.inconsistent_sizes, 0);
    assert!(fixture.buffered_injected);
    let (window, _) = fixture
        .windows
        .iter()
        .find(|(_, enabled)| *enabled)
        .unwrap();
    assert_eq!(
        fixture.last_prospective,
        Some((window.inner_size(), window.inner_position().unwrap()))
    );
}
