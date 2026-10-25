//! Persistable column identities and layouts, independent of Shell COM objects.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PropertyId {
    pub fmtid: u128,
    pub pid: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColumnId {
    Name,
    Date,
    Type,
    Size,
    Status,
    TotalSpace,
    FreeSpace,
    OriginalLocation,
    Tags,
    Property(PropertyId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColumnScope {
    Files,
    Computer,
    RecycleBin,
}

impl ColumnScope {
    pub fn from_view(computer: bool, recycle: bool) -> Self {
        if computer {
            Self::Computer
        } else if recycle {
            Self::RecycleBin
        } else {
            Self::Files
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnSetting {
    pub id: ColumnId,
    pub visible: bool,
    pub width: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnLayout {
    pub columns: Vec<ColumnSetting>,
}

impl ColumnLayout {
    pub fn defaults(scope: ColumnScope) -> Self {
        use ColumnId::*;
        let ids: &[ColumnId] = match scope {
            ColumnScope::Files => &[Name, Date, Type, Size, Status],
            ColumnScope::Computer => &[Name, TotalSpace, FreeSpace],
            ColumnScope::RecycleBin => &[Name, Date, Type, Size],
        };
        Self {
            columns: ids
                .iter()
                .map(|&id| ColumnSetting {
                    id,
                    visible: true,
                    width: if id == Name { 300.0 } else { 160.0 },
                })
                .collect(),
        }
    }

    pub fn normalize(&mut self) {
        let mut seen = HashSet::new();
        self.columns.retain(|column| seen.insert(column.id));
        if !seen.contains(&ColumnId::Name) {
            self.columns.insert(
                0,
                ColumnSetting {
                    id: ColumnId::Name,
                    visible: true,
                    width: 300.0,
                },
            );
        }
        for column in &mut self.columns {
            if column.id == ColumnId::Name {
                column.visible = true;
            }
            column.width = if column.width.is_finite() {
                column.width.clamp(80.0, 4000.0)
            } else {
                160.0
            };
        }
    }

    pub fn set_visible(&mut self, id: ColumnId, visible: bool) {
        if let Some(column) = self.columns.iter_mut().find(|column| column.id == id) {
            column.visible = visible || id == ColumnId::Name;
        } else {
            self.columns.push(ColumnSetting {
                id,
                visible: visible || id == ColumnId::Name,
                width: 160.0,
            });
        }
    }

    pub fn is_visible(&self, id: ColumnId) -> bool {
        self.columns
            .iter()
            .any(|column| column.id == id && column.visible)
    }

    pub fn move_before(&mut self, source: ColumnId, target: Option<ColumnId>) {
        if target == Some(source) {
            return;
        }
        let Some(index) = self.columns.iter().position(|column| column.id == source) else {
            return;
        };
        let column = self.columns.remove(index);
        let destination = target
            .and_then(|id| self.columns.iter().position(|column| column.id == id))
            .unwrap_or(self.columns.len());
        self.columns.insert(destination, column);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnPreferences {
    pub files: ColumnLayout,
    pub computer: ColumnLayout,
    pub recycle_bin: ColumnLayout,
}

impl Default for ColumnPreferences {
    fn default() -> Self {
        Self {
            files: ColumnLayout::defaults(ColumnScope::Files),
            computer: ColumnLayout::defaults(ColumnScope::Computer),
            recycle_bin: ColumnLayout::defaults(ColumnScope::RecycleBin),
        }
    }
}

impl ColumnPreferences {
    pub fn load(value: Option<&str>) -> Self {
        let mut result: Self = value
            .and_then(|value| serde_json::from_str(value).ok())
            .unwrap_or_default();
        result.files.normalize();
        result.computer.normalize();
        result.recycle_bin.normalize();
        result
    }

    pub fn layout(&self, scope: ColumnScope) -> &ColumnLayout {
        match scope {
            ColumnScope::Files => &self.files,
            ColumnScope::Computer => &self.computer,
            ColumnScope::RecycleBin => &self.recycle_bin,
        }
    }

    pub fn layout_mut(&mut self, scope: ColumnScope) -> &mut ColumnLayout {
        match scope {
            ColumnScope::Files => &mut self.files,
            ColumnScope::Computer => &mut self.computer,
            ColumnScope::RecycleBin => &mut self.recycle_bin,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn name_is_movable_but_cannot_be_hidden() {
        let mut layout = ColumnLayout::defaults(ColumnScope::Files);
        layout.move_before(ColumnId::Name, Some(ColumnId::Size));
        assert_eq!(layout.columns[2].id, ColumnId::Name);
        layout.set_visible(ColumnId::Name, false);
        assert!(layout.is_visible(ColumnId::Name));
    }
    #[test]
    fn preferences_round_trip_and_scopes_are_independent() {
        let mut prefs = ColumnPreferences::default();
        prefs.files.set_visible(ColumnId::Type, false);
        prefs.files.set_visible(
            ColumnId::Property(PropertyId {
                fmtid: u128::MAX,
                pid: 42,
            }),
            true,
        );
        prefs
            .computer
            .move_before(ColumnId::FreeSpace, Some(ColumnId::Name));
        assert_eq!(
            ColumnPreferences::load(Some(&serde_json::to_string(&prefs).unwrap())),
            prefs
        );
        assert!(prefs.recycle_bin.is_visible(ColumnId::Type));
    }
    #[test]
    fn invalid_settings_recover_without_duplicate_or_missing_name() {
        assert_eq!(
            ColumnPreferences::load(Some("invalid")),
            ColumnPreferences::default()
        );
        let mut layout = ColumnLayout {
            columns: vec![
                ColumnSetting {
                    id: ColumnId::Size,
                    visible: true,
                    width: f32::NAN
                };
                2
            ],
        };
        layout.normalize();
        assert_eq!(layout.columns.len(), 2);
        assert!(layout.is_visible(ColumnId::Name));
        assert!(layout.columns.iter().all(|column| column.width.is_finite()));
    }
}
