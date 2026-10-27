/// Cloud drive entry shown in the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CloudRoot {
    /// Operational filesystem path used for navigation and file operations.
    pub path: String,
    pub label: String,
    /// Provider label discovered from registered metadata, not the path name.
    pub breadcrumb_provider: Option<String>,
    /// Optional account or provider-root label supplied by discovery metadata.
    pub breadcrumb_account: Option<String>,
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
            breadcrumb_provider: None,
            breadcrumb_account: None,
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
            breadcrumb_provider: None,
            breadcrumb_account: None,
            icon_resource,
            kind: CloudRootKind::GoogleDriveShortcut,
            source_path: Some(source_path),
        }
    }

    pub fn is_google_drive_shortcut(&self) -> bool {
        self.kind == CloudRootKind::GoogleDriveShortcut
    }

    /// Paths tracked as cloud sync roots for sync-status detection.
    ///
    /// Google Drive entries track both the DriveFS virtual drive root and the
    /// mirrored folder resolved from the shortcut, so status icons apply to both
    /// surfaces.
    pub fn sync_root_paths(&self) -> Vec<&str> {
        match self.kind {
            CloudRootKind::WindowsCloudFiles => vec![self.path.as_str()],
            CloudRootKind::GoogleDriveShortcut => match self.source_path.as_deref() {
                Some(source) if !source.trim().is_empty() => vec![source, self.path.as_str()],
                _ => vec![self.path.as_str()],
            },
        }
    }
}
