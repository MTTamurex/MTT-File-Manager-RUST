//! Dedicated STA worker for Windows "Open with" application associations.
//!
//! Association enumeration is kept separate from the context-menu worker so a
//! slow third-party `IContextMenu` extension cannot delay the application list.

use std::collections::{HashMap, VecDeque};
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::Arc;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CoInitializeEx, CoTaskMemFree, CoUninitialize, IDataObject, COINIT_APARTMENTTHREADED,
    COINIT_MULTITHREADED,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    IAssocHandler, IShellFolder, SHAssocEnumHandlers, SHBindToParent, SHParseDisplayName,
    ASSOC_FILTER_NONE, ASSOC_FILTER_RECOMMENDED,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_QUIT,
};

use crate::infrastructure::prioritized_receiver::{receive_prioritized, PrioritizedReceive};

pub const OPEN_WITH_PARENT_ID: i32 = -201;
pub const OPEN_WITH_DIALOG_ID: i32 = -202;
pub const OPEN_WITH_HANDLER_ID_BASE: i32 = -1_000_000_000;
pub const OPEN_WITH_MENU_COMMAND: &str = "open_with_menu";
pub const OPEN_WITH_DIALOG_COMMAND: &str = "open_with_dialog";
pub const OPEN_WITH_HANDLER_COMMAND_PREFIX: &str = "open_with_handler:";
const MAX_ASSOCIATION_HANDLERS: usize = 64;
const MAX_HANDLER_NAME_UNITS: usize = 256;
const MAX_HANDLER_PATH_UNITS: usize = 32_768;

pub fn handler_id_from_command(command: &str) -> Option<u32> {
    command
        .strip_prefix(OPEN_WITH_HANDLER_COMMAND_PREFIX)?
        .parse()
        .ok()
}

pub fn menu_id_for_handler(handler_id: u32) -> Option<i32> {
    OPEN_WITH_HANDLER_ID_BASE.checked_sub(i32::try_from(handler_id).ok()?)
}

pub enum OpenWithRequest {
    Enumerate {
        request_id: u64,
        paths: Vec<PathBuf>,
    },
    Invoke {
        request_id: u64,
        handler_id: u32,
    },
    Cancel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenWithItemData {
    pub handler_id: u32,
    pub name: String,
}

pub enum OpenWithResponse {
    Ready {
        request_id: u64,
        items: Vec<OpenWithItemData>,
    },
    Error {
        request_id: u64,
        message: String,
    },
    Invoked {
        request_id: u64,
        result: Result<(), String>,
        fallback_path: Option<PathBuf>,
    },
    IconReady {
        request_id: u64,
        handler_id: u32,
        rgba: Vec<u8>,
        width: u32,
        height: u32,
    },
}

struct ComGuard;

impl ComGuard {
    fn init_sta() -> Result<Self, String> {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
            .ok()
            .map_err(|error| error.to_string())?;
        Ok(Self)
    }

