use super::*;

#[test]
fn command_upgrade_preserves_previous_defaults_and_restores_after_migration() {
    let mut reg = MemoryRegistry::default();
    for class in CLASSES {
        reg.write(&shell_key(class), "", "native handler").unwrap();
    }
    let before = reg.0.clone();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    for class in CLASSES {
        reg.write(
            &format!("{}\\{VERB}\\command", shell_key(class)),
            "",
            &legacy_folder_command(Path::new(EXE)),
        )
        .unwrap();
    }
    assert!(needs_folder_command_upgrade(&reg, Path::new(EXE)));
    apply(
        &mut reg,
        RegistryAction::UpgradeFolderCommands,
        Path::new(EXE),
    )
    .unwrap();
    assert!(!needs_folder_command_upgrade(&reg, Path::new(EXE)));
    for class in CLASSES {
        assert!(class_is_active(
            &reg,
            class,
            &folder_command(Path::new(EXE))
        ));
        assert_eq!(
            reg.read(&state_key(class), "PreviousDefault")
                .unwrap()
                .as_deref(),
            Some("native handler")
        );
    }
    let migrated = reg.0.clone();
    apply(
        &mut reg,
        RegistryAction::UpgradeFolderCommands,
        Path::new(EXE),
    )
    .unwrap();
    assert_eq!(reg.0, migrated);
    apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
    assert_eq!(reg.0, before);
}

#[test]
fn command_upgrade_changes_only_commands_owned_by_this_executable() {
    let mut reg = MemoryRegistry::default();
    let other_key = format!("{}\\{VERB}\\command", shell_key("Drive"));
    reg.write(&other_key, "", "another installation").unwrap();
    assert!(!needs_folder_command_upgrade(&reg, Path::new(EXE)));
    reg.write(LEGACY_KEYS[0], "", &legacy_folder_command(Path::new(EXE)))
        .unwrap();
    assert!(needs_folder_command_upgrade(&reg, Path::new(EXE)));
    apply(
        &mut reg,
        RegistryAction::UpgradeFolderCommands,
        Path::new(EXE),
    )
    .unwrap();
    assert_eq!(
        reg.read(&other_key, "").unwrap().as_deref(),
        Some("another installation")
    );
    assert_eq!(
        reg.read(LEGACY_KEYS[0], "").unwrap().as_deref(),
        Some(folder_command(Path::new(EXE)).as_str())
    );
    assert_eq!(reg.read(&shell_key("Directory"), "").unwrap(), None);
}

#[test]
fn failed_command_upgrade_rolls_back_completed_changes() {
    for fail_at in 1..=4 {
        let mut inner = MemoryRegistry::default();
        for class in CLASSES {
            inner
                .write(
                    &format!("{}\\{VERB}\\command", shell_key(class)),
                    "",
                    &legacy_folder_command(Path::new(EXE)),
                )
                .unwrap();
        }
        for key in LEGACY_KEYS {
            inner
                .write(key, "", &legacy_folder_command(Path::new(EXE)))
                .unwrap();
        }
        let before = inner.0.clone();
        let mut reg = FailOnceRegistry {
            inner,
            fail_at,
            mutations: 0,
            concurrent_change: None,
        };
        assert!(apply(
            &mut reg,
            RegistryAction::UpgradeFolderCommands,
            Path::new(EXE)
        )
        .is_err());
        assert_eq!(reg.inner.0, before);
    }
}
