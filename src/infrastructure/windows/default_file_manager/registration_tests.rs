use super::*;
use std::collections::BTreeMap;

#[derive(Default)]
struct MemoryRegistry(BTreeMap<(String, String), String>);

impl Registry for MemoryRegistry {
    fn read(&self, path: &str, name: &str) -> Result<Option<String>, String> {
        Ok(self.0.get(&(path.into(), name.into())).cloned())
    }
    fn write(&mut self, path: &str, name: &str, value: &str) -> Result<(), String> {
        self.0.insert((path.into(), name.into()), value.into());
        Ok(())
    }
    fn delete(&mut self, path: &str, name: &str) -> Result<(), String> {
        self.0.remove(&(path.into(), name.into()));
        Ok(())
    }
    fn delete_key_if_empty(&mut self, _path: &str) -> Result<(), String> {
        Ok(())
    }
}

const EXE: &str = r"C:\Program Files\Example\mtt-file-manager.exe";

#[test]
fn enabling_folders_leaves_virtual_folders_and_win_e_untouched() {
    let mut reg = MemoryRegistry::default();
    reg.write(LEGACY_KEYS[0], "", "native folder command")
        .unwrap();
    reg.write(WIN_E_COMMAND_KEY, "", "native launch command")
        .unwrap();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    for class in CLASSES {
        assert!(class_is_active(
            &reg,
            class,
            &folder_command(Path::new(EXE))
        ));
    }
    assert_eq!(
        reg.read(LEGACY_KEYS[0], "").unwrap().as_deref(),
        Some("native folder command")
    );
    assert_eq!(
        reg.read(WIN_E_COMMAND_KEY, "").unwrap().as_deref(),
        Some("native launch command")
    );
}

#[test]
fn repeated_enable_and_restore_preserves_previous_defaults_including_absent_and_empty() {
    for previous in [None, Some(""), Some("OtherManager.Open")] {
        let mut reg = MemoryRegistry::default();
        if let Some(value) = previous {
            for class in CLASSES {
                reg.write(&shell_key(class), "", value).unwrap();
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
fn migration_removes_only_legacy_commands_owned_by_this_installation() {
    let mut reg = MemoryRegistry::default();
    reg.write(LEGACY_KEYS[0], "", &folder_command(Path::new(EXE)))
        .unwrap();
    reg.write(LEGACY_KEYS[0], "DelegateExecute", "").unwrap();
    reg.write(LEGACY_KEYS[0], "Unrelated", "preserve").unwrap();
    reg.write(LEGACY_KEYS[1], "", "another manager").unwrap();
    reg.write(LEGACY_KEYS[1], "DelegateExecute", "another delegate")
        .unwrap();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    assert_eq!(reg.read(LEGACY_KEYS[0], "").unwrap(), None);
    assert_eq!(reg.read(LEGACY_KEYS[0], "DelegateExecute").unwrap(), None);
    assert_eq!(
        reg.read(LEGACY_KEYS[0], "Unrelated").unwrap().as_deref(),
        Some("preserve")
    );
    assert_eq!(
        reg.read(LEGACY_KEYS[1], "").unwrap().as_deref(),
        Some("another manager")
    );
    assert_eq!(
        reg.read(LEGACY_KEYS[1], "DelegateExecute")
            .unwrap()
            .as_deref(),
        Some("another delegate")
    );
}

#[test]
fn restore_does_not_replace_a_default_changed_by_another_application() {
    let mut reg = MemoryRegistry::default();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    reg.write(&shell_key("Directory"), "", "OtherManager.Open")
        .unwrap();
    apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
    assert_eq!(
        reg.read(&shell_key("Directory"), "").unwrap().as_deref(),
        Some("OtherManager.Open")
    );
}

#[test]
fn another_installations_verb_is_not_overwritten_or_removed() {
    let mut reg = MemoryRegistry::default();
    let key = format!("{}\\{VERB}\\command", shell_key("Directory"));
    reg.write(&key, "", "another installation").unwrap();
    let before = reg.0.clone();
    assert!(apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).is_err());
    apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
    assert_eq!(reg.0, before);
}

#[test]
fn win_e_changes_do_not_touch_folder_registration() {
    let mut reg = MemoryRegistry::default();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    let folders = reg.0.clone();
    apply(&mut reg, RegistryAction::EnableWinE, Path::new(EXE)).unwrap();
    apply(&mut reg, RegistryAction::RestoreWinE, Path::new(EXE)).unwrap();
    assert_eq!(reg.0, folders);
}

#[test]
fn shell_namespace_targets_are_rejected_before_opening_the_ui() {
    for target in [
        "shell:ControlPanelFolder",
        "ShElL:AppsFolder",
        "::{26EE0668-A00A-44D7-9371-BEB064C98683}",
        r"\\?\::{26EE0668-A00A-44D7-9371-BEB064C98683}",
    ] {
        assert!(!super::super::is_supported_folder_target(Path::new(target)));
    }
    let temp = tempfile::tempdir().unwrap();
    assert!(super::super::is_supported_folder_target(temp.path()));
    assert!(!super::super::is_supported_folder_target(
        &temp.path().join("missing")
    ));
    assert!(super::super::is_supported_folder_target(Path::new(
        r"\\server"
    )));
}
