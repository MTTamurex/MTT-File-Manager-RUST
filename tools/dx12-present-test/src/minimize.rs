use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{SW_MINIMIZE, ShowWindow};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

mod preview;

#[derive(Clone, Copy)]
pub enum MinimizeFlow {
    Winit,
    Native,
    Preview,
}

impl MinimizeFlow {
    pub fn from_arg(argument: &str) -> Result<Self, String> {
        match argument {
            "winit" => Ok(Self::Winit),
            "native" => Ok(Self::Native),
            "preview" => Ok(Self::Preview),
            _ => Err(format!("unknown minimize flow: {argument}")),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Winit => "winit",
            Self::Native => "ShowWindow",
            Self::Preview => "GDI + DWM preview + ShowWindow",
        }
    }

    pub fn install(self, window: &Window) -> Result<(), String> {
        if matches!(self, Self::Preview) {
            preview::install(window_hwnd(window)?)
        } else {
            Ok(())
        }
    }

    pub fn minimize(self, window: &Window) {
        if matches!(self, Self::Winit) {
            window.set_minimized(true);
            return;
        }

        let Ok(hwnd) = window_hwnd(window) else {
            eprintln!("could not obtain HWND for minimize");
            return;
        };

        match self {
            Self::Winit => unreachable!(),
            Self::Native => unsafe {
                let _ = ShowWindow(hwnd, SW_MINIMIZE);
            },
            Self::Preview => preview::request(hwnd),
        }
    }
}

fn window_hwnd(window: &Window) -> Result<HWND, String> {
    let handle = window.window_handle().map_err(|error| error.to_string())?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(HWND(handle.hwnd.get() as *mut core::ffi::c_void)),
        _ => Err("window does not have a Win32 handle".to_owned()),
    }
}
