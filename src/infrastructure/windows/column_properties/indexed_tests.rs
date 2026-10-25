use super::*;

#[test]
fn index_query_escapes_paths_and_deduplicates_identity_columns() {
    let sql = query_text(
        Path::new(r"C:\Example\file's video.mp4"),
        ["System.Size", "System.Media.Duration"].into_iter(),
    );
    assert!(sql.ends_with("WHERE System.ItemPathDisplay = 'C:\\Example\\file''s video.mp4'"));
    assert_eq!(sql.matches("System.Size").count(), 1);
    assert!(sql.contains("System.Media.Duration"));
}

#[test]
fn only_registered_canonical_identifiers_can_be_used_as_columns() {
    for name in [
        "System.Media.Duration",
        "System.Video.EncodingBitrate",
        "Vendor.Property_1",
    ] {
        assert!(valid_identifier(name));
    }
    for name in [
        "",
        "System.Size, other",
        "System.Size FROM SystemIndex",
        "[System.Size]",
        "name'",
    ] {
        assert!(!valid_identifier(name));
    }
}

#[test]
fn stale_or_replaced_files_cannot_use_indexed_metadata() {
    assert!(identity_matches(100, 10, Some(90), 100, 10, Some(90)));
    assert!(!identity_matches(100, 10, Some(90), 99, 10, Some(90)));
    assert!(!identity_matches(100, 10, Some(90), 100, 9, Some(90)));
    assert!(!identity_matches(100, 10, Some(90), 100, 10, Some(89)));
    assert!(!identity_matches(100, 10, Some(90), 100, 10, None));
    assert!(identity_matches(100, 10, None, 100, 10, None));
}

#[test]
fn an_unindexed_file_or_unavailable_index_returns_no_values_for_normal_fallback() {
    let _com = crate::infrastructure::windows::metadata::utils::ComGuard::new().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file.txt");
    std::fs::write(&path, b"sample data").unwrap();
    let mut reader = IndexedProperties::default();
    let duration = PropertyId {
        fmtid: 0x64440490_4c8b_11d1_8b70_080036b11a03,
        pid: 3,
    };
    assert!(reader.read(&path, 1, 11, None, &[duration]).is_empty());
}

#[test]
#[ignore = "requires an enabled Windows Search service"]
fn automation_bridge_can_query_the_native_index_when_available() {
    let _com = crate::infrastructure::windows::metadata::utils::ComGuard::new().unwrap();
    let mut reader = IndexedProperties::default();
    // Validate the native bridge without reading or exposing indexed user data.
    let connection = reader.connect().expect("Windows Search connection");
    let result = connection.call("Execute", DISPATCH_METHOD, &mut [
            "SELECT System.Size FROM SystemIndex WHERE System.ItemPathDisplay = 'Z:\\Example\\nonexistent.txt'".into()
        ]).unwrap();
    let rows = Dispatch(IDispatch::try_from(&result).unwrap());
    assert!(bool::try_from(&rows.call("EOF", DISPATCH_PROPERTYGET, &mut []).unwrap()).unwrap());
}

#[test]
fn automation_bridge_reads_typed_properties_from_a_native_recordset() {
    let _com = crate::infrastructure::windows::metadata::utils::ComGuard::new().unwrap();
    let object: IDispatch = unsafe {
        CoCreateInstance(
            &CLSIDFromProgID(w!("ADODB.Recordset")).unwrap(),
            None,
            CLSCTX_INPROC_SERVER,
        )
        .unwrap()
    };
    let rows = Dispatch(object);
    let fields = Dispatch(
        IDispatch::try_from(&rows.call("Fields", DISPATCH_PROPERTYGET, &mut []).unwrap()).unwrap(),
    );
    // adBigInt = 20, adDate = 7. This is an in-memory fixture, not an index modification.
    for (name, kind) in [
        ("System.Size", 20_i32),
        ("System.DateModified", 7_i32),
        ("System.Media.Duration", 20_i32),
    ] {
        fields
            .call("Append", DISPATCH_METHOD, &mut [name.into(), kind.into()])
            .unwrap();
    }
    rows.call("Open", DISPATCH_METHOD, &mut []).unwrap();
    rows.call("AddNew", DISPATCH_METHOD, &mut []).unwrap();
    let mut date = VARIANT::default();
    unsafe {
        (*date.Anonymous.Anonymous).vt = windows::Win32::System::Variant::VT_DATE;
        (*date.Anonymous.Anonymous).Anonymous.date = 46062.5;
    }
    for (name, value) in [
        ("System.Size", VARIANT::from(9_007_199_254_740_993_i64)),
        ("System.DateModified", date),
        ("System.Media.Duration", VARIANT::from(10_000_000_i64)),
    ] {
        let field = Dispatch(
            IDispatch::try_from(
                &fields
                    .call("Item", DISPATCH_PROPERTYGET, &mut [name.into()])
                    .unwrap(),
            )
            .unwrap(),
        );
        field
            .call("Value", DISPATCH_PROPERTYPUT, &mut [value])
            .unwrap();
    }
    rows.call("Update", DISPATCH_METHOD, &mut []).unwrap();
    rows.call("MoveFirst", DISPATCH_METHOD, &mut []).unwrap();
    assert_eq!(
        unsigned_value(&fields.field("System.Size").unwrap()).unwrap(),
        9_007_199_254_740_993
    );
    assert!(timestamp(&fields.field("System.DateModified").unwrap()).is_some());
    let duration = PropertyId {
        fmtid: 0x64440490_4c8b_11d1_8b70_080036b11a03,
        pid: 3,
    };
    let value = canonical_value(duration, &fields.field("System.Media.Duration").unwrap()).unwrap();
    assert_eq!(
        unsafe { PropVariantToUInt64(&value.0).unwrap() },
        10_000_000
    );
    assert!(!value.display_value(duration).text().is_empty());
}
