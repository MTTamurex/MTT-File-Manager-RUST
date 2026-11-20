use std::collections::HashMap;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::{
    GetDriveTypeW, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW,
};
use windows::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};

use super::{
    configured_max_bytes, current_user_sid, drive_root, is_recycle_bin_path, is_reparse_point,
    registry_dword, requires_shell_attributes, shell_confirmation_is_enabled, RecycleContext,
    EXPLORER_BITBUCKET,
};
use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

pub(crate) struct EligibilityBatch {
    confirmation_already_shown: bool,
    shell_confirmation_enabled: Option<bool>,
    machine_policy_allows_recycling: Option<bool>,
    user_sid: Option<String>,
    user_sid_loaded: bool,
    volumes: HashMap<PathBuf, Option<VolumeContext>>,
}

impl EligibilityBatch {
    pub(crate) fn new(confirmation_already_shown: bool) -> Self {
        Self {
            confirmation_already_shown,
            shell_confirmation_enabled: None,
            machine_policy_allows_recycling: None,
            user_sid: None,
            user_sid_loaded: false,
            volumes: HashMap::new(),
        }
    }

    pub(crate) fn prepare(&mut self, path: &Path) -> Option<RecycleContext> {
        if !self.confirmation_allows_recycling() {
            return None;
        }
        if !path.is_absolute()
            || path.file_name().is_none()
            || is_recycle_bin_path(path)
            || crate::infrastructure::security::validate_no_reparse_points(path).is_err()
        {
            return None;
        }
        if !self.machine_policy_allows_recycling() {
            return None;
        }

        let root = drive_root(path)?;
        if !self.user_sid_loaded {
            self.user_sid = current_user_sid();
            self.user_sid_loaded = true;
        }
        let user_sid = self.user_sid.as_deref();
        let volume = cached_volume_context(&mut self.volumes, root.clone(), || {
            user_sid.and_then(|sid| VolumeContext::load(&root, sid))
        })?;

        // Cache expensive volume queries, not mutable filesystem safety checks.
        if !recycle_directory_is_safe(&volume.recycle_directory) {
            return None;
        }
        let source_metadata = std::fs::symlink_metadata(path).ok()?;
        if requires_shell_attributes(&source_metadata)
            || (!source_metadata.is_file() && !source_metadata.is_dir())
        {
            return None;
        }
        let item_size = if source_metadata.is_dir() {
            crate::infrastructure::global_search::folder_size(path)
                .ok()
                .map(|result| result.0)?
        } else {
            source_metadata.len()
        };
        if !volume.has_capacity(item_size) {
            return None;
        }

        Some(RecycleContext {
            recycle_directory: volume.recycle_directory.clone(),
            volume_key: root,
            item_size,
        })
    }

    pub(crate) fn record_recycled(&mut self, context: &RecycleContext) {
        if let Some(Some(volume)) = self.volumes.get_mut(&context.volume_key) {
            volume.record_success(context.item_size);
        }
    }

    pub(crate) fn invalidate_after_shell(&mut self) {
        *self = Self::new(self.confirmation_already_shown);
    }

    fn confirmation_allows_recycling(&mut self) -> bool {
        if self.confirmation_already_shown {
            return true;
        }
        !*self
            .shell_confirmation_enabled
            .get_or_insert_with(|| shell_confirmation_is_enabled().unwrap_or(true))
    }

    fn machine_policy_allows_recycling(&mut self) -> bool {
        *self.machine_policy_allows_recycling.get_or_insert_with(|| {
            match registry_dword(HKEY_LOCAL_MACHINE, EXPLORER_BITBUCKET, "NukeOnDelete") {
                Ok(value) => !value.is_some_and(|value| value != 0),
                Err(()) => false,
            }
        })
    }
}

fn recycle_directory_is_safe(directory: &Path) -> bool {
    let Some(root) = directory.parent() else {
        return false;
    };
    [root, directory].into_iter().all(|path| {
        std::fs::symlink_metadata(path)
            .is_ok_and(|metadata| metadata.is_dir() && !is_reparse_point(&metadata))
    })
}

struct VolumeContext {
    recycle_directory: PathBuf,
    max_bytes: u64,
    used_bytes: u64,
}

impl VolumeContext {
    fn load(drive_root: &Path, user_sid: &str) -> Option<Self> {
        let root_wide: Vec<u16> = drive_root
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe { GetDriveTypeW(PCWSTR(root_wide.as_ptr())) } != 3 {
            return None;
        }

        let mut file_system = [0u16; 32];
        if unsafe {
            GetVolumeInformationW(
                PCWSTR(root_wide.as_ptr()),
                None,
                None,
                None,
                None,
                Some(&mut file_system),
            )
        }
        .is_err()
            || !String::from_utf16_lossy(&file_system)
                .trim_end_matches('\0')
                .eq_ignore_ascii_case("NTFS")
        {
            return None;
        }

        let volume_guid = volume_guid(drive_root)?;
        let max_bytes = configured_max_bytes(&volume_guid)?;
        let recycle_root = drive_root.join("$Recycle.Bin");
        let recycle_directory = recycle_root.join(user_sid);
        if !recycle_directory_is_safe(&recycle_directory) {
            return None;
        }

        let mut bin_info = SHQUERYRBINFO {
            cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
            ..Default::default()
        };
        if unsafe { SHQueryRecycleBinW(PCWSTR(root_wide.as_ptr()), &mut bin_info) }.is_err()
            || bin_info.i64Size < 0
        {
            return None;
        }

        Some(Self {
            recycle_directory,
            max_bytes,
            used_bytes: bin_info.i64Size as u64,
        })
    }

