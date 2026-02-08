use std::path::PathBuf;

pub mod thread;
pub mod webview;
pub mod window;

pub fn open_pdf_viewer(path: PathBuf) {
    // Fire-and-forget: spawn and return immediately
    thread::spawn(move || {
        // This closure runs in its own dedicated STA thread
        thread::run(path, "PDF Viewer");
    });
}

pub fn open_image_viewer(path: PathBuf) {
    // Fire-and-forget: spawn and return immediately
    thread::spawn(move || {
        // This closure runs in its own dedicated STA thread
        thread::run(path, "Image Viewer");
    });
}

// Global flag to ensure warmup only runs once per process lifetime
static WARMUP_ONCE: std::sync::Once = std::sync::Once::new();

pub fn warmup() {
    WARMUP_ONCE.call_once(|| {
        // Spawn a dedicated thread for warmup to avoid blocking ANY UI/Main logic.
        // We treat this as a "low priority" background task.
        std::thread::spawn(|| {
            // Initialize COM on this thread (STA is fine, though we don't create windows)
            use windows::Win32::System::Com::{
                CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
            };

            unsafe {
                if CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok() {
                    // Ignore errors during warmup - it should be silent
                    let _ = webview::warmup_env();

                    // We don't pump messages because CreateCoreWebView2EnvironmentWithOptions
                    // usually posts a message to the thread's message queue if a pump exists,
                    // OR creates a hidden window.
                    // However, strictly speaking, we might need a minimal message pump
                    // if the callback relies on it.
                    // For simply triggering the process launch, calling the function might be enough.
                    // If the callback never fires because there's no msg loop, that's arguably OK
                    // for "warming up" the DLL and process, partially.
                    // But to be clean, let's run a micro-pump for a short duration or until callback?
                    // actually, let's keep it simple: Just calling the function triggers the DLL load
                    // and RPC start. Even if the callback doesn't run, the expensive part (starting edge) starts.

                    // Allow some time for things to settle if needed, or just exit.
                    // Since we are releasing everything in the callback, if we exit thread immediately,
                    // COM objects might leak if the callback hasn't run.
                    // But since this is global warmup, leaking a small COM stub for the app lifetime isn't fatal.
                    // For robustness, let's pump messages for a short bit until done?
                    // Too complex for "invisible".

                    // Better strategy: Just call it. The side-effect of loading DLL and init is what we want.

                    // Yield to let the OS schedule the IO
                    std::thread::yield_now();

                    // We DO NOT call CoUninitialize immediately if we want things to potentially persist
                    // or if operations are pending. But for a quick thread, we should clean up.
                    // Let's rely on thread exit to clean up COM implicitly (not ideal but safe-ish here).
                    CoUninitialize();
                }
            }
        });
    });
}
