//! Background worker for Windows Shell context menu extraction and invocation.
//!
//! Shell extensions (antivirus, cloud sync) can block `IContextMenu::QueryContextMenu`
//! for 1–10+ seconds. This worker runs in a dedicated STA COM thread so those
//! blocking calls never reach the UI thread.
//!
//! ## Threading model
//! - The worker thread is the ONLY thread that creates or uses `IContextMenu` objects.
//! - `ShellMenuItemData` (the send-safe result type) carries no COM handles.
//! - Command invocation is also sent to this thread — it reuses the stored COM context.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

use crate::infrastructure::windows::native_menu::{
    extract_shell_menu, invoke_menu_command, is_known_verb, warmup_shell_extensions, ShellMenuItem,
};

// ── Public request / response types ────────────────────────────────────────

/// Commands sent to the shell menu worker thread.
pub enum ShellMenuRequest {
    /// Pre-initialize shell extensions on the worker STA thread.
    Warmup { hwnd_isize: isize },
    /// Extract a Shell context menu. The worker replies with `Ready` or `Error`.
    Extract {
        request_id: u64,
        hwnd_isize: isize,
        target: ShellMenuTarget,
    },
    /// Invoke a previously extracted shell command (positive `id` from the menu).
    Invoke {
        request_id: u64,
        command_id: u32,
        menu_x: i32,
        menu_y: i32,
        hwnd_isize: isize,
    },
    /// Discard the active `ShellMenuContext` (menu was dismissed without a command).
    Cancel,
    /// Expand a pending submenu for `item_id` (triggered by hover on a lazy item).
    LoadSubmenu { request_id: u64, item_id: u32 },
}

pub enum ShellMenuTarget {
    Selection(Vec<PathBuf>),
    FolderBackground(PathBuf),
}

/// Send-safe representation of a `ShellMenuItem` — carries no COM handles or OS handles.
#[derive(Clone)]
pub struct ShellMenuItemData {
    pub id: u32,
    pub text: String,
    /// Raw RGBA pixels + dimensions, ready to upload to the GPU.
    pub icon_rgba: Option<(Vec<u8>, u32, u32)>,
    pub sub_items: Vec<ShellMenuItemData>,
    pub is_separator: bool,
    pub is_enabled: bool,
    pub command_string: Option<String>,
    /// True when a submenu exists (but HMENU is not forwarded across threads).
    pub has_submenu: bool,
}

impl ShellMenuItemData {
    fn from_shell_item(item: &ShellMenuItem) -> Self {
        Self {
            id: item.id,
            text: item.text.clone(),
            icon_rgba: item.icon_rgba.clone(),
            sub_items: item.sub_items.iter().map(Self::from_shell_item).collect(),
            is_separator: item.is_separator,
            is_enabled: item.is_enabled,
            command_string: item.command_string.clone(),
            has_submenu: item.pending_submenu_handle.is_some() && item.sub_items.is_empty(),
        }
    }
}

/// Responses sent back from the worker to the UI thread.
pub enum ShellMenuResponse {
    /// Extraction complete; these items can be merged into the context menu.
    Ready {
        request_id: u64,
        items: Vec<ShellMenuItemData>,
    },
    /// Extraction failed (e.g. no shell extensions registered).
    Error { request_id: u64, message: String },
    /// A shell command was invoked (informational only; no result needed).
    Invoked { request_id: u64 },
    /// Submenu for `item_id` was lazily loaded; replace its sub_items in the UI.
    SubmenuLoaded {
        request_id: u64,
        item_id: u32,
        sub_items: Vec<ShellMenuItemData>,
    },
}

// ── Worker startup ──────────────────────────────────────────────────────────

/// Starts the dedicated shell menu STA thread.
/// Returns a `Sender` to send requests and a `Receiver` to collect responses.
pub fn start_shell_menu_worker() -> (Sender<ShellMenuRequest>, Receiver<ShellMenuResponse>) {
    let (req_tx, req_rx) = mpsc::channel::<ShellMenuRequest>();
    let (res_tx, res_rx) = mpsc::channel::<ShellMenuResponse>();

    std::thread::spawn(move || shell_menu_loop(req_rx, res_tx));

    (req_tx, res_rx)
}

// ── Worker loop (runs on its own STA thread) ────────────────────────────────

struct ComGuard;

