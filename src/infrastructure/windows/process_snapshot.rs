use super::OwnedHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, GetProcessHandleCount, OpenThread,
    TerminateProcess, THREAD_TERMINATE,
};
use windows::Win32::System::IO::CancelSynchronousIo;

pub struct ProcessKernelResources {
    pub gdi_objects: u32,
    pub user_objects: u32,
    pub handle_count: u32,
    pub thread_count: u32,
}

const GR_GDIOBJECTS: u32 = 0;
const GR_USEROBJECTS: u32 = 1;

extern "system" {
    fn GetGuiResources(hprocess: *mut core::ffi::c_void, uiflags: u32) -> u32;
}

pub fn get_current_process_kernel_resources() -> ProcessKernelResources {
    unsafe {
        let process = GetCurrentProcess();
        let handle_ptr = process.0 as *mut core::ffi::c_void;
        let gdi = GetGuiResources(handle_ptr, GR_GDIOBJECTS);
        let user = GetGuiResources(handle_ptr, GR_USEROBJECTS);

        let mut handles: u32 = 0;
        let _ = GetProcessHandleCount(process, &mut handles);

        ProcessKernelResources {
            gdi_objects: gdi,
            user_objects: user,
            handle_count: handles,
            thread_count: count_current_process_threads(),
        }
    }
}

pub fn count_current_process_threads() -> u32 {
    let mut count = 0u32;
    let _ = for_each_current_process_thread(|_| {
        count += 1;
    });
    count
}

pub fn cancel_pending_io_on_current_process_threads() -> u32 {
    let current_tid = unsafe { GetCurrentThreadId() };
    let mut cancelled = 0u32;

    let _ = for_each_current_process_thread(|thread_id| {
        if thread_id == current_tid {
            return;
        }

        let thread_handle = match unsafe { OpenThread(THREAD_TERMINATE, false, thread_id) } {
            Ok(handle) => handle,
            Err(_) => return,
        };
        let Some(thread_handle) = OwnedHandle::new(thread_handle) else {
            return;
        };

        if unsafe { CancelSynchronousIo(thread_handle.as_raw()) }.is_ok() {
            cancelled += 1;
        }
    });

    cancelled
}

pub fn terminate_current_process(exit_code: u32) {
    unsafe {
        let _ = TerminateProcess(GetCurrentProcess(), exit_code);
    }
}

fn for_each_current_process_thread(mut f: impl FnMut(u32)) -> bool {
    let current_pid = unsafe { GetCurrentProcessId() };
    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) } {
        Ok(handle) => handle,
        Err(_) => return false,
    };
    let Some(snapshot) = OwnedHandle::new(snapshot) else {
        return false;
    };

    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };

    unsafe {
        if Thread32First(snapshot.as_raw(), &mut entry).is_err() {
            return false;
        }

        loop {
            if entry.th32OwnerProcessID == current_pid {
                f(entry.th32ThreadID);
            }

            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            if Thread32Next(snapshot.as_raw(), &mut entry).is_err() {
                break;
            }
        }
    }

    true
}
