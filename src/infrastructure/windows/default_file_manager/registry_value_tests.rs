use super::*;

#[derive(Default)]
struct TypedRegistry {
    values: BTreeMap<(String, String), RegistryValue>,
    fail_at: Option<usize>,
    mutations: usize,
}

impl TypedRegistry {
    fn before_mutation(&mut self) -> Result<(), String> {
        self.mutations += 1;
        if self.fail_at == Some(self.mutations) {
            Err("Injected typed registry failure".into())
        } else {
            Ok(())
        }
    }
}

impl Registry for TypedRegistry {
    fn read(&self, path: &str, name: &str) -> Result<Option<String>, String> {
        self.read_value(path, name)
            .map(|value| value.map(|value| value.text))
    }
    fn write(&mut self, path: &str, name: &str, value: &str) -> Result<(), String> {
        self.write_value(path, name, &RegistryValue::plain(value))
    }
    fn read_value(&self, path: &str, name: &str) -> Result<Option<RegistryValue>, String> {
        Ok(self.values.get(&(path.into(), name.into())).cloned())
    }
    fn write_value(&mut self, path: &str, name: &str, value: &RegistryValue) -> Result<(), String> {
        self.before_mutation()?;
        self.values
            .insert((path.into(), name.into()), value.clone());
        Ok(())
    }
    fn delete(&mut self, path: &str, name: &str) -> Result<(), String> {
        self.before_mutation()?;
        self.values.remove(&(path.into(), name.into()));
        Ok(())
    }
    fn delete_key_if_empty(&mut self, _path: &str) -> Result<(), String> {
        self.before_mutation()
    }
}

fn seeded_registry() -> TypedRegistry {
    let mut reg = TypedRegistry::default();
    for class in CLASSES.into_iter().chain([NETWORK_LOCATION_CLASS]) {
        let open = format!("{}\\open", shell_key(class));
        reg.write_value(
            &format!("{open}\\command"),
            "",
            &RegistryValue {
                text: r"%SystemRoot%\Explorer.exe".into(),
                expandable: true,
            },
        )
        .unwrap();
        reg.write_value(
            &open,
            "Icon",
            &RegistryValue {
                text: r"%SystemRoot%\Explorer.exe,0".into(),
                expandable: true,
            },
        )
        .unwrap();
        reg.write(
            &format!("{open}\\command"),
            "DelegateExecute",
            "previous delegate",
        )
        .unwrap();
    }
    // Legacy migration must also accept unrelated native expandable commands.
    reg.write_value(
        LEGACY_KEYS[0],
        "",
        &RegistryValue {
            text: r"%SystemRoot%\Explorer.exe".into(),
            expandable: true,
        },
    )
    .unwrap();
    reg
}

#[test]
fn restore_preserves_expandable_registry_values_without_expanding_their_contents() {
    let mut reg = seeded_registry();
    let before = reg.values.clone();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
    apply(&mut reg, RegistryAction::RestoreFolders, Path::new(EXE)).unwrap();
    assert_eq!(reg.values, before);
}

#[test]
fn failed_enable_and_restore_roll_back_the_original_registry_types() {
    for action in [
        RegistryAction::EnableFolders,
        RegistryAction::RestoreFolders,
    ] {
        for fail_at in 1..=300 {
            let mut reg = seeded_registry();
            if matches!(action, RegistryAction::RestoreFolders) {
                apply(&mut reg, RegistryAction::EnableFolders, Path::new(EXE)).unwrap();
            }
            let before = reg.values.clone();
            reg.fail_at = Some(fail_at);
            reg.mutations = 0;
            let result = apply(&mut reg, action, Path::new(EXE));
            if reg.mutations >= fail_at {
                assert!(result.is_err());
                assert_eq!(reg.values, before, "failure at mutation {fail_at}");
            } else {
                assert!(result.is_ok());
            }
        }
    }
}