impl ComGuard {
    fn init_sta() -> Self {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        Self
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() }
    }
}

fn shell_menu_loop(rx: Receiver<ShellMenuRequest>, tx: Sender<ShellMenuResponse>) {
    let _com = ComGuard::init_sta();
    // Active shell context — kept alive between Extract and Invoke/Cancel.
    let mut active_ctx: Option<crate::infrastructure::windows::native_menu::ShellMenuContext> =
        None;
    let mut active_request_id: Option<u64> = None;
    let mut warmup_done = false;

    while let Ok(req) = rx.recv() {
        match req {
            ShellMenuRequest::Warmup { hwnd_isize } => {
                if warmup_done {
                    continue;
                }

                let hwnd = HWND(hwnd_isize as *mut _);
                warmup_shell_extensions(hwnd);
                warmup_done = true;
            }

            ShellMenuRequest::Extract {
                request_id,
                hwnd_isize,
                target,
            } => {
                // Drop any previous context before starting a new extraction.
                active_ctx = None;
                active_request_id = None;

                let hwnd = HWND(hwnd_isize as *mut _);
                let extracted = match target {
                    ShellMenuTarget::Selection(paths) => extract_shell_menu(hwnd, &paths),
                    ShellMenuTarget::FolderBackground(path) => {
                        crate::infrastructure::windows::shell_new::extract_background_menu(
                            hwnd, &path,
                        )
                    }
                };
                match extracted {
                    Ok(ctx) => {
                        let items: Vec<ShellMenuItemData> = ctx
                            .items
                            .borrow()
                            .iter()
                            .filter(|item| {
                                // Filter known verbs so we don't duplicate internal items.
                                if let Some(ref verb) = item.command_string {
                                    if is_known_verb(verb) {
                                        return false;
                                    }
                                }
                                true
                            })
                            .map(ShellMenuItemData::from_shell_item)
                            .collect();

                        active_ctx = Some(ctx);
                        active_request_id = Some(request_id);
                        let _ = tx.send(ShellMenuResponse::Ready { request_id, items });
                    }
                    Err(e) => {
                        let _ = tx.send(ShellMenuResponse::Error {
                            request_id,
                            message: e.to_string(),
                        });
                    }
                }
            }

            ShellMenuRequest::Invoke {
                request_id,
                command_id,
                menu_x,
                menu_y,
                hwnd_isize,
            } => {
                let hwnd = HWND(hwnd_isize as *mut _);
                if let Some(ref ctx) = active_ctx {
                    let _ =
                        invoke_menu_command(hwnd, &ctx.context_menu, command_id, menu_x, menu_y);
                } else {
                    log::warn!("[ShellMenuWorker] Invoke called with no active context");
                }
                // Context is still valid until the menu closes — keep it alive.
                let _ = tx.send(ShellMenuResponse::Invoked { request_id });
            }

            ShellMenuRequest::Cancel => {
                active_ctx = None;
                active_request_id = None;
                // No response needed.
            }

            ShellMenuRequest::LoadSubmenu {
                request_id,
                item_id,
            } => {
                if active_request_id != Some(request_id) {
                    continue;
                }

                if let Some(ref ctx) = active_ctx {
                    fn find_item_mut(
                        items: &mut [crate::infrastructure::windows::native_menu::ShellMenuItem],
                        id: u32,
                    ) -> Option<&mut crate::infrastructure::windows::native_menu::ShellMenuItem>
                    {
                        for item in items.iter_mut() {
                            if item.id == id {
                                return Some(item);
                            }
                            if let Some(found) = find_item_mut(&mut item.sub_items, id) {
                                return Some(found);
                            }
                        }
                        None
                    }

                    let mut items_guard = ctx.items.borrow_mut();
                    if let Some(shell_item) = find_item_mut(&mut items_guard, item_id) {
                        if ctx.load_pending_submenu(shell_item) {
                            let sub_items = shell_item
                                .sub_items
                                .iter()
                                .map(ShellMenuItemData::from_shell_item)
                                .collect();
                            let _ = tx.send(ShellMenuResponse::SubmenuLoaded {
                                request_id,
                                item_id,
                                sub_items,
                            });
                        }
                    }
                } else {
                    log::warn!("[ShellMenuWorker] LoadSubmenu called with no active context");
                }
            }
        }
    }
}
