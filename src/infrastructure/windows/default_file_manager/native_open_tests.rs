use super::*;
use crate::infrastructure::windows::default_file_manager::{legacy_menu, native_open};

fn open_key(class: &str) -> String {
    format!("{}\\open", shell_key(class))
}

#[test]
fn canonical_open_is_the_only_registered_opening_verb() {
    let mut reg = MemoryRegistry::default();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    for class in CLASSES.into_iter().chain([NETWORK_LOCATION_CLASS]) {
        assert_eq!(
            reg.read(&shell_key(class), "").unwrap().as_deref(),
            Some("open")
        );
        let command = if class == NETWORK_LOCATION_CLASS {
            network_location_command(Path::new(EXE))
        } else {
            folder_command(Path::new(EXE))
        };
        assert!(native_open::is_active(&reg, class, &command));
        assert_eq!(
            reg.read(&open_key(class), "Position").unwrap().as_deref(),
            Some("Top")
        );
        assert_eq!(reg.read(&open_key(class), "").unwrap(), None);
        let menu = format!("{}\\{VERB}", shell_key(class));
        assert_eq!(reg.read(&menu, "ProgrammaticAccessOnly").unwrap(), None);
        assert_eq!(reg.read(&menu, "").unwrap(), None);
    }
    assert!(!reg
        .0
        .keys()
        .any(|(key, _)| key.starts_with("Software\\Classes\\Folder\\")));
}

#[test]
fn repeated_enable_restores_native_open_values_exactly() {
    for prior in [None, Some(""), Some("previous value")] {
        let mut reg = MemoryRegistry::default();
        if let Some(prior) = prior {
            for class in CLASSES.into_iter().chain([NETWORK_LOCATION_CLASS]) {
                let open = open_key(class);
                reg.write(&format!("{open}\\command"), "", prior).unwrap();
                reg.write(&format!("{open}\\command"), "DelegateExecute", prior)
                    .unwrap();
                reg.write(&open, "Icon", prior).unwrap();
                reg.write(&open, "MultiSelectModel", prior).unwrap();
                reg.write(&open, "Position", prior).unwrap();
                reg.write(&shell_key(class), "", "open").unwrap();
            }
        }
        let before = reg.0.clone();
        apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
        apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
        apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
        assert_eq!(reg.0, before);
    }
}

#[test]
fn migration_of_the_old_default_preserves_the_original_association_backup() {
    let mut reg = MemoryRegistry::default();
    for class in CLASSES.into_iter().chain([NETWORK_LOCATION_CLASS]) {
        let shell = shell_key(class);
        let verb = format!("{shell}\\{VERB}");
        let command = if class == NETWORK_LOCATION_CLASS {
            network_location_command(Path::new(EXE))
        } else {
            folder_command(Path::new(EXE))
        };
        reg.write(&shell, "", VERB).unwrap();
        reg.write(&format!("{verb}\\command"), "", &command)
            .unwrap();
        reg.write(&verb, "", "Abrir no MTT").unwrap();
        reg.write(&verb, "ProgrammaticAccessOnly", "").unwrap();
        reg.write(&state_key(class), "HadDefault", "1").unwrap();
        reg.write(&state_key(class), "PreviousDefault", "original default")
            .unwrap();
    }
    assert!(legacy_menu::has_owned_entries(&reg, Path::new(EXE)));
    apply(&mut reg, RegistryAction::UpgradeContextMenu, Path::new(EXE)).unwrap();
    for class in CLASSES.into_iter().chain([NETWORK_LOCATION_CLASS]) {
        assert_eq!(
            reg.read(&shell_key(class), "").unwrap().as_deref(),
            Some("open")
        );
        assert_eq!(
            reg.read(&state_key(class), "PreviousDefault")
                .unwrap()
                .as_deref(),
            Some("original default")
        );
    }
    assert!(!legacy_menu::has_owned_entries(&reg, Path::new(EXE)));
    let migrated = reg.0.clone();
    apply(&mut reg, RegistryAction::UpgradeContextMenu, Path::new(EXE)).unwrap();
    assert_eq!(reg.0, migrated);
    apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
    assert_eq!(reg.0.len(), 3);
    for class in CLASSES.into_iter().chain([NETWORK_LOCATION_CLASS]) {
        assert_eq!(
            reg.read(&shell_key(class), "").unwrap().as_deref(),
            Some("original default")
        );
    }
}

#[test]
fn migration_does_not_take_back_a_default_changed_by_another_app() {
    let mut reg = MemoryRegistry::default();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    reg.write(&shell_key("Directory"), "", "OtherManager.Open")
        .unwrap();
    let menu = format!("{}\\{VERB}", shell_key("Directory"));
    reg.write(
        &format!("{menu}\\command"),
        "",
        &folder_command(Path::new(EXE)),
    )
    .unwrap();
    apply(&mut reg, RegistryAction::UpgradeContextMenu, Path::new(EXE)).unwrap();
    assert_eq!(
        reg.read(&shell_key("Directory"), "").unwrap().as_deref(),
        Some("OtherManager.Open")
    );
    assert_eq!(reg.read(&format!("{menu}\\command"), "").unwrap(), None);
}

#[test]
fn restore_preserves_a_changed_native_open_command_or_delegate() {
    for (name, value) in [
        ("", "another command"),
        ("DelegateExecute", "another delegate"),
    ] {
        let mut reg = MemoryRegistry::default();
        apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
        let key = format!("{}\\command", open_key("Directory"));
        reg.write(&key, name, value).unwrap();
        apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
        assert_eq!(reg.read(&key, name).unwrap().as_deref(), Some(value));
    }
}

#[test]
fn foreign_native_open_backup_is_not_overwritten() {
    let mut reg = MemoryRegistry::default();
    reg.write(&state_key("Directory"), "OpenOwner", "another installation")
        .unwrap();
    let before = reg.0.clone();
    assert!(apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).is_err());
    apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
    assert_eq!(reg.0, before);
}

#[test]
fn restore_preserves_position_changed_by_another_app() {
    let mut reg = MemoryRegistry::default();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    reg.write(&open_key("Directory"), "Position", "Bottom")
        .unwrap();
    apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
    assert_eq!(
        reg.read(&open_key("Directory"), "Position")
            .unwrap()
            .as_deref(),
        Some("Bottom")
    );
}

#[test]
fn old_installations_can_restore_or_enable_position_without_a_prior_backup() {
    for reenable in [false, true] {
        let mut reg = MemoryRegistry::default();
        apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
        for class in CLASSES.into_iter().chain([NETWORK_LOCATION_CLASS]) {
            reg.delete(&open_key(class), "Position").unwrap();
            for name in ["OpenPosition", "HadOpenPosition", "TypeOpenPosition"] {
                reg.delete(&state_key(class), name).unwrap();
            }
        }
        if reenable {
            apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
        }
        apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
        assert!(reg.0.is_empty());
    }
}
