/// Cloud drive entry shown in the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudRoot {
    /// Operational filesystem path used for navigation and file operations.
    pub path: String,
    pub label: String,
    pub icon_resource: Option<String>,
    pub kind: CloudRootKind,
    /// Optional provider-facing path that should not be treated as a normal drive
    /// entry, e.g. Google Drive's virtual drive root (`H:\`).
    pub source_path: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudRootKind {
    /// Windows Cloud Files sync roots registered with Explorer's SyncRootManager.
    WindowsCloudFiles,
    /// Google Drive for Desktop virtual drive resolved through an inner `.lnk`.
    GoogleDriveShortcut,
}

impl CloudRoot {
    pub fn windows_cloud_files(path: String, label: String, icon_resource: Option<String>) -> Self {
        Self {
            path,
            label,
            icon_resource,
            kind: CloudRootKind::WindowsCloudFiles,
            source_path: None,
        }
    }

    pub fn google_drive_shortcut(
        path: String,
        label: String,
        icon_resource: Option<String>,
        source_path: String,
    ) -> Self {
        Self {
            path,
            label,
            icon_resource,
            kind: CloudRootKind::GoogleDriveShortcut,
            source_path: Some(source_path),
        }
    }

    pub fn is_windows_cloud_files(&self) -> bool {
        self.kind == CloudRootKind::WindowsCloudFiles
    }
}
