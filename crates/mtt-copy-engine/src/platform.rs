use crate::{plan, scheduler::Permit, staging::StagingFile, Progress};
use std::cell::Cell;
use std::io;
use std::sync::atomic::Ordering;

struct Transfer<'a> {
    progress: &'a Progress,
    permit: Option<&'a Permit<'a>>,
    bytes: Cell<u64>,
    committed: bool,
}

impl Transfer<'_> {
    fn checkpoint(&self) -> io::Result<()> {
        match self.permit {
            Some(permit) => permit.checkpoint(),
            None => self.progress.checkpoint(),
        }
    }
    fn update(&self, current: u64) {
        let previous = self.bytes.get();
        self.bytes.set(current.max(previous));
        self.progress
            .bytes_copied
            .fetch_add(current.saturating_sub(previous), Ordering::Relaxed);
        self.progress
            .telemetry
            .transferred
            .fetch_add(current.saturating_sub(previous), Ordering::Relaxed);
    }
}

impl Drop for Transfer<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.progress
                .bytes_copied
                .fetch_sub(self.bytes.get(), Ordering::Relaxed);
        }
    }
}

fn check_source(item: &plan::File) -> io::Result<()> {
    plan::check_path(&item.source)?;
    let metadata = std::fs::metadata(&item.source)?;
    if !metadata.is_file()
        || metadata.len() != item.size
        || metadata.modified().ok() != item.modified
    {
        return Err(io::Error::other("Source changed after copy preflight"));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn copy_file(item: &plan::File, progress: &Progress) -> io::Result<()> {
    copy_file_impl(item, progress, None)
}

pub(crate) fn copy_file_scheduled<'a>(
    item: &plan::File,
    progress: &'a Progress,
    permit: &'a Permit<'a>,
) -> io::Result<()> {
    copy_file_impl(item, progress, Some(permit))
}

fn copy_file_impl<'a>(
    item: &plan::File,
    progress: &'a Progress,
    permit: Option<&'a Permit<'a>>,
) -> io::Result<()> {
    match permit {
        Some(permit) => permit.checkpoint()?,
        None => progress.checkpoint()?,
    }
    check_source(item)?;
    let parent = item
        .destination
        .parent()
        .ok_or_else(|| io::Error::other("Invalid target"))?;
    plan::check_path(parent)?;
    plan::require_absent(&item.destination)?;
    let _active_file = progress.track_file(&item.source);
    let temporary = StagingFile::new(parent)?;
    let mut transfer = Transfer {
        progress,
        permit,
        bytes: Cell::new(0),
        committed: false,
    };
    copy_contents(&item.source, temporary.path(), item.size, &transfer)?;
    transfer.checkpoint()?;
    check_source(item)?;
    if std::fs::metadata(temporary.path())?.len() != item.size {
        return Err(io::Error::other("Copy size verification failed"));
    }
    plan::check_path(parent)?;
    temporary.publish(&item.destination)?;
    // Include empty files and APIs that did not issue a final progress callback.
    transfer.update(item.size);
    transfer.committed = true;
    Ok(())
}

#[cfg(windows)]
unsafe extern "system" fn callback(
    _total: i64,
    transferred: i64,
    _stream: i64,
    _stream_transferred: i64,
    _number: u32,
    _reason: windows::Win32::Storage::FileSystem::LPPROGRESS_ROUTINE_CALLBACK_REASON,
    _source: windows::Win32::Foundation::HANDLE,
    _target: windows::Win32::Foundation::HANDLE,
    data: *const std::ffi::c_void,
) -> windows::Win32::Storage::FileSystem::COPYPROGRESSROUTINE_PROGRESS {
    use windows::Win32::Storage::FileSystem::{PROGRESS_CANCEL, PROGRESS_CONTINUE};
    // CopyFileEx invokes this synchronously while Transfer remains alive.
    let state = unsafe { &*data.cast::<Transfer<'_>>() };
    state.update(transferred.max(0) as u64);
    if state.checkpoint().is_err() {
        return PROGRESS_CANCEL;
    }
    PROGRESS_CONTINUE
}

#[cfg(windows)]
fn copy_flags(size: u64) -> windows::Win32::Storage::FileSystem::COPYFILE_FLAGS {
    use windows::Win32::Storage::FileSystem::{COPYFILE_FLAGS, COPY_FILE_NO_BUFFERING};
    // Match BetterCopy's large-file cache policy, independently of worker budgets.
    if size >= 256 * 1024 * 1024 {
        COPY_FILE_NO_BUFFERING
    } else {
        COPYFILE_FLAGS(0)
    }
}

#[cfg(windows)]
fn copy_contents(
    source: &std::path::Path,
    target: &std::path::Path,
    size: u64,
    transfer: &Transfer<'_>,
) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::CopyFileExW;
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        CopyFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            Some(callback),
            Some((transfer as *const Transfer<'_>).cast()),
            None,
            copy_flags(size),
        )
    }
    .map_err(|error| io::Error::from_raw_os_error(error.code().0 & 0xFFFF))
}

