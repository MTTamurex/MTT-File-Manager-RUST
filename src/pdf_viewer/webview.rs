use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::{LoadLibraryW, GetProcAddress};
use windows::Win32::UI::WindowsAndMessaging::*;

// GUIDs
const IID_ICoreWebView2Environment: GUID = GUID::from_u128(0x33D17ECE_82FA_47D9_83E6_131350E3ED79);
const IID_ICoreWebView2Controller: GUID = GUID::from_u128(0x4D00C0D1_9455_4428_9463_47C941B300C9);
const IID_ICoreWebView2: GUID = GUID::from_u128(0x76ECEACB_0462_4D94_AC83_420A6DDA05D2);
const IID_ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler: GUID = GUID::from_u128(0x4E8A3389_C9D8_4BD2_B6B5_124FEE6CC14D);
const IID_ICoreWebView2CreateCoreWebView2ControllerCompletedHandler: GUID = GUID::from_u128(0x6C4819F3_C9B7_4260_8127_C9F5BDE7F68C);
const IID_IUnknown: GUID = GUID::from_u128(0x00000000_0000_0000_C000_000000000046);

// VTable Definitions
#[repr(C)]
struct ICoreWebView2Environment_Vtbl {
    pub base: IUnknown_Vtbl,
    pub CreateCoreWebView2Controller: unsafe extern "system" fn(*mut c_void, HWND, *mut c_void) -> HRESULT,
    pub CreateWebResourceRequest: unsafe extern "system" fn(*mut c_void, PCWSTR, PCWSTR, *mut c_void, PCWSTR, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct ICoreWebView2Controller_Vtbl {
    pub base: IUnknown_Vtbl,
    pub get_IsVisible: unsafe extern "system" fn(*mut c_void, *mut BOOL) -> HRESULT,
    pub put_IsVisible: unsafe extern "system" fn(*mut c_void, BOOL) -> HRESULT,
    pub get_Bounds: unsafe extern "system" fn(*mut c_void, *mut RECT) -> HRESULT,
    pub put_Bounds: unsafe extern "system" fn(*mut c_void, RECT) -> HRESULT,
    pub get_ZoomFactor: unsafe extern "system" fn(*mut c_void, *mut f64) -> HRESULT,
    pub put_ZoomFactor: unsafe extern "system" fn(*mut c_void, f64) -> HRESULT,
    pub add_ZoomFactorChanged: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i64) -> HRESULT,
    pub remove_ZoomFactorChanged: unsafe extern "system" fn(*mut c_void, i64) -> HRESULT,
    pub SetBoundsAndZoomFactor: unsafe extern "system" fn(*mut c_void, RECT, f64) -> HRESULT,
    pub MoveFocus: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    pub add_MoveFocusRequested: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i64) -> HRESULT,
    pub remove_MoveFocusRequested: unsafe extern "system" fn(*mut c_void, i64) -> HRESULT,
    pub add_GotFocus: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i64) -> HRESULT,
    pub remove_GotFocus: unsafe extern "system" fn(*mut c_void, i64) -> HRESULT,
    pub add_LostFocus: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i64) -> HRESULT,
    pub remove_LostFocus: unsafe extern "system" fn(*mut c_void, i64) -> HRESULT,
    pub add_AcceleratorKeyPressed: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut i64) -> HRESULT,
    pub remove_AcceleratorKeyPressed: unsafe extern "system" fn(*mut c_void, i64) -> HRESULT,
    pub get_ParentWindow: unsafe extern "system" fn(*mut c_void, *mut HWND) -> HRESULT,
    pub put_ParentWindow: unsafe extern "system" fn(*mut c_void, HWND) -> HRESULT,
    pub NotifyParentWindowPositionChanged: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub Close: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    pub get_CoreWebView2: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
}

#[repr(C)]
struct ICoreWebView2_Vtbl {
    pub base: IUnknown_Vtbl,
    pub get_Settings: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    pub get_Source: unsafe extern "system" fn(*mut c_void, *mut PWSTR) -> HRESULT,
    pub Navigate: unsafe extern "system" fn(*mut c_void, PCWSTR) -> HRESULT,
    // ... many more, but we only need Navigate for now
}

// Wrapper Structs
struct CoreWebView2Environment { ptr: *mut c_void }
struct CoreWebView2Controller { ptr: *mut c_void }
struct CoreWebView2 { ptr: *mut c_void }

