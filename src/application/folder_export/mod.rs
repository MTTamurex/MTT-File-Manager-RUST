//! Read-only folder listing export. Files are committed only after successful generation.
mod collect;
mod render;
mod sizes;
#[cfg(test)]
mod tests;
pub(crate) mod worker;

use crate::domain::{
    date_format::DateFormat,
    file_entry::{FoldersPosition, SortMode},
    list_columns::ColumnId,
};
use rustc_hash::FxHashMap;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    Csv,
    Tsv,
    Text,
    Html,
}
impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Text => "txt",
            Self::Html => "html",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Contents {
    Both,
    Files,
    Folders,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CsvSeparator {
    Comma,
    Semicolon,
}
impl CsvSeparator {
    pub fn text(self) -> &'static str {
        match self {
            Self::Comma => ",",
            Self::Semicolon => ";",
        }
    }
    pub fn regional_default() -> Self {
        use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_SLIST};
        let mut buffer = [0u16; 16];
        // SAFETY: null selects the user's locale; the API writes within the buffer.
        let length = unsafe {
            GetLocaleInfoEx(
                windows::core::PCWSTR::null(),
                LOCALE_SLIST,
                Some(&mut buffer),
            )
        };
        if length > 1 && String::from_utf16_lossy(&buffer[..length as usize - 1]) == ";" {
            Self::Semicolon
        } else {
            Self::Comma
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Field {
    Column(ColumnId),
    RelativePath,
}

#[derive(Clone)]
pub(crate) struct ExportColumn {
    pub field: Field,
    pub label: String,
    pub enabled: bool,
}

#[derive(Clone)]
pub(crate) struct Options {
    pub root: PathBuf,
    /// None means enumerate the folder directly, without using UI filters.
    pub selection: Option<Vec<PathBuf>>,
    pub recursive: bool,
    pub contents: Contents,
    pub format: Format,
    pub csv_separator: CsvSeparator,
    pub calculate_sizes: bool,
    /// Completed sizes keyed by `normalize_tag_path_key`, including Windows path aliases.
    pub folder_sizes: std::collections::HashMap<String, u64>,
    pub tree: bool,
    pub columns: Vec<ExportColumn>,
    pub sort: SortMode,
    pub descending: bool,
    pub folders: FoldersPosition,
    pub date_format: DateFormat,
    pub tags: Arc<FxHashMap<String, Vec<i64>>>,
    pub tag_names: FxHashMap<i64, String>,
}

pub(crate) struct Row {
    pub depth: usize,
    pub directory: bool,
    pub included: bool,
    pub name: String,
    pub cells: Vec<String>,
    pub size: Option<u64>,
    pub traversable: bool,
    pub complete: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExportError {
    Cancelled,
    Read,
    Write,
    TooLarge,
    Exists,
    Worker,
}

#[derive(Default)]
pub(crate) struct Progress {
    pub cancelled: Arc<AtomicBool>,
    pub calculating_sizes: AtomicBool,
    pub items: AtomicUsize,
}
impl Progress {
    pub fn check(&self) -> Result<(), ExportError> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(ExportError::Cancelled)
        } else {
            Ok(())
        }
    }
}

pub(crate) struct Listing {
    pub rows: Vec<Row>,
    pub skipped: usize,
}
