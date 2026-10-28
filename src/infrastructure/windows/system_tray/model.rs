use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum LocationKind {
    #[default]
    Folder,
    Tag,
    Computer,
    RecycleBin,
}

impl LocationKind {
    pub(super) const ALL: [Self; 4] = [Self::Folder, Self::Tag, Self::Computer, Self::RecycleBin];

    pub(crate) fn for_path(path: &str) -> Self {
        use crate::domain::special_paths::{
            is_tag_view_path, COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID,
        };
        if is_tag_view_path(path) {
            Self::Tag
        } else if path == COMPUTER_VIEW_ID {
            Self::Computer
        } else if path == RECYCLE_BIN_VIEW_ID {
            Self::RecycleBin
        } else {
            Self::Folder
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Target {
    pub workspace: usize,
    pub panel: Option<(bool, usize)>, // true = right panel
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub id: usize,
    pub path: String,
    #[serde(default)]
    pub icon: LocationKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Workspace {
    pub tab: Entry,
    pub panels: Option<[Vec<Entry>; 2]>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Snapshot {
    pub workspaces: Vec<Workspace>,
    pub active_path: String,
    #[serde(default)]
    pub active_icon: LocationKind,
    pub hidden_at: Option<String>,
}

pub(crate) struct ResolvedTarget {
    pub workspace_index: usize,
    pub panel: Option<(bool, usize)>,
}

impl Snapshot {
    pub(crate) fn resolve(&self, target: &Target) -> Option<ResolvedTarget> {
        let index = self
            .workspaces
            .iter()
            .position(|w| w.tab.id == target.workspace)?;
        let panel = if let Some((right, id)) = target.panel {
            let entries = &self.workspaces[index].panels.as_ref()?[usize::from(right)];
            Some((right, entries.iter().position(|entry| entry.id == id)?))
        } else {
            None
        };
        Some(ResolvedTarget {
            workspace_index: index,
            panel,
        })
    }
}

#[derive(Serialize, Deserialize)]
pub(super) enum Packet {
    Snapshot(Snapshot),
    Activate(Target),
    RequestSession {
        token: String,
    },
    SessionReply {
        token: String,
        session: crate::tabs::TabManagerSession,
    },
    ExitSessionGroup {
        token: String,
    },
    SessionExitFailed {
        token: String,
    },
}

pub(super) fn menu_label(value: &str) -> String {
    // Native menus interpret ampersands as accelerators and tabs as columns.
    value
        .replace('&', "&&")
        .replace(['\t', '\r', '\n', '\0'], " ")
}
