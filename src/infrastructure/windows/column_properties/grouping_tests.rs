use super::*;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;

fn band(group: Option<SchemaGroup>) -> (u32, String) {
    match group.expect("registered property grouping") {
        SchemaGroup::Range { index, label } => (index, label),
        _ => panic!("Windows must supply its registered enum ranges"),
    }
}

#[test]
fn duration_uses_windows_ranges_instead_of_a_group_for_each_exact_value() {
    let _com = super::super::super::metadata::utils::ComGuard::new().unwrap();
    let duration = PropertyId {
        fmtid: 0x64440490_4c8b_11d1_8b70_080036b11a03,
        pid: 3,
    };
    let grouping = PropertyGrouping::new(duration).unwrap();
    assert_eq!(
        unsafe { grouping.description.GetGroupingRange().unwrap() },
        PDGR_ENUMERATED
    );
    let seconds = |seconds: u64| PropertyValue(PROPVARIANT::from(seconds * 10_000_000));
    let first = band(grouping.classify(&seconds(10)));
    let same = band(grouping.classify(&seconds(30)));
    let longer = band(grouping.classify(&seconds(600)));
    assert_eq!(first, same);
    assert_ne!(first.0, longer.0);
    assert!(!first.1.is_empty());
}

#[test]
fn bitrate_uses_the_property_schema_without_hardcoded_thresholds() {
    let _com = super::super::super::metadata::utils::ComGuard::new().unwrap();
    let bitrate = PropertyId {
        fmtid: 0x64440490_4c8b_11d1_8b70_080036b11a03,
        pid: 4,
    };
    let grouping = PropertyGrouping::new(bitrate).unwrap();
    assert_eq!(
        unsafe { grouping.description.GetGroupingRange().unwrap() },
        PDGR_ENUMERATED
    );
    let first = band(grouping.classify(&PropertyValue(PROPVARIANT::from(1_000_u32))));
    let same = band(grouping.classify(&PropertyValue(PROPVARIANT::from(10_000_u32))));
    let higher = band(grouping.classify(&PropertyValue(PROPVARIANT::from(200_000_u32))));
    assert_eq!(first, same);
    assert_ne!(first.0, higher.0);
}

#[test]
fn video_data_rate_does_not_inherit_unrelated_audio_bands_when_windows_provides_no_enum_ranges() {
    let _com = super::super::super::metadata::utils::ComGuard::new().unwrap();
    let bitrate = PropertyId {
        fmtid: 0x64440491_4c8b_11d1_8b70_080036b11a03,
        pid: 8,
    };
    let grouping = PropertyGrouping::new(bitrate).unwrap();
    assert_eq!(
        unsafe { grouping.description.GetGroupingRange().unwrap() },
        PDGR_DYNAMIC
    );
    let first = grouping.classify(&PropertyValue(PROPVARIANT::from(100_000_u32)));
    let higher = grouping.classify(&PropertyValue(PROPVARIANT::from(200_000_u32)));
    match (first, higher) {
        (Some(SchemaGroup::Text(first)), Some(SchemaGroup::Text(higher))) => {
            assert_ne!(first, higher)
        }
        _ => panic!("dynamic video properties must not borrow unrelated audio bitrate thresholds"),
    }
}

#[test]
fn folder_category_provider_preserves_explorer_category_comparisons() {
    let com = super::super::super::com_scope::ComScope::sta();
    assert!(com.is_initialized());
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.bin");
    let second = directory.path().join("second.bin");
    let larger = directory.path().join("larger.bin");
    std::fs::write(&first, [0_u8; 100]).unwrap();
    std::fs::write(&second, [0_u8; 200]).unwrap();
    std::fs::File::create(&larger)
        .unwrap()
        .set_len(50_000)
        .unwrap();
    let size = PropertyId {
        fmtid: 0xb725f130_47ef_101a_a5f1_02608c9eebac,
        pid: 12,
    };
    let grouping = match FolderGrouping::open(directory.path(), size) {
        Ok(grouping) => grouping,
        Err(error) if error.code() == windows::Win32::Foundation::E_NOINTERFACE => {
            // Category providers are optional on filesystem folders. The schema still
            // explicitly requests size ranges, not a separate group for every byte count.
            let schema = PropertyGrouping::new(size).unwrap();
            assert!(matches!(
                schema.classify(&PropertyValue(PROPVARIANT::from(100_u64))),
                Some(SchemaGroup::Size(100))
            ));
            return;
        }
        Err(error) => panic!("native category provider failed: {error}"),
    };
    let a = grouping.category(&first).unwrap();
    let b = grouping.category(&second).unwrap();
    let c = grouping.category(&larger).unwrap();
    assert!(!a.label.is_empty());
    assert_eq!(a.id, b.id);
    assert_eq!(grouping.compare(a.id, b.id), Some(Ordering::Equal));
    assert_eq!(grouping.compare(a.id, c.id), Some(Ordering::Less));
    assert_eq!(grouping.compare(c.id, a.id), Some(Ordering::Greater));
}
