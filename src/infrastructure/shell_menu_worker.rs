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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::Arc;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_QUIT,
};

use crate::infrastructure::prioritized_receiver::{receive_prioritized, PrioritizedReceive};
use crate::infrastructure::windows::native_menu::{
    extract_shell_menu, invoke_menu_command, is_known_verb, ShellMenuItem,
};

// ── Public request / response types ────────────────────────────────────────

/// Commands sent to the shell menu worker thread.
pub enum ShellMenuRequest {
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
pub struct ShellMenuWorkerChannels {
    pub request_tx: SyncSender<ShellMenuRequest>,
    pub control_tx: SyncSender<ShellMenuRequest>,
    pub response_rx: Receiver<ShellMenuResponse>,
    pub latest_request_id: Arc<AtomicU64>,
    pub pending_invocation_id: Arc<AtomicU64>,
}

pub fn start_shell_menu_worker(repaint_ctx: eframe::egui::Context) -> ShellMenuWorkerChannels {
    let (req_tx, req_rx) = mpsc::sync_channel::<ShellMenuRequest>(64);
    let (control_tx, control_rx) = mpsc::sync_channel::<ShellMenuRequest>(4);
    let (res_tx, res_rx) = mpsc::channel::<ShellMenuResponse>();
    let latest_request_id = Arc::new(AtomicU64::new(0));
    let pending_invocation_id = Arc::new(AtomicU64::new(0));
    let worker_latest_request_id = Arc::clone(&latest_request_id);
    let worker_pending_invocation_id = Arc::clone(&pending_invocation_id);

    std::thread::spawn(move || {
        shell_menu_loop(
            req_rx,
            control_rx,
            res_tx,
            repaint_ctx,
            worker_latest_request_id,
            worker_pending_invocation_id,
        )
    });

    ShellMenuWorkerChannels {
        request_tx: req_tx,
        control_tx,
        response_rx: res_rx,
        latest_request_id,
        pending_invocation_id,
    }
}

// ── Worker loop (runs on its own STA thread) ────────────────────────────────

struct ComGuard;

impl ComGuard {
    fn init_sta() -> Result<Self, String> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|error| error.to_string())?;
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn send_response(
    tx: &Sender<ShellMenuResponse>,
    repaint_ctx: &eframe::egui::Context,
    response: ShellMenuResponse,
) {
    if tx.send(response).is_ok() {
        repaint_ctx.request_repaint();
    }
}

fn shell_menu_loop(
    rx: Receiver<ShellMenuRequest>,
    control_rx: Receiver<ShellMenuRequest>,
    tx: Sender<ShellMenuResponse>,
    repaint_ctx: eframe::egui::Context,
    latest_request_id: Arc<AtomicU64>,
    pending_invocation_id: Arc<AtomicU64>,
) {
    let _com = match ComGuard::init_sta() {
        Ok(com) => com,
        Err(error) => {
            log::error!("[ShellMenuWorker] Failed to initialize STA COM: {}", error);
            shell_menu_com_failure_loop(rx, control_rx, tx, repaint_ctx, error);
            return;
        }
    };
    // Active shell context — kept alive between Extract and Invoke/Cancel.
    let mut active_ctx: Option<crate::infrastructure::windows::native_menu::ShellMenuContext> =
        None;
    let mut active_request_id: Option<u64> = None;
    let mut deferred_request = None;

    loop {
        if active_request_id.is_some_and(|request_id| {
            latest_request_id.load(Ordering::Acquire) != request_id
                && pending_invocation_id.load(Ordering::Acquire) != request_id
        }) {
            active_ctx = None;
            active_request_id = None;
        }
        if !pump_sta_messages() {
            break;
        }
        let req = match receive_prioritized(&rx, &control_rx, &mut deferred_request) {
            PrioritizedReceive::Request(request) => request,
            PrioritizedReceive::Timeout => continue,
            PrioritizedReceive::Disconnected => break,
        };
        match req {
            ShellMenuRequest::Extract {
                request_id,
                hwnd_isize,
                target,
            } => {
                // A newer right-click may have superseded this request while it waited
                // behind another Shell extension call. Do not start obsolete COM work.
                if latest_request_id.load(Ordering::Acquire) != request_id {
                    continue;
                }

                // Drop any previous context before starting a new extraction.
                active_ctx = None;
                active_request_id = None;

                let hwnd = HWND(hwnd_isize as *mut _);
                let started = std::time::Instant::now();
                let extracted = match target {
                    ShellMenuTarget::Selection(paths) => extract_shell_menu(hwnd, &paths),
                    ShellMenuTarget::FolderBackground(path) => {
                        crate::infrastructure::windows::shell_new::extract_background_menu(
                            hwnd, &path,
                        )
                    }
                };
                log::debug!(
                    "[ShellMenuWorker] Extraction request {} completed in {:.1}ms",
                    request_id,
                    started.elapsed().as_secs_f64() * 1000.0
                );
                match extracted {
                    Ok(ctx) => {
                        if latest_request_id.load(Ordering::Acquire) != request_id {
                            continue;
                        }
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
                                !crate::infrastructure::windows::native_menu::is_filtered_shell_text(
                                    &item.text,
                                )
                            })
                            .map(ShellMenuItemData::from_shell_item)
                            .collect();

                        active_ctx = Some(ctx);
                        active_request_id = Some(request_id);
                        send_response(
                            &tx,
                            &repaint_ctx,
                            ShellMenuResponse::Ready { request_id, items },
                        );
                    }
                    Err(e) => {
                        if latest_request_id.load(Ordering::Acquire) != request_id {
                            continue;
                        }
                        send_response(
                            &tx,
                            &repaint_ctx,
                            ShellMenuResponse::Error {
                                request_id,
                                message: e.to_string(),
                            },
                        );
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
                if active_request_id == Some(request_id) {
                    let Some(ref ctx) = active_ctx else {
                        log::warn!("[ShellMenuWorker] Invoke called with no active context");
                        pending_invocation_id
                            .compare_exchange(request_id, 0, Ordering::AcqRel, Ordering::Acquire)
                            .ok();
                        send_response(&tx, &repaint_ctx, ShellMenuResponse::Invoked { request_id });
                        continue;
                    };
                    let _ =
                        invoke_menu_command(hwnd, &ctx.context_menu, command_id, menu_x, menu_y);
                } else {
                    log::warn!(
                        "[ShellMenuWorker] Invoke request {} does not match the active context",
                        request_id
                    );
                }
                active_ctx = None;
                active_request_id = None;
                pending_invocation_id
                    .compare_exchange(request_id, 0, Ordering::AcqRel, Ordering::Acquire)
                    .ok();
                send_response(&tx, &repaint_ctx, ShellMenuResponse::Invoked { request_id });
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
                if active_request_id != Some(request_id)
                    || latest_request_id.load(Ordering::Acquire) != request_id
                {
                    continue;
                }

                let sub_items = if let Some(ref ctx) = active_ctx {
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
                        ctx.load_pending_submenu(shell_item);
                        shell_item
                            .sub_items
                            .iter()
                            .map(ShellMenuItemData::from_shell_item)
                            .collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    log::warn!("[ShellMenuWorker] LoadSubmenu called with no active context");
                    Vec::new()
                };
                send_response(
                    &tx,
                    &repaint_ctx,
                    ShellMenuResponse::SubmenuLoaded {
                        request_id,
                        item_id,
                        sub_items,
                    },
                );
            }
        }
    }
}

fn shell_menu_com_failure_loop(
    rx: Receiver<ShellMenuRequest>,
    control_rx: Receiver<ShellMenuRequest>,
    tx: Sender<ShellMenuResponse>,
    repaint_ctx: eframe::egui::Context,
    error: String,
) {
    let mut deferred_request = None;
    loop {
        let request = match receive_prioritized(&rx, &control_rx, &mut deferred_request) {
            PrioritizedReceive::Request(request) => request,
            PrioritizedReceive::Timeout => continue,
            PrioritizedReceive::Disconnected => break,
        };
        let response = match request {
            ShellMenuRequest::Extract { request_id, .. } => Some(ShellMenuResponse::Error {
                request_id,
                message: error.clone(),
            }),
            ShellMenuRequest::Invoke { request_id, .. } => {
                Some(ShellMenuResponse::Invoked { request_id })
            }
            ShellMenuRequest::LoadSubmenu {
                request_id,
                item_id,
            } => Some(ShellMenuResponse::SubmenuLoaded {
                request_id,
                item_id,
                sub_items: Vec::new(),
            }),
            ShellMenuRequest::Cancel => None,
        };
        if let Some(response) = response {
            send_response(&tx, &repaint_ctx, response);
        }
    }
}

fn pump_sta_messages() -> bool {
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            if message.message == WM_QUIT {
                return false;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    true
}
