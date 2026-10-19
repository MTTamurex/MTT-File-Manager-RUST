use super::*;
use crate::infrastructure::windows::default_file_manager::legacy_menu;

const OLD_VERB: &str = "Software\\Classes\\Directory\\shell\\MTT.FileManager.OpenInMTT";
const OLD_COMMAND: &str = "Software\\Classes\\Directory\\shell\\MTT.FileManager.OpenInMTT\\command";
const OLD_STATE: &str = "Software\\MTT-File-Manager\\ShellIntegration\\ContextMenu";

fn seed_legacy(reg: &mut impl Registry) {
    reg.write(OLD_VERB, "", "Abrir no MTT").unwrap();
    reg.write(OLD_VERB, "Icon", EXE).unwrap();
    reg.write(OLD_COMMAND, "", &legacy_folder_command(Path::new(EXE)))
        .unwrap();
    reg.write(OLD_STATE, "", EXE).unwrap();
}

fn seed_class(reg: &mut impl Registry, class: &str) {
    let verb = format!("{}\\{VERB}", shell_key(class));
    let command = if class == NETWORK_LOCATION_CLASS {
        network_location_command(Path::new(EXE))
    } else {
        folder_command(Path::new(EXE))
    };
    reg.write(&verb, "", "Abrir no MTT").unwrap();
    reg.write(&verb, "Icon", EXE).unwrap();
    reg.write(&verb, "Position", "Top").unwrap();
    reg.write(&format!("{verb}\\command"), "", &command)
        .unwrap();
}

#[test]
fn enabling_registers_only_native_open_without_any_extra_menu_verb() {
    let mut reg = MemoryRegistry::default();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    assert!(!legacy_menu::has_owned_entries(&reg, Path::new(EXE)));
    for class in CLASSES.into_iter().chain([NETWORK_LOCATION_CLASS]) {
        assert_eq!(
            reg.read(&shell_key(class), "").unwrap().as_deref(),
            Some("open")
        );
        let verb = format!("{}\\{VERB}", shell_key(class));
        assert!(!reg.0.keys().any(|(key, _)| key.starts_with(&verb)));
    }
}

#[test]
fn migration_removes_both_deprecated_verbs_and_preserves_the_native_backup() {
    let mut reg = MemoryRegistry::default();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    let native_only = reg.0.clone();
    seed_legacy(&mut reg);
    for class in CLASSES.into_iter().chain([NETWORK_LOCATION_CLASS]) {
        seed_class(&mut reg, class);
    }
    assert!(legacy_menu::has_owned_entries(&reg, Path::new(EXE)));
    apply(&mut reg, RegistryAction::UpgradeContextMenu, Path::new(EXE)).unwrap();
    assert_eq!(reg.0, native_only);
    assert!(!legacy_menu::has_owned_entries(&reg, Path::new(EXE)));
    apply(&mut reg, RegistryAction::UpgradeContextMenu, Path::new(EXE)).unwrap();
    assert_eq!(reg.0, native_only);
}

#[test]
fn migration_preserves_unknown_values_in_deprecated_menu_keys() {
    let mut reg = MemoryRegistry::default();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    seed_legacy(&mut reg);
    seed_class(&mut reg, "Directory");
    let verb = format!("{}\\{VERB}", shell_key("Directory"));
    for key in [OLD_VERB, verb.as_str()] {
        reg.write(key, "Unrelated", "preserve").unwrap();
    }
    apply(&mut reg, RegistryAction::UpgradeContextMenu, Path::new(EXE)).unwrap();
    for key in [OLD_VERB, verb.as_str()] {
        assert_eq!(
            reg.read(key, "Unrelated").unwrap().as_deref(),
            Some("preserve")
        );
        assert_eq!(reg.read(key, "").unwrap(), None);
    }
}

#[test]
fn foreign_legacy_commands_or_markers_are_preserved() {
    for (key, value) in [
        (OLD_COMMAND, "another command"),
        (OLD_STATE, r"C:\Other\mtt.exe"),
    ] {
        let mut reg = MemoryRegistry::default();
        seed_legacy(&mut reg);
        reg.write(key, "", value).unwrap();
        let before = reg.0.clone();
        apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
        apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
        assert_eq!(reg.0, before);
    }
}

#[test]
fn failures_restore_the_native_handlers_and_both_deprecated_menu_entries() {
    for action in [
        RegistryAction::EnableFolders,
        RegistryAction::UpgradeContextMenu,
        RegistryAction::RestoreFolders,
    ] {
        for fail_at in 1..=300 {
            let mut inner = MemoryRegistry::default();
            apply(&mut inner, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
            seed_legacy(&mut inner);
            for class in CLASSES.into_iter().chain([NETWORK_LOCATION_CLASS]) {
                seed_class(&mut inner, class);
            }
            let before = inner.0.clone();
            let mut reg = FailOnceRegistry {
                inner,
                fail_at,
                mutations: 0,
                concurrent_change: None,
            };
            let result = apply(&mut reg, action, Path::new(EXE));
            if reg.mutations >= fail_at {
                assert!(result.is_err());
                assert_eq!(reg.inner.0, before, "failure at mutation {fail_at}");
            } else {
                assert!(result.is_ok());
            }
        }
    }
}

#[test]
fn restoring_removes_native_overrides_and_both_owned_extra_menu_entries() {
    let mut reg = MemoryRegistry::default();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    seed_legacy(&mut reg);
    for class in CLASSES.into_iter().chain([NETWORK_LOCATION_CLASS]) {
        seed_class(&mut reg, class);
    }
    apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
    assert!(reg.0.is_empty());
}