// State management
pub struct WebViewState {
    controller: CoreWebView2Controller,
    webview: CoreWebView2,
}

impl Drop for WebViewState {
    fn drop(&mut self) {
        unsafe {
            let vtbl = *(self.controller.ptr as *mut *mut ICoreWebView2Controller_Vtbl);
            ((*vtbl).base.Release)(self.controller.ptr);
            
            let vtbl_wv = *(self.webview.ptr as *mut *mut ICoreWebView2_Vtbl);
            ((*vtbl_wv).base.Release)(self.webview.ptr);
        }
    }
}

// API
pub fn init(hwnd: HWND, url: String) -> Result<()> {
    unsafe {
        eprintln!("WebView2: Loading WebView2Loader.dll");
        // Load DLL
        let dll_name = w!("WebView2Loader.dll");
        let hmodule = LoadLibraryW(dll_name)?;
        
        eprintln!("WebView2: Getting ProcAddress");
        let func_name = s!("CreateCoreWebView2EnvironmentWithOptions");
        let create_env_ptr = GetProcAddress(hmodule, func_name)
            .ok_or(Error::from_win32())?;
            
        let create_env: unsafe extern "system" fn(
            PCWSTR, PCWSTR, *mut c_void, *mut c_void
        ) -> HRESULT = std::mem::transmute(create_env_ptr);

        // Create Environment Handler
        let handler = EnvironmentCompletedHandler::create(hwnd, url);
        let handler_ptr: *mut c_void = handler;

        eprintln!("WebView2: Calling CreateCoreWebView2EnvironmentWithOptions");
        let hr = create_env(PCWSTR::null(), PCWSTR::null(), std::ptr::null_mut(), handler_ptr);
        eprintln!("WebView2: CreateEnv result: {:?}", hr);
        hr.ok()
    }
}

pub fn resize(hwnd: HWND) {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if ptr != 0 {
            let state = &*(ptr as *mut WebViewState);
            let mut rect = RECT::default();
            GetClientRect(hwnd, &mut rect);
            
            let vtbl = *(state.controller.ptr as *mut *mut ICoreWebView2Controller_Vtbl);
            ((*vtbl).put_Bounds)(state.controller.ptr, rect);
        }
    }
}

pub fn close(hwnd: HWND) {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
        if ptr != 0 {
            let _state = Box::from_raw(ptr as *mut WebViewState);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            // Drop state -> Releases COM objects
        }
    }
}

// COM Handlers
#[repr(C)]
struct EnvironmentCompletedHandler_Vtbl {
    pub base: IUnknown_Vtbl,
    pub Invoke: unsafe extern "system" fn(*mut c_void, HRESULT, *mut c_void) -> HRESULT,
}

#[repr(C)]
struct EnvironmentCompletedHandler {
    vtbl: *const EnvironmentCompletedHandler_Vtbl,
    ref_count: AtomicUsize,
    hwnd: HWND,
    url: String,
}

impl EnvironmentCompletedHandler {
    fn create(hwnd: HWND, url: String) -> *mut c_void {
        let handler = Box::new(Self {
            vtbl: &ENV_HANDLER_VTBL,
            ref_count: AtomicUsize::new(1),
            hwnd,
            url,
        });
        Box::into_raw(handler) as *mut c_void
    }

    unsafe extern "system" fn QueryInterface(this: *mut c_void, iid: *const GUID, obj: *mut *mut c_void) -> HRESULT {
        if *iid == IID_IUnknown || *iid == IID_ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler {
            *obj = this;
            Self::AddRef(this);
            return HRESULT(0);
        }
        *obj = null_mut();
        HRESULT(0x80004002u32 as i32) // E_NOINTERFACE
    }

    unsafe extern "system" fn AddRef(this: *mut c_void) -> u32 {
        let handler = &*(this as *mut Self);
        handler.ref_count.fetch_add(1, Ordering::Relaxed) as u32 + 1
    }

    unsafe extern "system" fn Release(this: *mut c_void) -> u32 {
        let handler = &*(this as *mut Self);
        let count = handler.ref_count.fetch_sub(1, Ordering::Relaxed) - 1;
        if count == 0 {
            let _ = Box::from_raw(this as *mut Self);
        }
        count as u32
    }

