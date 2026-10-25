//! Read-only Windows Search cache. Never extracts content or adds files to the index.
use super::{key, take_string, PropertyDisplayValue, PropertyId, PropertyValue};
use std::{
    collections::HashMap,
    path::Path,
    time::{Duration, Instant},
};
use windows::core::{w, Interface, GUID, PCWSTR};
use windows::Win32::System::{
    Com::StructuredStorage::{PropVariantToFileTime, PropVariantToUInt64, PROPVARIANT},
    Com::{
        CLSIDFromProgID, CoCreateInstance, IDispatch, CLSCTX_INPROC_SERVER, DISPATCH_FLAGS,
        DISPATCH_METHOD, DISPATCH_PROPERTYGET, DISPATCH_PROPERTYPUT, DISPPARAMS,
    },
    Ole::DISPID_PROPERTYPUT,
    Variant::{PSTF_UTC, VARIANT, VT_EMPTY, VT_NULL},
};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IPropertyDescription, PSCoerceToCanonicalValue, PSGetNameFromPropertyKey,
    PSGetPropertyDescription,
};

#[derive(Default)]
pub struct IndexedProperties {
    connection: Option<Dispatch>,
    retry_after: Option<Instant>,
    names: HashMap<PropertyId, Option<String>>,
}

impl IndexedProperties {
    /// Only return indexed values when the index's file identity matches the live listing.
    /// An unavailable service, an unindexed file/property, or stale data means normal fallback.
    pub fn read(
        &mut self,
        path: &Path,
        modified: u64,
        size: u64,
        created: Option<u64>,
        properties: &[PropertyId],
    ) -> HashMap<PropertyId, PropertyDisplayValue> {
        if modified == 0 || properties.is_empty() {
            return HashMap::new();
        }
        let columns: Vec<_> = properties
            .iter()
            .filter_map(|&id| {
                let name = self.names.entry(id).or_insert_with(|| unsafe {
                    PSGetNameFromPropertyKey(&key(id))
                        .ok()
                        .map(|name| take_string(name))
                        .filter(|name| valid_identifier(name))
                });
                name.as_ref().map(|name| (id, name.clone()))
            })
            .collect();
        if columns.is_empty() {
            return HashMap::new();
        }
        self.query(path, modified, size, created, &columns)
            .unwrap_or_default()
    }

    fn connect(&mut self) -> Option<&Dispatch> {
        if self.connection.is_none() {
            if self.retry_after.is_some_and(|retry| Instant::now() < retry) {
                return None;
            }
            self.retry_after = Some(Instant::now() + Duration::from_secs(30));
            // SAFETY: this object is owned and used only by its initialized COM worker.
            let object: IDispatch = unsafe {
                CoCreateInstance(
                    &CLSIDFromProgID(w!("ADODB.Connection")).ok()?,
                    None,
                    CLSCTX_INPROC_SERVER,
                )
                .ok()?
            };
            let connection = Dispatch(object);
            connection
                .call(
                    "ConnectionTimeout",
                    DISPATCH_PROPERTYPUT,
                    &mut [1_i32.into()],
                )
                .ok()?;
            connection
                .call("CommandTimeout", DISPATCH_PROPERTYPUT, &mut [1_i32.into()])
                .ok()?;
            connection
                .call(
                    "Open",
                    DISPATCH_METHOD,
                    &mut [
                        "Provider=Search.CollatorDSO;Extended Properties='Application=Windows';"
                            .into(),
                    ],
                )
                .ok()?;
            self.connection = Some(connection);
            self.retry_after = None;
        }
        self.connection.as_ref()
    }

    fn query(
        &mut self,
        path: &Path,
        modified: u64,
        size: u64,
        created: Option<u64>,
        columns: &[(PropertyId, String)],
    ) -> Option<HashMap<PropertyId, PropertyDisplayValue>> {
        let sql = query_text(path, columns.iter().map(|(_, name)| name.as_str()));
        let connection = self.connect()?;
        let rows = Dispatch(
            IDispatch::try_from(
                &connection
                    .call("Execute", DISPATCH_METHOD, &mut [sql.as_str().into()])
                    .ok()?,
            )
            .ok()?,
        );
        if bool::try_from(&rows.call("EOF", DISPATCH_PROPERTYGET, &mut []).ok()?).ok()? {
            return None;
        }
        let fields = Dispatch(
            IDispatch::try_from(&rows.call("Fields", DISPATCH_PROPERTYGET, &mut []).ok()?).ok()?,
        );
        let indexed_modified = timestamp(&fields.field("System.DateModified")?)?;
        let indexed_size = unsigned_value(&fields.field("System.Size")?)?;
        let indexed_created = fields
            .field("System.DateCreated")
            .and_then(|value| timestamp(&value));
        if !identity_matches(
            modified,
            size,
            created,
            indexed_modified,
            indexed_size,
            indexed_created,
        ) {
            return None;
        }
        Some(
            columns
                .iter()
                .filter_map(|(id, name)| {
                    let value = fields.field(name)?;
                    if matches!(value.vt(), VT_EMPTY | VT_NULL) {
                        return None;
                    }
                    let value = canonical_value(*id, &value)?;
                    Some((*id, value.display_value(*id)))
                })
                .collect(),
        )
    }
}

