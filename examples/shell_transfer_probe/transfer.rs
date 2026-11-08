use super::input::CopyInput;
use super::observation::{Observation, Signal};
use super::observed_data_object;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, Prefix};
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{HWND, POINTL};
use windows::Win32::System::Com::{CoTaskMemFree, IDataObject};
use windows::Win32::System::Ole::{IDropTarget, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_SCROLL};
use windows::Win32::System::SystemServices::{MK_CONTROL, MK_LBUTTON};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, IDataObjectAsyncCapability, IShellFolder, SHBindToParent,
    SHCreateShellItemArrayFromIDLists, SHParseDisplayName,
};

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;

struct Pidl(*mut ITEMIDLIST);

fn shell_parsing_name(path: &Path) -> Vec<u16> {
    // canonicalize() produces verbatim paths on Windows. Filesystem validation
    // keeps those paths, but the Shell parser needs ordinary DOS/UNC names.
    // Convert only recognized prefixes and preserve the original UTF-16 units.
    let encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
    let mut wide = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(_) => encoded[4..].to_vec(),
            Prefix::VerbatimUNC(_, _) => {
                [r"\\".encode_utf16().collect(), encoded[8..].to_vec()].concat()
            }
            _ => encoded,
        },
        _ => encoded,
    };
    wide.push(0);
    wide
}

impl Pidl {
    fn parse(path: &Path) -> Result<Self, String> {
        let wide = shell_parsing_name(path);
        let mut pidl = Self(std::ptr::null_mut());
        unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl.0, 0, None) }
            .map_err(|error| format!("SHParseDisplayName: {}", error.code()))?;
        if pidl.0.is_null() {
            return Err("SHParseDisplayName returned a null PIDL".into());
        }
        Ok(pidl)
    }
}

impl Drop for Pidl {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.cast())) };
    }
}

pub struct CopySession {
    target: IDropTarget,
    data: IDataObject,
    observation: Option<Observation>,
}

impl CopySession {
    pub fn prepare(input: &CopyInput, owner: HWND) -> Result<Self, String> {
        let sources = input
            .sources
            .iter()
            .enumerate()
            .map(|(index, path)| {
                Pidl::parse(path).map_err(|error| format!("Source item {}: {error}", index + 1))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let raw_sources: Vec<*const ITEMIDLIST> =
            sources.iter().map(|pidl| pidl.0.cast_const()).collect();
        unsafe {
            let items = SHCreateShellItemArrayFromIDLists(&raw_sources)
                .map_err(|error| format!("SHCreateShellItemArrayFromIDLists: {}", error.code()))?;
            let data: IDataObject = items
                .BindToHandler(None, &BHID_DataObject)
                .map_err(|error| format!("BHID_DataObject: {}", error.code()))?;
            if let Ok(async_data) = data.cast::<IDataObjectAsyncCapability>() {
                async_data
                    .SetAsyncMode(false)
                    .map_err(|error| format!("SetAsyncMode(false): {}", error.code()))?;
            }
            let observation = input.observe.then(Observation::new);
            let data = if let Some(observation) = &observation {
                observed_data_object::create(data, observation.clone())
                    .map_err(|error| format!("Create diagnostic IDataObject: {}", error.code()))?
            } else {
                data
            };
            let destination = Pidl::parse(&input.destination)
                .map_err(|error| format!("Destination folder: {error}"))?;
            let mut child = std::ptr::null_mut();
            let parent: IShellFolder = SHBindToParent(destination.0, Some(&mut child))
                .map_err(|error| format!("SHBindToParent: {}", error.code()))?;
            let target: IDropTarget = parent
                .GetUIObjectOf(owner, &[child.cast_const()], None)
                .map_err(|error| format!("GetUIObjectOf(IDropTarget): {}", error.code()))?;
            println!("Prepared Shell IDataObject and destination IDropTarget.");
            Ok(Self {
                target,
                data,
                observation,
            })
        }
    }

    pub fn print_observation_summary(&self) {
        if let Some(observation) = &self.observation {
            observation.summary();
        }
    }

    pub fn dispatch(&self) -> Result<DROPEFFECT, String> {
        let point = POINTL { x: 0, y: 0 };
        let mut effect = DROPEFFECT_COPY;
        unsafe {
            println!("Calling DragEnter (left button + Ctrl, COPY only).");
            if let Err(error) =
                self.target
                    .DragEnter(&self.data, MK_LBUTTON | MK_CONTROL, point, &mut effect)
            {
                let _ = self.target.DragLeave();
                return Err(format!("DragEnter: {}", error.code()));
            }
            println!("DragEnter accepted effect: {effect:?}.");
            if DROPEFFECT(effect.0 & !DROPEFFECT_SCROLL.0) != DROPEFFECT_COPY {
                let _ = self.target.DragLeave();
                return Err("Destination did not accept COPY; Drop was not called".into());
            }
            effect = DROPEFFECT_COPY;
            println!("Calling Drop (Ctrl, COPY only). No fallback will be attempted.");
            let result = self.target.Drop(&self.data, MK_CONTROL, point, &mut effect);
            if let Some(observation) = &self.observation {
                observation.record(Signal::DropReturned);
            }
            result
                .map_err(|error| format!("Drop: {}; transfer outcome is unknown", error.code()))?;
        }
        Ok(effect)
    }
}
