//! Roll back completed registry value changes if a multi-class action fails.

use super::registration::{Registry, RegistryValue};

struct UndoValue {
    path: String,
    name: String,
    previous: Option<RegistryValue>,
    written: Option<RegistryValue>,
}

pub(super) struct RegistryTransaction<'a, R: Registry> {
    registry: &'a mut R,
    undo: Vec<UndoValue>,
    cleanup: Vec<String>,
}

pub(super) fn run<R: Registry>(
    registry: &mut R,
    action: impl FnOnce(&mut RegistryTransaction<'_, R>) -> Result<(), String>,
) -> Result<(), String> {
    let mut transaction = RegistryTransaction {
        registry,
        undo: Vec::new(),
        cleanup: Vec::new(),
    };
    let result = action(&mut transaction).and_then(|_| transaction.commit_cleanup());
    match result {
        Ok(()) => Ok(()),
        Err(error) => match transaction.rollback() {
            Ok(()) => Err(error),
            Err(rollback) => Err(format!(
                "{error} Rollback could not restore all settings: {rollback}"
            )),
        },
    }
}

impl<R: Registry> RegistryTransaction<'_, R> {
    fn commit_cleanup(&mut self) -> Result<(), String> {
        for path in &self.cleanup {
            self.registry.delete_key_if_empty(path)?;
        }
        Ok(())
    }

    fn rollback(&mut self) -> Result<(), String> {
        let mut failure = None;
        for value in self.undo.iter().rev() {
            let restored = (|| {
                if self.registry.read_value(&value.path, &value.name)? != value.written {
                    return Err(
                        "A shell setting changed concurrently; that change was preserved.".into(),
                    );
                }
                if let Some(previous) = &value.previous {
                    self.registry
                        .write_value(&value.path, &value.name, previous)
                } else {
                    self.registry.delete(&value.path, &value.name)
                }
            })();
            if let Err(error) = restored {
                failure.get_or_insert(error);
            }
        }
        // Remove only empty containers owned by MTT, never a generic class/Shell key.
        let mut keys: Vec<&str> = self
            .undo
            .iter()
            .filter(|value| value.previous.is_none())
            .map(|value| value.path.as_str())
            .filter(|path| {
                path.contains("\\MTT.FileManager.Default")
                    || super::registration::CLASSES
                        .into_iter()
                        .chain([super::registration::NETWORK_LOCATION_CLASS])
                        .any(|class| {
                            let verb = format!("{}\\open", super::registration::shell_key(class));
                            *path == verb || *path == format!("{verb}\\command")
                        })
                    || *path == "Software\\MTT-File-Manager\\ShellIntegration\\WinE"
                    || path.starts_with(
                        "Software\\MTT-File-Manager\\ShellIntegration\\DefaultFolders\\",
                    )
            })
            .collect();
        keys.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
        keys.dedup();
        for key in keys {
            if let Err(error) = self.registry.delete_key_if_empty(key) {
                failure.get_or_insert(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

impl<R: Registry> Registry for RegistryTransaction<'_, R> {
    fn read(&self, path: &str, name: &str) -> Result<Option<String>, String> {
        self.registry.read(path, name)
    }

    fn write(&mut self, path: &str, name: &str, value: &str) -> Result<(), String> {
        self.write_value(path, name, &RegistryValue::plain(value))
    }

    fn read_value(&self, path: &str, name: &str) -> Result<Option<RegistryValue>, String> {
        self.registry.read_value(path, name)
    }

    fn write_value(&mut self, path: &str, name: &str, value: &RegistryValue) -> Result<(), String> {
        let previous = self.registry.read_value(path, name)?;
        self.registry.write_value(path, name, value)?;
        self.undo.push(UndoValue {
            path: path.into(),
            name: name.into(),
            previous,
            written: Some(value.clone()),
        });
        Ok(())
    }

    fn delete(&mut self, path: &str, name: &str) -> Result<(), String> {
        let previous = self.registry.read_value(path, name)?;
        self.registry.delete(path, name)?;
        self.undo.push(UndoValue {
            path: path.into(),
            name: name.into(),
            previous,
            written: None,
        });
        Ok(())
    }

    fn delete_key_if_empty(&mut self, path: &str) -> Result<(), String> {
        // Retain keys until all value changes succeed, so failed actions can be undone.
        if !self.cleanup.iter().any(|key| key == path) {
            self.cleanup.push(path.into());
        }
        Ok(())
    }
}
