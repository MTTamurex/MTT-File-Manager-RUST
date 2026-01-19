use std::path::PathBuf;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

pub fn spawn<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    std::thread::spawn(f);
}

pub fn run(path: PathBuf) {
    unsafe {
        // 1. Initialize COM as STA (Single Threaded Apartment) - Critical for UI/WebView2
        let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if hr.is_err() {
            eprintln!("PDF Viewer: Failed to initialize COM: {:?}", hr);
            return;
        }

        // 2. Run the window message loop
        if let Err(e) = crate::pdf_viewer::window::create_and_run(path) {
            eprintln!("PDF Viewer: Window error: {}", e);
        }

        // 3. Cleanup
        CoUninitialize();
    }
}
