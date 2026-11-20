//! Snapshot Shell identity without comparing a deleted namespace item.
use windows::core::{IUnknown, Interface};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{IShellItem, SIGDN_DESKTOPABSOLUTEPARSING};

pub(super) struct Identity {
    unknown: IUnknown,
    parsing_name: Option<String>,
}

impl Identity {
    pub(super) fn new(item: &IShellItem) -> Option<Self> {
        let unknown = item.cast().ok()?;
        let parsing_name = unsafe {
            item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)
                .ok()
                .and_then(|name| {
                    let value = name.to_string().ok().filter(|name| !name.is_empty());
                    CoTaskMemFree(Some(name.as_ptr().cast()));
                    value
                })
        };
        Some(Self {
            unknown,
            parsing_name,
        })
    }

    pub(super) fn matches(&self, other: &Self) -> bool {
        self.unknown == other.unknown
            || self
                .parsing_name
                .as_ref()
                .zip(other.parsing_name.as_ref())
                .is_some_and(|(left, right)| left == right)
    }
}
