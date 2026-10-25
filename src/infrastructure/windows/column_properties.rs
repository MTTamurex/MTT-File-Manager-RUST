//! Windows column catalog and read-only property access. Called only by workers.
mod grouping;
mod indexed;
use crate::domain::date_format::{date_format, DateFormat};
use crate::domain::list_columns::{ColumnId, PropertyId};
pub use grouping::{FolderGrouping, NativeCategory, PropertyGrouping, SchemaGroup};
pub use indexed::IndexedProperties;
use std::cmp::Ordering;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::{Interface, PWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Com::StructuredStorage::{
    PropVariantClear, PropVariantCompareEx, PropVariantToFileTime, PROPVARIANT, PVCF_DEFAULT,
    PVCU_DEFAULT,
};
use windows::Win32::System::Variant::{PSTF_UTC, VT_DATE, VT_FILETIME};
use windows::Win32::UI::Shell::PropertiesSystem::*;

pub const DATE_CREATED: PropertyId = PropertyId {
    fmtid: 0xb725f130_47ef_101a_a5f1_02608c9eebac,
    pid: 15,
};

#[derive(Clone, Debug)]
pub struct PropertyColumn {
    pub id: ColumnId,
    pub canonical_name: String,
    pub label: String,
}

fn key(id: PropertyId) -> PROPERTYKEY {
    PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(id.fmtid),
        pid: id.pid,
    }
}

unsafe fn take_string(value: PWSTR) -> String {
    let result = unsafe { value.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(value.as_ptr().cast())) };
    result
}

pub fn catalog() -> windows::core::Result<Vec<PropertyColumn>> {
    // SAFETY: caller initializes COM; interfaces and allocated strings stay on this thread.
    unsafe {
        let mut raw = std::ptr::null_mut();
        PSEnumeratePropertyDescriptions(PDEF_COLUMN, &IPropertyDescriptionList::IID, &mut raw)?;
        let list = IPropertyDescriptionList::from_raw(raw);
        let mut columns = Vec::new();
        for index in 0..list.GetCount()? {
            let Ok(description) = list.GetAt::<IPropertyDescription>(index) else {
                continue;
            };
            if description
                .GetViewFlags()
                .is_ok_and(|flags| flags.0 & PDVF_HIDDEN.0 != 0)
            {
                continue;
            }
            let mut property_key = PROPERTYKEY::default();
            if description.GetPropertyKey(&mut property_key).is_err() {
                continue;
            }
            let Ok(canonical_name) = description.GetCanonicalName() else {
                continue;
            };
            let canonical_name = take_string(canonical_name);
            let Ok(label) = description.GetDisplayName() else {
                continue;
            };
            let label = take_string(label);
            if label.is_empty() {
                continue;
            }
            let id = match canonical_name.as_str() {
                "System.ItemNameDisplay" => ColumnId::Name,
                "System.DateModified" => ColumnId::Property(PropertyId {
                    fmtid: property_key.fmtid.to_u128(),
                    pid: property_key.pid,
                }),
                "System.ItemTypeText" => ColumnId::Type,
                "System.Size" => ColumnId::Size,
                "System.Capacity" => ColumnId::TotalSpace,
                "System.FreeSpace" => ColumnId::FreeSpace,
                "System.Recycle.DeletedFrom" => ColumnId::OriginalLocation,
                _ => ColumnId::Property(PropertyId {
                    fmtid: property_key.fmtid.to_u128(),
                    pid: property_key.pid,
                }),
            };
            columns.push(PropertyColumn {
                id,
                canonical_name,
                label,
            });
        }
        columns.sort_by(|a, b| natord::compare_ignore_case(&a.label, &b.label));
        Ok(columns)
    }
}

/// Owns a PROPVARIANT on its originating COM thread. Never crosses a channel.
pub struct PropertyValue(PROPVARIANT);

impl From<PROPVARIANT> for PropertyValue {
    fn from(value: PROPVARIANT) -> Self {
        Self(value)
    }
}

/// Dates stay unformatted in the UI cache so preference changes need no new Shell reads.
#[derive(Clone, Debug)]
pub enum PropertyDisplayValue {
    Text(String),
    Date(u64),
}

