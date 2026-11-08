use serde_json::{json, Value};
use std::{sync::mpsc, thread, time::Duration};
use windows_sys::Win32::System::{
    ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
    Threading::{GetCurrentProcess, GetProcessHandleCount},
};

pub fn resources() -> Value {
    unsafe {
        let mut memory: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        let size = std::mem::size_of_val(&memory) as u32;
        memory.cb = size;
        let process = GetCurrentProcess();
        let memory_ok = GetProcessMemoryInfo(process, &mut memory, size) != 0;
        let mut handles = 0;
        let handles_ok = GetProcessHandleCount(process, &mut handles) != 0;
        json!({
            "working_set_bytes": memory_ok.then_some(memory.WorkingSetSize),
            "commit_bytes": memory_ok.then_some(memory.PagefileUsage),
            "handles": handles_ok.then_some(handles),
        })
    }
}

/// This standalone POC can kill itself on a native hang, including init/teardown.
/// A production in-process thread cannot offer this guarantee safely.
pub struct Watchdog {
    cancel: Option<mpsc::Sender<()>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Watchdog {
    pub fn start(timeout: Duration) -> Self {
        let (cancel, receiver) = mpsc::channel();
        let thread = thread::spawn(move || {
            if matches!(
                receiver.recv_timeout(timeout),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                eprintln!("POC deadline exceeded; terminating the test process (exit 124)");
                std::process::exit(124);
            }
        });
        Self {
            cancel: Some(cancel),
            thread: Some(thread),
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn percentile(sorted: &[f64], percent: usize) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (sorted.len() * percent).div_ceil(100).max(1);
    sorted.get(rank - 1).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles() {
        assert_eq!(percentile(&[], 95), None);
        assert_eq!(percentile(&[3.0], 95), Some(3.0));
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0], 50), Some(2.0));
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0], 95), Some(4.0));
    }
}
