//! Window handle management
//!
//! This module captures and stores the native window handle (HWND).

use crate::app::state::ImageViewerApp;
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
use windows::core::PCWSTR;

impl ImageViewerApp {
    /// Captura e armazena o HWND nativo a partir do título da janela principal.
    pub fn ensure_window_handle(&mut self, _frame: &eframe::Frame) {
        if self.native_hwnd.is_some() {
            return;
        }

        // Tenta encontrar a janela pelo título
        // Nota: Isso é um hack porque eframe ainda não expõe HWND diretamente de forma safe/fácil no Windows
        // O título deve bater com o definido em main.rs
        let window_title = "MTT File Manager\0".encode_utf16().collect::<Vec<u16>>();
        
        unsafe {
            if let Ok(hwnd) = FindWindowW(None, PCWSTR(window_title.as_ptr())) {
                if !hwnd.is_invalid() {
                    self.native_hwnd = Some(hwnd);
                }
            }
        }
    }
}
