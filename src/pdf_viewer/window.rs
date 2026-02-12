use crate::pdf_viewer::webview;
use std::path::PathBuf;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

pub fn create_and_run(path: PathBuf, title_prefix: &str) -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class_name_str = "MttPdfViewerClass";
        let mut class_name_u16: Vec<u16> = class_name_str.encode_utf16().collect();
        class_name_u16.push(0);
        let class_name = PCWSTR::from_raw(class_name_u16.as_ptr());

        let wnd_class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as usize as *mut core::ffi::c_void),
            lpszClassName: class_name,
            ..Default::default()
        };

        RegisterClassW(&wnd_class);

        let title = format!(
            "{} - {}",
            title_prefix,
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        let mut title_u16: Vec<u16> = title.encode_utf16().collect();
        title_u16.push(0);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            PCWSTR::from_raw(title_u16.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1024,
            768,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        if hwnd.0.is_null() {
            return Err(Error::from_win32());
        }

        // Initialize WebView
        // We pass the HWND to the webview module to attach the browser
        // Percent-encode characters that have special meaning in URLs but can
        // appear in Windows filenames, to prevent URL parsing issues.
        // '%' must be encoded first to avoid double-encoding.
        let path_str = path.display().to_string().replace('\\', "/");
        let url = format!(
            "file:///{}",
            path_str
                .replace('%', "%25")
                .replace(' ', "%20")
                .replace('#', "%23")
                .replace('?', "%3F")
        );
        if let Err(e) = webview::init(hwnd, url) {
            eprintln!("Failed to init WebView2: {}", e);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
    }
    Ok(())
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_SIZE => {
            // Resize logic will be called here
            webview::resize(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            webview::close(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
