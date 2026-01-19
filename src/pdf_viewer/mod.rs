use std::path::PathBuf;

pub mod thread;
pub mod window;
pub mod webview;

pub fn open_pdf_viewer(path: PathBuf) {
    // Fire-and-forget: spawn and return immediately
    thread::spawn(move || {
        // This closure runs in its own dedicated STA thread
        thread::run(path);
    });
}