fn valid_identifier(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_'))
}

fn query_text<'a>(path: &Path, columns: impl Iterator<Item = &'a str>) -> String {
    let mut names = vec!["System.DateModified", "System.DateCreated", "System.Size"];
    for name in columns {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    format!(
        "SELECT {} FROM SystemIndex WHERE System.ItemPathDisplay = '{}'",
        names.join(", "),
        path.to_string_lossy().replace('\'', "''")
    )
}

fn timestamp(value: &VARIANT) -> Option<u64> {
    let value = PropertyValue(PROPVARIANT::try_from(value).ok()?);
    let time = unsafe { PropVariantToFileTime(&value.0, PSTF_UTC).ok()? };
    let ticks = (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    (ticks / 10_000_000).checked_sub(11_644_473_600)
}

fn unsigned_value(value: &VARIANT) -> Option<u64> {
    // ADO exposes unsigned/big integer database fields as VT_DECIMAL. Converting
    // through the registered unsigned property type avoids lossy floating-point casts.
    let mut converted = VARIANT::default();
    unsafe {
        windows::Win32::System::Variant::VariantChangeType(
            &mut converted,
            value,
            windows::Win32::System::Variant::VAR_CHANGE_FLAGS(0),
            windows::Win32::System::Variant::VT_UI8,
        )
        .ok()?;
        PropVariantToUInt64(&PROPVARIANT::try_from(&converted).ok()?).ok()
    }
}

fn canonical_value(id: PropertyId, value: &VARIANT) -> Option<PropertyValue> {
    let mut normalized = VARIANT::default();
    let value = if value.vt() == windows::Win32::System::Variant::VT_DECIMAL {
        // VariantToPropVariant does not handle ADO's decimal representation of
        // large integer columns. Convert to the property's actual registered type first.
        let mut raw = std::ptr::null_mut();
        unsafe {
            PSGetPropertyDescription(&key(id), &IPropertyDescription::IID, &mut raw).ok()?;
            let description = IPropertyDescription::from_raw(raw);
            let kind =
                windows::Win32::System::Variant::VARENUM(description.GetPropertyType().ok()?);
            windows::Win32::System::Variant::VariantChangeType(
                &mut normalized,
                value,
                windows::Win32::System::Variant::VAR_CHANGE_FLAGS(0),
                kind,
            )
            .ok()?;
        }
        &normalized
    } else {
        value
    };
    let mut value = PropertyValue(PROPVARIANT::try_from(value).ok()?);
    // Preserve UTC native dates for the shared formatter. Other types must match
    // their registered Shell schema (ADO may expose a different numeric type).
    if !matches!(
        value.0.vt(),
        windows::Win32::System::Variant::VT_DATE | windows::Win32::System::Variant::VT_FILETIME
    ) {
        unsafe {
            PSCoerceToCanonicalValue(&key(id), &mut value.0).ok()?;
        }
    }
    Some(value)
}

fn identity_matches(
    modified: u64,
    size: u64,
    created: Option<u64>,
    indexed_modified: u64,
    indexed_size: u64,
    indexed_created: Option<u64>,
) -> bool {
    modified == indexed_modified
        && size == indexed_size
        && created
            .filter(|&value| value > 0)
            .is_none_or(|value| Some(value) == indexed_created)
}

/// Minimal Automation bridge for the Windows Search OLE DB provider shipped with Windows.
struct Dispatch(IDispatch);

impl Dispatch {
    fn call(
        &self,
        name: &str,
        flags: DISPATCH_FLAGS,
        args: &mut [VARIANT],
    ) -> windows::core::Result<VARIANT> {
        let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut id = 0;
        let mut named = DISPID_PROPERTYPUT;
        args.reverse(); // Automation arguments are passed right-to-left.
        let params = DISPPARAMS {
            rgvarg: args.as_mut_ptr(),
            cArgs: args.len() as u32,
            rgdispidNamedArgs: if flags == DISPATCH_PROPERTYPUT {
                &mut named
            } else {
                std::ptr::null_mut()
            },
            cNamedArgs: u32::from(flags == DISPATCH_PROPERTYPUT),
        };
        let mut result = VARIANT::default();
        // SAFETY: all strings, argument variants, and output slots live through Invoke.
        unsafe {
            self.0
                .GetIDsOfNames(&GUID::zeroed(), &PCWSTR(name.as_ptr()), 1, 0, &mut id)?;
            self.0.Invoke(
                id,
                &GUID::zeroed(),
                0,
                flags,
                &params,
                Some(&mut result),
                None,
                None,
            )?;
        }
        Ok(result)
    }

    fn field(&self, name: &str) -> Option<VARIANT> {
        let field = Dispatch(
            IDispatch::try_from(
                &self
                    .call("Item", DISPATCH_PROPERTYGET, &mut [name.into()])
                    .ok()?,
            )
            .ok()?,
        );
        field.call("Value", DISPATCH_PROPERTYGET, &mut []).ok()
    }
}

#[cfg(test)]
#[path = "indexed_tests.rs"]
mod tests;