#[cfg(not(windows))]
fn copy_contents(
    source: &std::path::Path,
    target: &std::path::Path,
    _size: u64,
    transfer: &Transfer<'_>,
) -> io::Result<()> {
    use std::io::{Read, Write};
    let mut source = std::fs::File::open(source)?;
    let mut target = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(target)?;
    let mut buffer = vec![0; 1024 * 1024];
    let mut copied = 0;
    loop {
        transfer.checkpoint()?;
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        target.write_all(&buffer[..count])?;
        copied += count as u64;
        transfer.update(copied);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn win32_cache_policy_changes_at_256_mib_only() {
        use windows::Win32::Storage::FileSystem::{COPYFILE_FLAGS, COPY_FILE_NO_BUFFERING};
        let threshold = 256 * 1024 * 1024;
        for size in [0, 1, threshold - 1] {
            assert_eq!(copy_flags(size), COPYFILE_FLAGS(0));
        }
        for size in [threshold, threshold + 1, u64::MAX] {
            assert_eq!(copy_flags(size), COPY_FILE_NO_BUFFERING);
        }
    }

    #[test]
    fn failed_transfer_rolls_back_only_its_own_progress() {
        let progress = Progress::default();
        progress.bytes_copied.store(100, Ordering::Relaxed);
        {
            let transfer = Transfer {
                progress: &progress,
                permit: None,
                bytes: Cell::new(0),
                committed: false,
            };
            transfer.update(12);
            transfer.update(8);
            transfer.update(16);
            assert_eq!(progress.bytes_copied.load(Ordering::Relaxed), 116);
        }
        assert_eq!(progress.bytes_copied.load(Ordering::Relaxed), 100);
        assert_eq!(progress.telemetry.transferred.load(Ordering::Relaxed), 16);
    }

    #[cfg(windows)]
    #[test]
    fn callback_aborts_in_flight_copy_on_cancellation() {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            CALLBACK_CHUNK_FINISHED, PROGRESS_CANCEL, PROGRESS_CONTINUE,
        };
        let progress = Progress::default();
        let transfer = Transfer {
            progress: &progress,
            permit: None,
            bytes: Cell::new(0),
            committed: false,
        };
        let call = || unsafe {
            callback(
                100,
                20,
                100,
                20,
                0,
                CALLBACK_CHUNK_FINISHED,
                HANDLE::default(),
                HANDLE::default(),
                (&transfer as *const Transfer<'_>).cast(),
            )
        };
        assert_eq!(call(), PROGRESS_CONTINUE);
        progress.cancel.store(true, Ordering::Relaxed);
        assert_eq!(call(), PROGRESS_CANCEL);
    }

    #[cfg(windows)]
    #[test]
    fn win32_cancellation_cleans_the_owned_partial_file() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("file.txt");
        let target = root.path().join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(&source, vec![42u8; 2 * 1024 * 1024]).unwrap();
        let progress = Progress::default();
        progress.cancel.store(true, Ordering::Relaxed);
        {
            let staged = StagingFile::new(&target).unwrap();
            let transfer = Transfer {
                progress: &progress,
                permit: None,
                bytes: Cell::new(0),
                committed: false,
            };
            assert!(copy_contents(&source, staged.path(), 2 * 1024 * 1024, &transfer).is_err());
        }
        assert_eq!(std::fs::read_dir(target).unwrap().count(), 0);
        assert_eq!(std::fs::metadata(source).unwrap().len(), 2 * 1024 * 1024);
        assert_eq!(progress.bytes_copied.load(Ordering::Relaxed), 0);
    }

    #[cfg(windows)]
    #[test]
    fn in_flight_callback_sleeps_until_resume_or_cancel() {
        use std::sync::{mpsc, Arc};
        use std::time::{Duration, Instant};
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            CALLBACK_CHUNK_FINISHED, PROGRESS_CANCEL, PROGRESS_CONTINUE,
        };
        for cancel in [false, true] {
            let progress = Arc::new(Progress::default());
            progress.pause();
            let (ready, ready_rx) = mpsc::channel();
            let (done, done_rx) = mpsc::channel();
            std::thread::scope(|scope| {
                let worker = progress.clone();
                scope.spawn(move || {
                    let _participant = worker.participant();
                    let transfer = Transfer {
                        progress: &worker,
                        permit: None,
                        bytes: Cell::new(0),
                        committed: false,
                    };
                    ready.send(()).unwrap();
                    let result = unsafe {
                        callback(
                            100,
                            20,
                            100,
                            20,
                            0,
                            CALLBACK_CHUNK_FINISHED,
                            HANDLE::default(),
                            HANDLE::default(),
                            (&transfer as *const Transfer<'_>).cast(),
                        )
                    };
                    done.send(result).unwrap();
                });
                ready_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                let deadline = Instant::now() + Duration::from_secs(3);
                while progress.pause_state() != crate::PauseState::Paused
                    && Instant::now() < deadline
                {
                    std::thread::yield_now();
                }
                let paused = progress.pause_state() == crate::PauseState::Paused;
                let blocked = done_rx.try_recv().is_err();
                if cancel {
                    progress.cancel();
                } else {
                    progress.resume();
                }
                let result = done_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                assert!(paused && blocked);
                assert_eq!(
                    result,
                    if cancel {
                        PROGRESS_CANCEL
                    } else {
                        PROGRESS_CONTINUE
                    }
                );
            });
            assert_eq!(progress.bytes_copied.load(Ordering::Relaxed), 0);
        }
    }
}
