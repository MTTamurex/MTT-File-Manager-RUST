//! Explorer category providers and registered property grouping rules. COM worker only.
use super::{key, take_string, PropertyId, PropertyValue};
use std::{cmp::Ordering, os::windows::ffi::OsStrExt, path::Path};
use windows::core::{Interface, PCWSTR};
use windows::Win32::{
    Foundation::HWND,
    System::Com::{
        CoTaskMemFree,
        StructuredStorage::{PropVariantToFileTime, PropVariantToUInt64},
    },
    System::Variant::PSTF_UTC,
    UI::Shell::{
        BHID_SFObject, Common::ITEMIDLIST, ICategorizer, ICategoryProvider, IShellFolder,
        IShellItem, PropertiesSystem::*, SHCreateItemFromParsingName, CATEGORY_INFO,
        CATSORT_DEFAULT,
    },
};

pub struct NativeCategory {
    pub id: u32,
    pub label: String,
}

pub struct FolderGrouping {
    folder: IShellFolder,
    categorizer: ICategorizer,
}

impl FolderGrouping {
    pub fn open(parent: &Path, property: PropertyId) -> windows::core::Result<Self> {
        let parent: Vec<_> = parent.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: caller initializes COM; interfaces and PIDLs stay on this worker.
        unsafe {
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(parent.as_ptr()), None)?;
            let folder: IShellFolder = item.BindToHandler(None, &BHID_SFObject)?;
            let provider: ICategoryProvider = folder
                .cast()
                .or_else(|_| folder.CreateViewObject(HWND::default()))?;
            provider.CanCategorizeOnSCID(&key(property))?;
            let kind = provider.GetCategoryForSCID(&key(property))?;
            let categorizer = provider.CreateCategory(&kind)?;
            Ok(Self {
                folder,
                categorizer,
            })
        }
    }

    pub fn category(&self, path: &Path) -> windows::core::Result<NativeCategory> {
        let name: Vec<_> = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut pidl = ChildPidl(std::ptr::null_mut());
        let mut attributes = 0;
        let mut id = 0;
        let mut info = CATEGORY_INFO::default();
        unsafe {
            self.folder.ParseDisplayName(
                HWND::default(),
                None,
                PCWSTR(name.as_ptr()),
                None,
                &mut pidl.0,
                &mut attributes,
            )?;
            self.categorizer
                .GetCategory(1, &pidl.0.cast_const(), &mut id)?;
            self.categorizer.GetCategoryInfo(id, &mut info)?;
        }
        let length = info
            .wszName
            .iter()
            .position(|&character| character == 0)
            .unwrap_or(info.wszName.len());
        Ok(NativeCategory {
            id,
            label: String::from_utf16_lossy(&info.wszName[..length]),
        })
    }

    pub fn compare(&self, a: u32, b: u32) -> Option<Ordering> {
        // CompareCategory packs a signed comparison into a successful HRESULT. The generated
        // Result<()> wrapper discards that value, so preserve it through the native vtable.
        let result = unsafe {
            (self.categorizer.vtable().CompareCategory)(
                self.categorizer.as_raw(),
                CATSORT_DEFAULT,
                a,
                b,
            )
        };
        result.is_ok().then(|| (result.0 as u16 as i16).cmp(&0))
    }
}

struct ChildPidl(*mut ITEMIDLIST);
impl Drop for ChildPidl {
    fn drop(&mut self) {
        unsafe {
            CoTaskMemFree(Some(self.0.cast()));
        }
    }
}

pub enum SchemaGroup {
    Range { index: u32, label: String },
    Date(u64),
    Size(u64),
    Alphabetic(String),
    Text(String),
}

pub struct PropertyGrouping {
    property: PropertyId,
    description: IPropertyDescription,
    enumerated: Option<IPropertyEnumTypeList>,
}

impl PropertyGrouping {
    pub fn new(property: PropertyId) -> windows::core::Result<Self> {
        let mut raw = std::ptr::null_mut();
        unsafe {
            PSGetPropertyDescription(&key(property), &IPropertyDescription::IID, &mut raw)?;
            let description = IPropertyDescription::from_raw(raw);
            let enumerated = description.GetEnumTypeList().ok();
            Ok(Self {
                property,
                description,
                enumerated,
            })
        }
    }

    pub fn classify(&self, value: &PropertyValue) -> Option<SchemaGroup> {
        unsafe {
            let range = self.description.GetGroupingRange().ok()?;
            if let Some(list) = self
                .enumerated
                .as_ref()
                .filter(|_| range == PDGR_ENUMERATED)
            {
                if let Ok(index) = list.FindMatchingIndex(&value.0) {
                    let entry: IPropertyEnumType = list.GetAt(index).ok()?;
                    let label = take_string(entry.GetDisplayText().ok()?);
                    if !label.is_empty() {
                        return Some(SchemaGroup::Range { index, label });
                    }
                }
            }
            if range == PDGR_SIZE {
                return PropVariantToUInt64(&value.0).ok().map(SchemaGroup::Size);
            }
            if range == PDGR_DATE {
                let time = PropVariantToFileTime(&value.0, PSTF_UTC).ok()?;
                let ticks = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
                return (ticks / 10_000_000)
                    .checked_sub(11_644_473_600)
                    .map(SchemaGroup::Date);
            }
            let text = value.display_value(self.property).text();
            if text.is_empty() {
                return None;
            }
            // Dynamic ranges may be supplied by a folder's categorizer rather than
            // its public property schema. If absent, keep the displayed values distinct;
            // never invent thresholds or reuse another property's unrelated ranges.
            Some(if range == PDGR_ALPHANUMERIC {
                SchemaGroup::Alphabetic(text)
            } else {
                SchemaGroup::Text(text)
            })
        }
    }
}

#[cfg(test)]
#[path = "grouping_tests.rs"]
mod tests;
