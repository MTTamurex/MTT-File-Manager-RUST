//! Minimal dynamically loaded subset of the public libmpv client.h ABI.
//! No import library, extra decoder DLL, or dependency on the application's UI.
use anyhow::{bail, ensure, Context, Result};
use libloading::os::windows::{
    Library, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::path::Path;

#[repr(C)]
pub struct Node {
    pub value: Value,
    pub format: c_int,
}

#[repr(C)]
pub union Value {
    pub string: *mut c_char,
    pub flag: c_int,
    pub integer: i64,
    pub double: f64,
    pub list: *mut NodeList,
    pub bytes: *mut ByteArray,
}

#[repr(C)]
pub struct NodeList {
    pub count: c_int,
    pub values: *mut Node,
    pub keys: *mut *mut c_char,
}

#[repr(C)]
pub struct ByteArray {
    pub data: *mut c_void,
    pub size: usize,
}

#[repr(C)]
pub struct Event {
    pub id: c_int,
    pub error: c_int,
    pub userdata: u64,
    pub data: *mut c_void,
}

#[repr(C)]
pub struct CommandReply {
    pub result: Node,
}

pub struct Api {
    pub(crate) create: unsafe extern "C" fn() -> *mut c_void,
    pub(crate) initialize: unsafe extern "C" fn(*mut c_void) -> c_int,
    pub(crate) set_option: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> c_int,
    pub(crate) command: unsafe extern "C" fn(*mut c_void, u64, *const *const c_char) -> c_int,
    pub(crate) wait: unsafe extern "C" fn(*mut c_void, f64) -> *mut Event,
    pub(crate) get_property:
        unsafe extern "C" fn(*mut c_void, *const c_char, c_int, *mut c_void) -> c_int,
    pub(crate) free_node: unsafe extern "C" fn(*mut Node),
    pub(crate) destroy: unsafe extern "C" fn(*mut c_void),
    pub(crate) error_string: unsafe extern "C" fn(c_int) -> *const c_char,
    pub(crate) client_version: unsafe extern "C" fn() -> u64,
    _library: Library,
}

impl Api {
    pub fn load(path: &Path) -> Result<Self> {
        // Absolute path and restricted dependency search exclude PATH/CWD DLL planting.
        let path = path
            .canonicalize()
            .context("Cannot resolve the requested libmpv DLL")?;
        unsafe {
            let library = Library::load_with_flags(
                &path,
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
            .context("Cannot load libmpv or one of its runtime dependencies")?;
            let api = Self {
                create: *library.get(b"mpv_create\0")?,
                initialize: *library.get(b"mpv_initialize\0")?,
                set_option: *library.get(b"mpv_set_option_string\0")?,
                command: *library.get(b"mpv_command_async\0")?,
                wait: *library.get(b"mpv_wait_event\0")?,
                get_property: *library.get(b"mpv_get_property\0")?,
                free_node: *library.get(b"mpv_free_node_contents\0")?,
                destroy: *library.get(b"mpv_terminate_destroy\0")?,
                error_string: *library.get(b"mpv_error_string\0")?,
                client_version: *library.get(b"mpv_client_api_version\0")?,
                _library: library,
            };
            ensure!(
                (api.client_version)() >> 16 == 2,
                "This POC requires libmpv client API major 2"
            );
            Ok(api)
        }
    }

    pub(crate) fn check(&self, status: c_int, operation: &str) -> Result<()> {
        if status < 0 {
            // SAFETY: libmpv returns a static NUL-terminated error description.
            let message = unsafe { CStr::from_ptr((self.error_string)(status)) }.to_string_lossy();
            bail!("{operation}: {message} ({status})");
        }
        Ok(())
    }
}

#[cfg(all(test, target_pointer_width = "64"))]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn client_header_x64_layouts() {
        assert_eq!(size_of::<Node>(), 16);
        assert_eq!(offset_of!(Node, format), 8);
        assert_eq!(size_of::<NodeList>(), 24);
        assert_eq!(offset_of!(NodeList, values), 8);
        assert_eq!(offset_of!(NodeList, keys), 16);
        assert_eq!(size_of::<ByteArray>(), 16);
        assert_eq!(size_of::<Event>(), 24);
        assert_eq!(offset_of!(Event, userdata), 8);
        assert_eq!(offset_of!(Event, data), 16);
        assert_eq!(size_of::<CommandReply>(), 16);
    }
}
