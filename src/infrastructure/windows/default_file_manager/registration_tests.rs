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

#[path = "command_upgrade_tests.rs"]
mod command_upgrade;

#[path = "win_e_tests.rs"]
mod win_e;

#[path = "legacy_menu_tests.rs"]
mod legacy_menu;

#[path = "native_open_tests.rs"]
mod native_open;

#[path = "registry_value_tests.rs"]
mod registry_values;

struct FailOnceRegistry {
    inner: MemoryRegistry,
    fail_at: usize,
    mutations: usize,
    concurrent_change: Option<(String, String, String)>,
}

impl FailOnceRegistry {
    fn before_mutation(&mut self) -> Result<(), String> {
        self.mutations += 1;
        if self.mutations == self.fail_at {
            if let Some((path, name, value)) = self.concurrent_change.take() {
                self.inner.0.insert((path, name), value);
            }
            Err("Injected registry mutation failure".into())
        } else {
            Ok(())
        }
    }
}

impl Registry for FailOnceRegistry {
    fn read(&self, path: &str, name: &str) -> Result<Option<String>, String> {
        self.inner.read(path, name)
    }
    fn write(&mut self, path: &str, name: &str, value: &str) -> Result<(), String> {
        self.before_mutation()?;
        self.inner.write(path, name, value)
    }
    fn delete(&mut self, path: &str, name: &str) -> Result<(), String> {
        self.before_mutation()?;
        self.inner.delete(path, name)
    }
    fn delete_key_if_empty(&mut self, path: &str) -> Result<(), String> {
        self.before_mutation()?;
        self.inner.delete_key_if_empty(path)
    }
}

#[test]
fn failed_registration_and_restore_return_all_completed_values_to_their_prior_state() {
    for action in [
        RegistryAction::EnableFolders,
        RegistryAction::RestoreFolders,
    ] {
        for fail_at in 1..=300 {
            let mut inner = MemoryRegistry::default();
            for class in CLASSES.into_iter().chain([NETWORK_LOCATION_CLASS]) {
                inner
                    .write(&shell_key(class), "", "previous default")
                    .unwrap();
            }
            if matches!(action, RegistryAction::RestoreFolders) {
                apply(&mut inner, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
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
                assert!(
                    result.is_err(),
                    "failure at mutation {fail_at} was not reported"
                );
                assert_eq!(reg.inner.0, before, "failure at mutation {fail_at}");
            } else {
                assert!(result.is_ok());
            }
        }
    }
}

#[test]
fn a_network_handler_owned_by_another_installation_is_detected_before_any_writes() {
    let mut inner = MemoryRegistry::default();
    inner
        .write(
            &format!("{}\\{VERB}\\command", shell_key(NETWORK_LOCATION_CLASS)),
            "",
            "another installation",
        )
        .unwrap();
    let before = inner.0.clone();
    let mut reg = FailOnceRegistry {
        inner,
        fail_at: usize::MAX,
        mutations: 0,
        concurrent_change: None,
    };
    assert!(apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).is_err());
    assert_eq!(reg.mutations, 0);
    assert_eq!(reg.inner.0, before);
}

#[test]
fn rollback_does_not_overwrite_a_concurrent_default_change() {
    let mut reg = FailOnceRegistry {
        inner: MemoryRegistry::default(),
        fail_at: 7,
        mutations: 0,
        concurrent_change: Some((
            shell_key("Directory"),
            "".into(),
            "OtherManager.Open".into(),
        )),
    };
    assert!(apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).is_err());
    assert_eq!(
        reg.read(&shell_key("Directory"), "").unwrap().as_deref(),
        Some("OtherManager.Open")
    );
    assert!(reg
        .read(&format!("{}\\{VERB}\\command", shell_key("Directory")), "")
        .unwrap()
        .is_none());
}

#[test]
fn folder_shortcut_registration_uses_its_own_resolving_command_and_restores_its_previous_default() {
    let mut reg = MemoryRegistry::default();
    reg.write(
        &shell_key(NETWORK_LOCATION_CLASS),
        "",
        "native shortcut handler",
    )
    .unwrap();
    let before = reg.0.clone();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    assert!(class_is_active(
        &reg,
        NETWORK_LOCATION_CLASS,
        &network_location_command(Path::new(EXE))
    ));
    assert!(network_location_command(Path::new(EXE)).contains("--open-network-location"));
    assert!(!needs_network_location_upgrade(&reg, Path::new(EXE)));
    apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
    assert_eq!(reg.0, before);
}

#[test]
fn folder_shortcut_upgrade_requires_existing_owned_directory_and_drive_defaults() {
    let mut reg = MemoryRegistry::default();
    assert!(!needs_network_location_upgrade(&reg, Path::new(EXE)));
    enable_class(
        &mut reg,
        "Directory",
        &folder_command(Path::new(EXE)),
        Path::new(EXE),
    )
    .unwrap();
    assert!(!needs_network_location_upgrade(&reg, Path::new(EXE)));
    enable_class(
        &mut reg,
        "Drive",
        &folder_command(Path::new(EXE)),
        Path::new(EXE),
    )
    .unwrap();
    assert!(needs_network_location_upgrade(&reg, Path::new(EXE)));
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    reg.write(&shell_key(NETWORK_LOCATION_CLASS), "", "OtherManager.Open")
        .unwrap();
    assert!(!needs_network_location_upgrade(&reg, Path::new(EXE)));
}

#[test]
fn network_location_activation_accepts_an_already_resolved_unc_target_without_probing_it() {
    let target = Path::new(r"\\server\share\folder");
    assert_eq!(
        super::super::resolve_network_location_open_target(target),
        Some(target.to_path_buf())
    );
    assert!(super::super::is_supported_folder_target(target));
}

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