impl PropertyDisplayValue {
    pub fn text(&self) -> String {
        self.text_with_format(date_format())
    }

    pub(crate) fn text_with_format(&self, format: DateFormat) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Date(ticks) => super::formatting::format_filetime_with_format(*ticks, format),
        }
    }
}

impl Drop for PropertyValue {
    fn drop(&mut self) {
        unsafe {
            let _ = PropVariantClear(&mut self.0);
        }
    }
}

impl PropertyValue {
    pub fn compare(&self, other: &Self) -> Ordering {
        // Windows compares the actual property types (dates, numbers, vectors…), not display text.
        unsafe { PropVariantCompareEx(&self.0, &other.0, PVCU_DEFAULT, PVCF_DEFAULT).cmp(&0) }
    }

    pub fn display_value(&self, id: PropertyId) -> PropertyDisplayValue {
        unsafe {
            if matches!(self.0.Anonymous.Anonymous.vt, VT_FILETIME | VT_DATE) {
                // Both native date representations are UTC. Conversion must not apply the
                // local time zone here: the shared application formatter does that once.
                let ticks = PropVariantToFileTime(&self.0, PSTF_UTC)
                    .map(|time| {
                        (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
                    })
                    .unwrap_or(0);
                return PropertyDisplayValue::Date(ticks);
            }
            let mut raw = std::ptr::null_mut();
            if PSGetPropertyDescription(&key(id), &IPropertyDescription::IID, &mut raw).is_err() {
                return PropertyDisplayValue::Text(String::new());
            }
            let description = IPropertyDescription::from_raw(raw);
            let text = description
                .FormatForDisplay(&self.0, PDFF_DEFAULT)
                .map(|text| take_string(text))
                .unwrap_or_default();
            PropertyDisplayValue::Text(text)
        }
    }
}

pub struct PropertyReader(IPropertyStore);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyReadError {
    Deferred,
    Unavailable,
}

impl PropertyReader {
    /// Ask only for innate/in-memory properties, without loading the file's property handler.
    /// Missing values must fall back to the normal reader, not be cached as absent.
    pub fn open_fast(path: &Path) -> Result<Self, PropertyReadError> {
        Self::open_with_flags(path, GPS_FASTPROPERTIESONLY | GPS_BESTEFFORT)
    }

    pub fn open(path: &Path) -> Result<Self, PropertyReadError> {
        Self::check_safe(path)?;
        Self::open_with_flags(path, GPS_BESTEFFORT | GPS_DELAYCREATION)
    }

    pub fn check_safe(path: &Path) -> Result<(), PropertyReadError> {
        // Avoid hydration and interference with active writers. These checks are also worker-only.
        if (crate::infrastructure::onedrive::is_cloud_sync_path(path)
            && !crate::infrastructure::onedrive::is_locally_available(path))
            // The file write-lock probe cannot open directories without backup semantics.
            // Reading directory properties does not open their children for content access.
            || (!path.is_dir() && super::file_flags::is_file_unsafe_to_read(path))
        {
            return Err(PropertyReadError::Deferred);
        }
        Ok(())
    }

    fn open_with_flags(
        path: &Path,
        flags: GETPROPERTYSTOREFLAGS,
    ) -> Result<Self, PropertyReadError> {
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: callers initialize COM; the UTF-16 buffer lives through this call.
        // Never request GPS_OPENSLOWITEM: unavailable cloud content must not be hydrated.
        unsafe {
            SHGetPropertyStoreFromParsingName(windows::core::PCWSTR(path.as_ptr()), None, flags)
                .map(Self)
                .map_err(|_| PropertyReadError::Unavailable)
        }
    }

    pub fn read(&self, id: PropertyId) -> Option<PropertyValue> {
        unsafe {
            let value = PropertyValue::from(self.0.GetValue(&key(id)).ok()?);
            // VT_EMPTY and VT_NULL are both missing, rather than zero or an empty date.
            if value.0.Anonymous.Anonymous.vt.0 <= 1 {
                None
            } else {
                Some(value)
            }
        }
    }
}

#[cfg(test)]
#[path = "column_properties_tests.rs"]
mod tests;