    unsafe extern "system" fn Invoke(this: *mut c_void, result: HRESULT, env: *mut c_void) -> HRESULT {
        eprintln!("WebView2: EnvHandler::Invoke called. Result: {:?}, Env: {:p}", result, env);
        if result.is_err() || env.is_null() { return HRESULT(0); }
        let handler = &*(this as *mut Self);

        // Env -> CreateController
        eprintln!("WebView2: Creating Controller");
        let vtbl = *(env as *mut *mut ICoreWebView2Environment_Vtbl);
        let controller_handler = ControllerCompletedHandler::create(handler.hwnd, handler.url.clone());
        
        let hr = ((*vtbl).CreateCoreWebView2Controller)(env, handler.hwnd, controller_handler);
        eprintln!("WebView2: CreateController result: {:?}", hr);
        HRESULT(0)
    }
}

static ENV_HANDLER_VTBL: EnvironmentCompletedHandler_Vtbl = EnvironmentCompletedHandler_Vtbl {
    base: IUnknown_Vtbl {
        QueryInterface: EnvironmentCompletedHandler::QueryInterface,
        AddRef: EnvironmentCompletedHandler::AddRef,
        Release: EnvironmentCompletedHandler::Release,
    },
    Invoke: EnvironmentCompletedHandler::Invoke,
};

#[repr(C)]
struct ControllerCompletedHandler_Vtbl {
    pub base: IUnknown_Vtbl,
    pub Invoke: unsafe extern "system" fn(*mut c_void, HRESULT, *mut c_void) -> HRESULT,
}

#[repr(C)]
struct ControllerCompletedHandler {
    vtbl: *const ControllerCompletedHandler_Vtbl,
    ref_count: AtomicUsize,
    hwnd: HWND,
    url: String,
}

impl ControllerCompletedHandler {
    fn create(hwnd: HWND, url: String) -> *mut c_void {
        let handler = Box::new(Self {
            vtbl: &CONTROLLER_HANDLER_VTBL,
            ref_count: AtomicUsize::new(1),
            hwnd,
            url,
        });
        Box::into_raw(handler) as *mut c_void
    }

    unsafe extern "system" fn QueryInterface(this: *mut c_void, iid: *const GUID, obj: *mut *mut c_void) -> HRESULT {
        if *iid == IID_IUnknown || *iid == IID_ICoreWebView2CreateCoreWebView2ControllerCompletedHandler {
            *obj = this;
            Self::AddRef(this);
            return HRESULT(0);
        }
        *obj = null_mut();
        HRESULT(0x80004002u32 as i32)
    }

    unsafe extern "system" fn AddRef(this: *mut c_void) -> u32 {
        let handler = &*(this as *mut Self);
        handler.ref_count.fetch_add(1, Ordering::Relaxed) as u32 + 1
    }

    unsafe extern "system" fn Release(this: *mut c_void) -> u32 {
        let handler = &*(this as *mut Self);
        let count = handler.ref_count.fetch_sub(1, Ordering::Relaxed) - 1;
        if count == 0 {
            let _ = Box::from_raw(this as *mut Self);
        }
        count as u32
    }

    unsafe extern "system" fn Invoke(this: *mut c_void, result: HRESULT, controller_ptr: *mut c_void) -> HRESULT {
        if result.is_err() || controller_ptr.is_null() { return HRESULT(0); }
        let handler = &*(this as *mut Self);

        // Get CoreWebView2
        let ctrl_vtbl = *(controller_ptr as *mut *mut ICoreWebView2Controller_Vtbl);
        let mut webview_ptr: *mut c_void = null_mut();
        ((*ctrl_vtbl).get_CoreWebView2)(controller_ptr, &mut webview_ptr);

        if !webview_ptr.is_null() {
            // Resize to window
            let mut rect = RECT::default();
            GetClientRect(handler.hwnd, &mut rect);
            ((*ctrl_vtbl).put_Bounds)(controller_ptr, rect);

            // Navigate
             let mut wide_url: Vec<u16> = handler.url.encode_utf16().collect();
            wide_url.push(0);
            let wv_vtbl = *(webview_ptr as *mut *mut ICoreWebView2_Vtbl);
             ((*wv_vtbl).Navigate)(webview_ptr, PCWSTR::from_raw(wide_url.as_ptr()));
            
            // Store state
            let state = Box::new(WebViewState {
                controller: CoreWebView2Controller { ptr: controller_ptr },
                webview: CoreWebView2 { ptr: webview_ptr },
            });
            
            // AddRef because we kept pointers
            ((*ctrl_vtbl).base.AddRef)(controller_ptr);
            ((*wv_vtbl).base.AddRef)(webview_ptr);

            SetWindowLongPtrW(handler.hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
        }
        
        HRESULT(0)
    }
}

static CONTROLLER_HANDLER_VTBL: ControllerCompletedHandler_Vtbl = ControllerCompletedHandler_Vtbl {
    base: IUnknown_Vtbl {
        QueryInterface: ControllerCompletedHandler::QueryInterface,
        AddRef: ControllerCompletedHandler::AddRef,
        Release: ControllerCompletedHandler::Release,
    },
    Invoke: ControllerCompletedHandler::Invoke,
};

// --- WARMUP HANDLER ---
// This handler is used solely to initialize the WebView2 runtime process.
// It does NOT create a Controller or Window. It just ensures the DLL is loaded and the
// browser process is started/ready.

#[repr(C)]
struct WarmupCompletedHandler {
    vtbl: *const EnvironmentCompletedHandler_Vtbl, // Reusing the same VTable type as it's the same interface
    ref_count: AtomicUsize,
}

impl WarmupCompletedHandler {
    fn create() -> *mut c_void {
        let handler = Box::new(Self {
            vtbl: &WARMUP_HANDLER_VTBL,
            ref_count: AtomicUsize::new(1),
        });
        Box::into_raw(handler) as *mut c_void
    }

