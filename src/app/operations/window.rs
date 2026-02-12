//! Window handle management
//!
//! This module captures and stores the native window handle (HWND)
//! and performs initialization tasks that require it.

use crate::app::state::ImageViewerApp;
use crate::infrastructure::windows::native_menu::warmup_shell_extensions;
use crate::infrastructure::windows::window_subclass::install_borderless_subclass;
use windows::core::PCWSTR;
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

impl ImageViewerApp {
    /// Captura e armazena o HWND nativo a partir do título da janela principal.
    /// Na primeira captura, também faz warmup das shell extensions para evitar
    /// lentidão na primeira abertura do menu de contexto.
    ///
    /// # Borderless Window Support
    /// When HWND is obtained, installs a native subclass to handle WM_NCHITTEST
    /// for resize borders on the borderless window.
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

                    // Install borderless subclass for resize borders
                    // This handles WM_NCHITTEST to provide resize zones on window edges
                    if install_borderless_subclass(hwnd) {
                        log::info!("Borderless resize subclass installed successfully");
                    } else {
                        log::warn!("Failed to install borderless resize subclass");
                    }

                    // Warmup shell extensions to avoid first-use delay on context menu
                    // This pre-loads extensions like WinRAR, Send to, etc.
                    warmup_shell_extensions(hwnd);
                }
            }
        }
    }
}
