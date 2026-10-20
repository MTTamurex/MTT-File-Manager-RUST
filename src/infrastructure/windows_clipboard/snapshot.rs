//! Read-only, coherent file clipboard snapshots. Call only from a worker thread.
use super::{clipboard_sequence_number, ClipboardFileOp, DROPEFFECT_COPY, DROPEFFECT_MOVE};
use clipboard_win::{formats, Clipboard, Getter};
use std::path::PathBuf;
use windows::core::w;
use windows::Win32::Foundation::HGLOBAL;
use windows::Win32::System::DataExchange::{GetClipboardData, RegisterClipboardFormatW};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};

const CF_HDROP: u32 = 15;

pub(crate) struct FileSnapshot {
    pub sequence: u32,
    pub paths: Vec<PathBuf>,
    pub operation: ClipboardFileOp,
}

pub(crate) fn read(expected: u32) -> Option<FileSnapshot> {
    let _clipboard = Clipboard::new_attempts(3).ok()?;
    if clipboard_sequence_number() != Some(expected) {
        return None;
    }
    let mut paths: Vec<String> = Vec::new();
    // Absence of CF_HDROP is a valid empty snapshot, not a transient failure.
    if clipboard_win::is_format_avail(CF_HDROP) {
        formats::FileList.read_clipboard(&mut paths).ok()?;
    }
    // Do not request unrelated (potentially delay-rendered) data for text-only
    // or empty clipboards.
    let effect = if paths.is_empty() {
        DROPEFFECT_COPY
    } else {
        unsafe { preferred_effect() }
    };
    // Delay-rendering can change the sequence while reading the payload.
    if clipboard_sequence_number() != Some(expected) {
        return None;
    }
    Some(FileSnapshot {
        sequence: expected,
        paths: paths.into_iter().map(PathBuf::from).collect(),
        operation: if effect & DROPEFFECT_MOVE != 0 {
            ClipboardFileOp::Move
        } else {
            ClipboardFileOp::Copy
        },
    })
}

// The caller owns the clipboard lock for both the file list and the effect.
unsafe fn preferred_effect() -> u32 {
    let format = RegisterClipboardFormatW(w!("Preferred DropEffect"));
    if format == 0 {
        return DROPEFFECT_COPY;
    }
    let Ok(handle) = GetClipboardData(format) else {
        return DROPEFFECT_COPY;
    };
    let memory = HGLOBAL(handle.0);
    if GlobalSize(memory) < std::mem::size_of::<u32>() {
        return DROPEFFECT_COPY;
    }
    let pointer = GlobalLock(memory);
    if pointer.is_null() {
        return DROPEFFECT_COPY;
    }
    let effect = pointer.cast::<u32>().read_unaligned();
    let _ = GlobalUnlock(memory);
    effect
}