    unsafe extern "system" fn QueryInterface(this: *mut c_void, iid: *const GUID, obj: *mut *mut c_void) -> HRESULT {
        if *iid == IID_IUnknown || *iid == IID_ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler {
            *obj = this;
            Self::AddRef(this);
            return HRESULT(0);
        }
        *obj = null_mut();
        HRESULT(0x80004002u32 as i32)
    }

    unsafe extern "system" fn AddRef(this: *mut c_void) -> u32 {
        let handler = &*(this as *mut Self);
        handler.ref_count.fetch_add(1, Ordering::Relaxed) as u32 + 1
    }

    unsafe extern "system" fn Release(this: *mut c_void) -> u32 {
        let handler = &*(this as *mut Self);
        let count = handler.ref_count.fetch_sub(1, Ordering::Relaxed) - 1;
        if count == 0 {
            let _ = Box::from_raw(this as *mut Self);
        }
        count as u32
    }

    unsafe extern "system" fn Invoke(_this: *mut c_void, result: HRESULT, env: *mut c_void) -> HRESULT {
        // This is where the magic happens (or doesn't).
        // We successfully created the environment, which means the runtime is loaded.
        // We simply release the environment and return.
        // This keeps the "msedgewebview2.exe" process warm for a short period or in cache.
        
        if result.is_err() || env.is_null() {
            // Silently fail in warmup
            return HRESULT(0);
        }

        // Just release the environment immediately.
        // The act of creating it was enough to warm up the DLL and likely spawn the child process.
        let vtbl = *(env as *mut *mut ICoreWebView2Environment_Vtbl);
        ((*vtbl).base.Release)(env);

        HRESULT(0)
    }
}

static WARMUP_HANDLER_VTBL: EnvironmentCompletedHandler_Vtbl = EnvironmentCompletedHandler_Vtbl {
    base: IUnknown_Vtbl {
        QueryInterface: WarmupCompletedHandler::QueryInterface,
        AddRef: WarmupCompletedHandler::AddRef,
        Release: WarmupCompletedHandler::Release,
    },
    Invoke: WarmupCompletedHandler::Invoke,
};

pub fn warmup_env() -> Result<()> {
    unsafe {
        // Load DLL silently
         let dll_name = w!("WebView2Loader.dll");
        // We use LoadLibraryW directly. If it's already loaded, it just increments ref count.
        let hmodule = LoadLibraryW(dll_name).ok().ok_or(Error::from_win32())?;
        
        let func_name = s!("CreateCoreWebView2EnvironmentWithOptions");
        let create_env_ptr = GetProcAddress(hmodule, func_name)
            .ok_or(Error::from_win32())?;
            
        let create_env: unsafe extern "system" fn(
            PCWSTR, PCWSTR, *mut c_void, *mut c_void
        ) -> HRESULT = std::mem::transmute(create_env_ptr);

        let handler = WarmupCompletedHandler::create();
        
        // Pass NULL for user_data_folder to use default (standard for app),
        // OR use the same logic as real viewer if we set specific path.
        // Real viewer uses nullptr (default), so we use nullptr here too.
        let _ = create_env(PCWSTR::null(), PCWSTR::null(), std::ptr::null_mut(), handler);
        
        Ok(())
    }
}