    fn has_capacity(&self, item_size: u64) -> bool {
        self.used_bytes
            .checked_add(item_size)
            .is_some_and(|projected_size| projected_size <= self.max_bytes)
    }

    fn record_success(&mut self, item_size: u64) {
        self.used_bytes = self.used_bytes.saturating_add(item_size);
    }
}

fn volume_guid(drive_root: &Path) -> Option<String> {
    let root: Vec<u16> = drive_root
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let mut name = [0u16; 50];
    if unsafe { GetVolumeNameForVolumeMountPointW(PCWSTR(root.as_ptr()), &mut name) }.is_err() {
        return None;
    }
    let end = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    let value = String::from_utf16(&name[..end]).ok()?;
    let start = value.find('{')?;
    let finish = value[start..].find('}')? + start + 1;
    Some(value[start..finish].to_string())
}

fn cached_volume_context<'a>(
    volumes: &'a mut HashMap<PathBuf, Option<VolumeContext>>,
    key: PathBuf,
    load: impl FnOnce() -> Option<VolumeContext>,
) -> Option<&'a mut VolumeContext> {
    volumes.entry(key).or_insert_with(load).as_mut()
}

#[cfg(test)]
mod tests {
    use super::{cached_volume_context, EligibilityBatch, VolumeContext};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn volume(max_bytes: u64, used_bytes: u64) -> VolumeContext {
        VolumeContext {
            recycle_directory: PathBuf::from("recycle-bin"),
            max_bytes,
            used_bytes,
        }
    }

    #[test]
    fn same_volume_context_is_loaded_once_and_capacity_tracks_successes() {
        let mut batch = EligibilityBatch::new(true);
        let mut load_count = 0;
        let key = PathBuf::from(r"C:\");

        {
            let first = cached_volume_context(&mut batch.volumes, key.clone(), || {
                load_count += 1;
                Some(volume(100, 20))
            })
            .unwrap();
            assert!(first.has_capacity(30));
        }
        batch.record_recycled(&super::super::RecycleContext {
            recycle_directory: PathBuf::from("recycle-bin"),
            volume_key: key.clone(),
            item_size: 30,
        });
        {
            let second = cached_volume_context(&mut batch.volumes, key.clone(), || {
                load_count += 1;
                Some(volume(100, 0))
            })
            .unwrap();
            assert!(second.has_capacity(50));
            assert!(!second.has_capacity(51));
            assert_eq!(second.used_bytes, 50);
        }
        assert_eq!(load_count, 1);
    }

    #[test]
    fn unavailable_volume_is_cached_for_the_batch() {
        let mut volumes = HashMap::new();
        let mut load_count = 0;
        let key = PathBuf::from(r"C:\");

        assert!(cached_volume_context(&mut volumes, key.clone(), || {
            load_count += 1;
            None
        })
        .is_none());
        assert!(cached_volume_context(&mut volumes, key, || {
            load_count += 1;
            Some(volume(100, 0))
        })
        .is_none());
        assert_eq!(load_count, 1);
    }

    #[test]
    fn shell_fallback_invalidates_occupancy_and_policy_snapshots() {
        let mut batch = EligibilityBatch::new(true);
        let key = PathBuf::from(r"C:\");
        batch.volumes.insert(key.clone(), Some(volume(100, 20)));
        batch.shell_confirmation_enabled = Some(false);
        batch.machine_policy_allows_recycling = Some(true);
        batch.user_sid_loaded = true;
        batch.user_sid = Some("test-sid".to_string());

        batch.invalidate_after_shell();

        assert!(batch.confirmation_already_shown);
        assert!(batch.volumes.is_empty());
        assert!(batch.shell_confirmation_enabled.is_none());
        assert!(batch.machine_policy_allows_recycling.is_none());
        assert!(!batch.user_sid_loaded);
        let reloaded =
            cached_volume_context(&mut batch.volumes, key, || Some(volume(100, 90))).unwrap();
        assert!(!reloaded.has_capacity(11));
        assert!(reloaded.has_capacity(10));
    }

    #[test]
    fn volumes_have_independent_capacity_and_reject_overflow() {
        let mut batch = EligibilityBatch::new(true);
        let first = PathBuf::from(r"C:\");
        let second = PathBuf::from(r"D:\");
        batch.volumes.insert(first.clone(), Some(volume(100, 20)));
        batch
            .volumes
            .insert(second.clone(), Some(volume(u64::MAX, u64::MAX)));
        batch.record_recycled(&super::super::RecycleContext {
            recycle_directory: PathBuf::from("recycle-bin"),
            volume_key: first.clone(),
            item_size: 30,
        });
        assert_eq!(batch.volumes[&first].as_ref().unwrap().used_bytes, 50);
        let other = batch.volumes[&second].as_ref().unwrap();
        assert_eq!(other.used_bytes, u64::MAX);
        assert!(!other.has_capacity(1));
        assert!(other.has_capacity(0));
    }

    #[test]
    fn cached_destination_is_rechecked_when_replaced_by_a_file() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("bin");
        let directory = root.join("test-sid");
        std::fs::create_dir_all(&directory).unwrap();
        assert!(super::recycle_directory_is_safe(&directory));
        // Rename the fixture out of the way; never remove user data.
        std::fs::rename(&directory, root.join("previous")).unwrap();
        std::fs::write(&directory, b"not a directory").unwrap();
        assert!(!super::recycle_directory_is_safe(&directory));
        assert!(!super::recycle_directory_is_safe(&root.join("missing")));
    }
}