    fn init_mta() -> Result<Self, String> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
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

struct ActiveOpenWith {
    request_id: u64,
    handlers: Vec<IAssocHandler>,
    data_object: IDataObject,
    path: PathBuf,
    extension: String,
}

struct IconLoadRequest {
    request_id: u64,
    extension: String,
    items: Vec<OpenWithItemData>,
}

pub fn start_open_with_worker(
    repaint_ctx: eframe::egui::Context,
    latest_request_id: Arc<AtomicU64>,
    pending_invocation_id: Arc<AtomicU64>,
) -> (
    SyncSender<OpenWithRequest>,
    SyncSender<OpenWithRequest>,
    Receiver<OpenWithResponse>,
) {
    let (req_tx, req_rx) = mpsc::sync_channel(64);
    let (control_tx, control_rx) = mpsc::sync_channel(4);
    let (res_tx, res_rx) = mpsc::channel();
    let (icon_tx, icon_rx) = mpsc::sync_channel(8);
    let icon_res_tx = res_tx.clone();
    let icon_repaint_ctx = repaint_ctx.clone();
    let icon_latest_request_id = Arc::clone(&latest_request_id);
    let worker_pending_invocation_id = Arc::clone(&pending_invocation_id);
    std::thread::spawn(move || {
        open_with_icon_loop(
            icon_rx,
            icon_res_tx,
            icon_repaint_ctx,
            icon_latest_request_id,
        )
    });
    std::thread::spawn(move || {
        open_with_loop(
            req_rx,
            control_rx,
            res_tx,
            repaint_ctx,
            latest_request_id,
            worker_pending_invocation_id,
            icon_tx,
        )
    });
    (req_tx, control_tx, res_rx)
}

fn send_response(
    tx: &Sender<OpenWithResponse>,
    repaint_ctx: &eframe::egui::Context,
    response: OpenWithResponse,
) {
    if tx.send(response).is_ok() {
        repaint_ctx.request_repaint();
    }
}

fn open_with_loop(
    rx: Receiver<OpenWithRequest>,
    control_rx: Receiver<OpenWithRequest>,
    tx: Sender<OpenWithResponse>,
    repaint_ctx: eframe::egui::Context,
    latest_request_id: Arc<AtomicU64>,
    pending_invocation_id: Arc<AtomicU64>,
    icon_tx: SyncSender<IconLoadRequest>,
) {
    let _com = match ComGuard::init_sta() {
        Ok(com) => com,
        Err(error) => {
            log::error!("[OpenWith] Failed to initialize STA COM: {}", error);
            open_with_com_failure_loop(
                rx,
                control_rx,
                tx,
                repaint_ctx,
                error,
                pending_invocation_id,
            );
            return;
        }
    };
    let mut active: Option<ActiveOpenWith> = None;
    let mut deferred_request = None;

    loop {
        if active.as_ref().is_some_and(|context| {
            latest_request_id.load(Ordering::Acquire) != context.request_id
                && pending_invocation_id.load(Ordering::Acquire) != context.request_id
        }) {
            active = None;
        }
        if !pump_sta_messages() {
            break;
        }
        let request = match receive_prioritized(&rx, &control_rx, &mut deferred_request) {
            PrioritizedReceive::Request(request) => request,
            PrioritizedReceive::Timeout => continue,
            PrioritizedReceive::Disconnected => break,
        };
        match request {
            OpenWithRequest::Enumerate { request_id, paths } => {
                if latest_request_id.load(Ordering::Acquire) != request_id {
                    continue;
                }
                active = None;
                let started = std::time::Instant::now();
                match enumerate_handlers(request_id, &paths) {
                    Ok((context, items)) => {
                        if latest_request_id.load(Ordering::Acquire) != request_id {
                            continue;
                        }
                        log::debug!(
                            "[OpenWith] Enumerated {} handlers in {:.1}ms",
                            items.len(),
                            started.elapsed().as_secs_f64() * 1000.0
                        );
                        let icon_request = IconLoadRequest {
                            request_id,
                            extension: context.extension.clone(),
                            items: items.clone(),
                        };
                        active = Some(context);
                        send_response(
                            &tx,
                            &repaint_ctx,
                            OpenWithResponse::Ready { request_id, items },
                        );
                        let _ = icon_tx.try_send(icon_request);
                    }
                    Err(message) => {
                        if latest_request_id.load(Ordering::Acquire) != request_id {
                            continue;
                        }
                        log::debug!(
                            "[OpenWith] Enumeration failed after {:.1}ms",
                            started.elapsed().as_secs_f64() * 1000.0
                        );
                        send_response(
                            &tx,
                            &repaint_ctx,
                            OpenWithResponse::Error {
                                request_id,
                                message,
                            },
                        )
                    }
                }
            }
            OpenWithRequest::Invoke {
                request_id,
                handler_id,
            } => {
                let fallback_path = active
                    .as_ref()
                    .filter(|context| context.request_id == request_id)
                    .map(|context| context.path.clone());
                let result = active
                    .as_ref()
                    .filter(|context| context.request_id == request_id)
                    .ok_or_else(|| "Open with context is no longer active".to_string())
                    .and_then(|context| {
                        context
                            .handlers
                            .get(handler_id as usize)
                            .ok_or_else(|| "Open with handler was not found".to_string())
                            .and_then(|handler| unsafe {
                                handler
                                    .Invoke(&context.data_object)
                                    .map_err(|error| error.to_string())
                            })
                    });
                let fallback_path = result.is_err().then_some(fallback_path).flatten();
                active = None;
                pending_invocation_id
                    .compare_exchange(request_id, 0, Ordering::AcqRel, Ordering::Acquire)
                    .ok();
                send_response(
                    &tx,
                    &repaint_ctx,
                    OpenWithResponse::Invoked {
                        request_id,
                        result,
                        fallback_path,
                    },
                );
            }
            OpenWithRequest::Cancel => {
                active = None;
            }
        }
    }
}

fn open_with_com_failure_loop(
    rx: Receiver<OpenWithRequest>,
    control_rx: Receiver<OpenWithRequest>,
    tx: Sender<OpenWithResponse>,
    repaint_ctx: eframe::egui::Context,
    error: String,
    pending_invocation_id: Arc<AtomicU64>,
) {
    let mut deferred_request = None;
    loop {
        let request = match receive_prioritized(&rx, &control_rx, &mut deferred_request) {
            PrioritizedReceive::Request(request) => request,
            PrioritizedReceive::Timeout => continue,
            PrioritizedReceive::Disconnected => break,
        };
        let response = match request {
            OpenWithRequest::Enumerate { request_id, .. } => Some(OpenWithResponse::Error {
                request_id,
                message: error.clone(),
            }),
            OpenWithRequest::Invoke { request_id, .. } => {
                pending_invocation_id
                    .compare_exchange(request_id, 0, Ordering::AcqRel, Ordering::Acquire)
                    .ok();
                Some(OpenWithResponse::Invoked {
                    request_id,
                    result: Err(error.clone()),
                    fallback_path: None,
                })
            }
            OpenWithRequest::Cancel => None,
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

fn enumerate_handlers(
    request_id: u64,
    paths: &[PathBuf],
) -> Result<(ActiveOpenWith, Vec<OpenWithItemData>), String> {
    let first_path = paths
        .first()
        .ok_or_else(|| "Open with requires a file path".to_string())?;
    let extension = first_path
        .extension()
        .filter(|extension| !extension.is_empty())
        .ok_or_else(|| "The selected file has no extension".to_string())?;
    let extension = format!(".{}", extension.to_string_lossy());
    let mut handlers = enumerate_associations(&extension)?;
    if handlers.is_empty() {
        return Err("Windows returned no associated applications".to_string());
    }

    let mut items = Vec::with_capacity(handlers.len());
    handlers.retain(|handler| {
        let Some(name) = handler_ui_name(handler) else {
            return false;
        };
        let handler_id = items.len() as u32;
        items.push(OpenWithItemData { handler_id, name });
        true
    });

    if handlers.is_empty() {
        return Err("Associated applications did not provide display names".to_string());
    }

    if paths.len() != 1 {
        return Err("Direct Open with requires exactly one file".to_string());
    }
    let path = first_path.clone();
    let data_object = create_shell_data_object(first_path)?;
    Ok((
        ActiveOpenWith {
            request_id,
            handlers,
            data_object,
            path,
            extension,
        },
        items,
    ))
}

fn enumerate_associations(extension: &str) -> Result<Vec<IAssocHandler>, String> {
    let extension_wide: Vec<u16> = std::ffi::OsStr::new(extension)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut handlers = enumerate_for_filter(&extension_wide, true)?;
    if handlers.is_empty() {
        handlers = enumerate_for_filter(&extension_wide, false)?;
    }
    Ok(handlers)
}

fn enumerate_for_filter(
    extension_wide: &[u16],
    recommended_only: bool,
) -> Result<Vec<IAssocHandler>, String> {
    let filter = if recommended_only {
        ASSOC_FILTER_RECOMMENDED
    } else {
        ASSOC_FILTER_NONE
    };
    let enumerator = unsafe { SHAssocEnumHandlers(PCWSTR(extension_wide.as_ptr()), filter) }
        .map_err(|error| error.to_string())?;
    let mut handlers = Vec::new();

    loop {
        let mut slot = [None];
        let mut fetched = 0;
        unsafe { enumerator.Next(&mut slot, Some(&mut fetched)) }
            .map_err(|error| error.to_string())?;
        if fetched == 0 {
            break;
        }
        if let Some(handler) = slot[0].take() {
            handlers.push(handler);
            if handlers.len() >= MAX_ASSOCIATION_HANDLERS {
                break;
            }
        }
    }
    Ok(handlers)
}

fn handler_ui_name(handler: &IAssocHandler) -> Option<String> {
    let value = unsafe { handler.GetUIName().ok()? };
    take_com_string(value, MAX_HANDLER_NAME_UNITS)
}

fn create_shell_data_object(path: &std::path::Path) -> Result<IDataObject, String> {
    let wide_path: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut absolute_pidl: *mut ITEMIDLIST = std::ptr::null_mut();

    unsafe {
        SHParseDisplayName(
            PCWSTR(wide_path.as_ptr()),
            None,
            &mut absolute_pidl,
            0,
            None,
        )
        .map_err(|error| error.to_string())?;
    }
    if absolute_pidl.is_null() {
        return Err("Windows returned an empty PIDL for Open with".to_string());
    }

    struct PidlGuard(*mut ITEMIDLIST);
    impl Drop for PidlGuard {
        fn drop(&mut self) {
            unsafe { CoTaskMemFree(Some(self.0.cast())) };
        }
    }
    let absolute_pidl = PidlGuard(absolute_pidl);
    let mut child_pidl: *mut ITEMIDLIST = std::ptr::null_mut();
    let parent: IShellFolder = unsafe { SHBindToParent(absolute_pidl.0, Some(&mut child_pidl)) }
        .map_err(|error| error.to_string())?;
    if child_pidl.is_null() {
        return Err("Windows returned an empty child PIDL for Open with".to_string());
    }

    let children = [child_pidl as *const ITEMIDLIST];
    let data_object: IDataObject =
        unsafe { parent.GetUIObjectOf(HWND::default(), &children, None) }
            .map_err(|error| error.to_string())?;
    Ok(data_object)
}

fn open_with_icon_loop(
    rx: Receiver<IconLoadRequest>,
    tx: Sender<OpenWithResponse>,
    repaint_ctx: eframe::egui::Context,
    latest_request_id: Arc<AtomicU64>,
) {
    let _com = match ComGuard::init_mta() {
        Ok(com) => com,
        Err(error) => {
            log::warn!("[OpenWith] Failed to initialize icon worker COM: {}", error);
            return;
        }
    };
    while let Ok(mut request) = rx.recv() {
        while let Ok(newer_request) = rx.try_recv() {
            request = newer_request;
        }
        if latest_request_id.load(Ordering::Acquire) != request.request_id {
            continue;
        }
        let Ok(handlers) = enumerate_associations(&request.extension) else {
            continue;
        };
        if latest_request_id.load(Ordering::Acquire) != request.request_id {
            continue;
        }
        let mut requested_ids: HashMap<String, VecDeque<u32>> = HashMap::new();
        for item in request.items {
            requested_ids
                .entry(item.name)
                .or_default()
                .push_back(item.handler_id);
        }
        for handler in handlers {
            if latest_request_id.load(Ordering::Acquire) != request.request_id {
                break;
            }
            let Some(name) = handler_ui_name(&handler) else {
                continue;
            };
            let Some(handler_id) = requested_ids.get_mut(&name).and_then(VecDeque::pop_front)
            else {
                continue;
            };
            let mut path = PWSTR::null();
            let mut index = 0;
            let icon_result = unsafe { handler.GetIconLocation(&mut path, &mut index) };
            let resource = if icon_result.is_ok() {
                take_com_string(path, MAX_HANDLER_PATH_UNITS).map(|path| (path, index))
            } else {
                free_com_string(path);
                None
            };
            let executable = unsafe { handler.GetName().ok() }
                .and_then(|value| take_com_string(value, MAX_HANDLER_PATH_UNITS))
                .map(PathBuf::from);
            let icon = crate::infrastructure::windows::open_with_icons::extract_handler_icon(
                resource.as_ref(),
                executable.as_deref(),
            );
            let Some((rgba, width, height)) = icon else {
                continue;
            };
            send_response(
                &tx,
                &repaint_ctx,
                OpenWithResponse::IconReady {
                    request_id: request.request_id,
                    handler_id,
                    rgba,
                    width,
                    height,
                },
            );
        }
    }
}

fn take_com_string(value: PWSTR, max_units: usize) -> Option<String> {
    if value.is_null() {
        return None;
    }
    let text = unsafe {
        let mut len = 0;
        while len < max_units && *value.0.add(len) != 0 {
            len += 1;
        }
        (len < max_units)
            .then(|| String::from_utf16_lossy(std::slice::from_raw_parts(value.0, len)))
    };
    unsafe { CoTaskMemFree(Some(value.0.cast())) };
    text.filter(|text| !text.trim().is_empty())
}

fn free_com_string(value: PWSTR) {
    if !value.is_null() {
        unsafe { CoTaskMemFree(Some(value.0.cast())) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_handler_ids_use_a_private_negative_range() {
        assert_eq!(menu_id_for_handler(0), Some(-1_000_000_000));
        assert_eq!(menu_id_for_handler(7), Some(-1_000_000_007));
        assert_eq!(menu_id_for_handler(u32::MAX), None);
    }

    #[test]
    fn handler_commands_only_accept_numeric_suffixes() {
        assert_eq!(handler_id_from_command("open_with_handler:7"), Some(7));
        assert_eq!(handler_id_from_command("open_with_handler:nope"), None);
        assert_eq!(handler_id_from_command("other:7"), None);
    }

    #[test]
    fn creates_native_shell_data_object_for_one_file() {
        let result = std::thread::spawn(|| {
            let _com = ComGuard::init_sta()?;
            let path = std::env::temp_dir().join(format!(
                "mtt_open_with_data_object_{}_{}.txt",
                std::process::id(),
                std::thread::current().name().unwrap_or("test")
            ));
            std::fs::File::create(&path).map_err(|error| error.to_string())?;
            let result = create_shell_data_object(&path).map(|_| ());
            let _ = std::fs::remove_file(&path);
            result
        })
        .join()
        .expect("Open with data-object test thread panicked");

        assert!(result.is_ok(), "{result:?}");
    }
}
