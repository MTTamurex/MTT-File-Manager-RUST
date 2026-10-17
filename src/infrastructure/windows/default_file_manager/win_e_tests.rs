use super::*;

#[test]
fn repeated_enable_restores_both_values_including_absent_and_empty() {
    for command in [None, Some(""), Some("previous command")] {
        for delegate in [None, Some(""), Some("previous delegate")] {
            let mut reg = MemoryRegistry::default();
            if let Some(value) = command {
                reg.write(WIN_E_COMMAND_KEY, "", value).unwrap();
            }
            if let Some(value) = delegate {
                reg.write(WIN_E_COMMAND_KEY, "DelegateExecute", value)
                    .unwrap();
            }
            let before = reg.0.clone();
            apply(&mut reg, RegistryAction::EnableWinE, Path::new(EXE)).unwrap();
            apply(&mut reg, RegistryAction::EnableWinE, Path::new(EXE)).unwrap();
            apply(&mut reg, RegistryAction::RestoreWinE, Path::new(EXE)).unwrap();
            apply(&mut reg, RegistryAction::RestoreWinE, Path::new(EXE)).unwrap();
            assert_eq!(reg.0, before);
        }
    }
}

#[test]
fn restore_preserves_a_new_command_or_delegate() {
    for changed_name in ["", "DelegateExecute"] {
        let mut reg = MemoryRegistry::default();
        reg.write(WIN_E_COMMAND_KEY, "", "previous command")
            .unwrap();
        reg.write(WIN_E_COMMAND_KEY, "DelegateExecute", "previous delegate")
            .unwrap();
        apply(&mut reg, RegistryAction::EnableWinE, Path::new(EXE)).unwrap();
        reg.write(WIN_E_COMMAND_KEY, changed_name, "another application")
            .unwrap();
        apply(&mut reg, RegistryAction::RestoreWinE, Path::new(EXE)).unwrap();
        assert_eq!(
            reg.read(WIN_E_COMMAND_KEY, changed_name)
                .unwrap()
                .as_deref(),
            Some("another application")
        );
    }
}

#[test]
fn legacy_owned_registration_restores_without_backing_up_its_own_command() {
    let mut reg = MemoryRegistry::default();
    reg.write(WIN_E_COMMAND_KEY, "", &win_e_command(Path::new(EXE)))
        .unwrap();
    reg.write(WIN_E_COMMAND_KEY, "DelegateExecute", "").unwrap();
    apply(&mut reg, RegistryAction::EnableWinE, Path::new(EXE)).unwrap();
    apply(&mut reg, RegistryAction::RestoreWinE, Path::new(EXE)).unwrap();
    assert!(reg.0.is_empty());
}

#[test]
fn another_installation_cannot_overwrite_or_restore_the_backup() {
    let mut reg = MemoryRegistry::default();
    apply(&mut reg, RegistryAction::EnableWinE, Path::new(EXE)).unwrap();
    let before = reg.0.clone();
    let other = Path::new(r"C:\Apps\Other\mtt-file-manager.exe");
    assert!(apply(&mut reg, RegistryAction::EnableWinE, other).is_err());
    apply(&mut reg, RegistryAction::RestoreWinE, other).unwrap();
    assert_eq!(reg.0, before);
}

#[test]
fn failed_enable_and_restore_roll_back_every_completed_mutation() {
    for action in [RegistryAction::EnableWinE, RegistryAction::RestoreWinE] {
        for fail_at in 1..=20 {
            let mut inner = MemoryRegistry::default();
            inner
                .write(WIN_E_COMMAND_KEY, "", "previous command")
                .unwrap();
            inner
                .write(WIN_E_COMMAND_KEY, "DelegateExecute", "previous delegate")
                .unwrap();
            if matches!(action, RegistryAction::RestoreWinE) {
                apply(&mut inner, RegistryAction::EnableWinE, Path::new(EXE)).unwrap();
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
                assert_eq!(reg.inner.0, before, "mutation {fail_at}");
            } else {
                assert!(result.is_ok());
            }
        }
    }
}
