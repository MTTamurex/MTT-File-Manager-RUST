use super::*;
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::System::Com::StructuredStorage::InitPropVariantFromFileTime;
use windows::Win32::System::Variant::{SystemTimeToVariantTime, VT_UI8};

const CREATED: PropertyId = DATE_CREATED;
const TIMESTAMP: u64 = 1_770_638_400;

fn filetime_value(ticks: u64) -> PropertyValue {
    let time = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    PropertyValue(unsafe { InitPropVariantFromFileTime(&time).unwrap() })
}

fn automation_value(date: f64) -> PropertyValue {
    let mut raw = PROPVARIANT::default();
    // SAFETY: VT_DATE selects the inline OLE date value and owns no pointers.
    unsafe {
        (*raw.Anonymous.Anonymous).vt = VT_DATE;
        (*raw.Anonymous.Anonymous).Anonymous.date = date;
    }
    PropertyValue(raw)
}

fn automation_time() -> f64 {
    let time = SYSTEMTIME {
        wYear: 2026,
        wMonth: 2,
        wDay: 9,
        wHour: 12,
        ..Default::default()
    };
    let mut date = 0.0;
    assert_ne!(unsafe { SystemTimeToVariantTime(&time, &mut date) }, 0);
    date
}

#[test]
fn catalog_uses_registered_column_descriptions() {
    let _com = super::super::metadata::utils::ComGuard::new().unwrap();
    let columns = catalog().unwrap();
    assert!(columns
        .iter()
        .any(|column| column.canonical_name == "System.Size"));
    assert_eq!(
        columns
            .iter()
            .find(|column| column.canonical_name == "System.DateCreated")
            .map(|column| column.id),
        Some(ColumnId::Property(DATE_CREATED))
    );
    assert!(columns.iter().all(|column| !column.label.is_empty()));
}

#[test]
fn numeric_values_compare_numerically_instead_of_using_display_strings() {
    fn number(number: u64) -> PropertyValue {
        let mut value = PROPVARIANT::default();
        // SAFETY: VT_UI8 selects the unsigned 64-bit member and owns no pointers.
        unsafe {
            (*value.Anonymous.Anonymous).vt = VT_UI8;
            (*value.Anonymous.Anonymous).Anonymous.uhVal = number;
        }
        PropertyValue(value)
    }
    assert!(number(2).compare(&number(10)).is_lt());
    assert!(number(u64::MAX - 1).compare(&number(u64::MAX)).is_lt());
}

#[test]
fn property_store_reads_and_formats_a_local_file_without_writing_it() {
    let _com = super::super::metadata::utils::ComGuard::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file.txt");
    std::fs::write(&path, b"sample data").unwrap();
    let size = PropertyId {
        fmtid: CREATED.fmtid,
        pid: 12,
    };
    let reader = PropertyReader::open(&path).unwrap();
    let value = reader.read(size).expect("local size property");
    assert!(!value.display_value(size).text().is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), b"sample data");
}

#[test]
fn fast_property_store_reads_innate_metadata_without_the_content_handler() {
    let _com = super::super::metadata::utils::ComGuard::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file.txt");
    std::fs::write(&path, b"sample data").unwrap();
    let size = PropertyId {
        fmtid: CREATED.fmtid,
        pid: 12,
    };
    let reader = PropertyReader::open_fast(&path).unwrap();
    assert!(!reader
        .read(size)
        .unwrap()
        .display_value(size)
        .text()
        .is_empty());
    assert!(reader.read(PropertyId { fmtid: 0, pid: 999 }).is_none());
    assert_eq!(std::fs::read(&path).unwrap(), b"sample data");
}

#[test]
fn date_created_is_readable_and_formatted_for_directories() {
    let _com = super::super::metadata::utils::ComGuard::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("folder");
    std::fs::create_dir(&path).unwrap();
    let reader = PropertyReader::open(&path)
        .expect("directory properties must not be treated as a locked file");
    let value = reader.read(CREATED).expect("directory creation date");
    assert!(!value.display_value(CREATED).text().is_empty());
    // Keep the native date type so sorting does not fall back to formatted text.
    assert_eq!(unsafe { value.0.Anonymous.Anonymous.vt }, VT_FILETIME);
}

#[test]
fn files_open_for_writing_still_defer_property_reads() {
    use std::os::windows::fs::OpenOptionsExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file.txt");
    std::fs::write(&path, b"sample data").unwrap();
    let _writer = std::fs::OpenOptions::new()
        .write(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    assert!(matches!(
        PropertyReader::open(&path),
        Err(PropertyReadError::Deferred)
    ));
}

#[test]
fn native_dates_use_the_same_local_format_as_last_modified() {
    let native = filetime_value((TIMESTAMP + 11_644_473_600) * 10_000_000);
    let display = native.display_value(CREATED);
    assert!(matches!(display, PropertyDisplayValue::Date(_)));
    for format in [
        DateFormat::DayMonthYear,
        DateFormat::MonthDayYear,
        DateFormat::YearMonthDay,
    ] {
        assert_eq!(
            display.text_with_format(format),
            super::super::formatting::format_date_with_format(TIMESTAMP, format)
        );
    }
}

#[test]
fn automation_dates_use_the_app_format_instead_of_shell_display_text() {
    let native = automation_value(automation_time());
    let display = native.display_value(CREATED);
    assert!(matches!(display, PropertyDisplayValue::Date(_)));
    for format in [
        DateFormat::DayMonthYear,
        DateFormat::MonthDayYear,
        DateFormat::YearMonthDay,
    ] {
        assert_eq!(
            display.text_with_format(format),
            super::super::formatting::format_date_with_format(TIMESTAMP, format)
        );
    }
}

#[test]
fn invalid_automation_dates_are_unknown_instead_of_cached_localized_text() {
    let display = automation_value(f64::NAN).display_value(CREATED);
    assert!(matches!(display, PropertyDisplayValue::Date(0)));
    assert_eq!(
        display.text_with_format(DateFormat::YearMonthDay),
        rust_i18n::t!("file_info.unknown_date").to_string()
    );
}

#[test]
fn formatting_dates_does_not_change_their_chronological_sort_order() {
    let earlier = filetime_value((TIMESTAMP + 11_644_473_600) * 10_000_000);
    let later = automation_value(automation_time() + 1.0);
    for format in [
        DateFormat::DayMonthYear,
        DateFormat::MonthDayYear,
        DateFormat::YearMonthDay,
    ] {
        earlier.display_value(CREATED).text_with_format(format);
        later.display_value(CREATED).text_with_format(format);
        assert!(earlier.compare(&later).is_lt());
        assert!(later.compare(&earlier).is_gt());
    }
}
