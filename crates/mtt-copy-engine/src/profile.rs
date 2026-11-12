//! Per-job policy. Only Windows-reported disk identities are used, never drive letters.
#[cfg(test)]
mod tests;
mod windows;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub(crate) const LARGE_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Ssd,
    Hdd,
    Remote,
    Unknown,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Device {
    pub disk: Option<u32>,
    pub kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Resource {
    Disk(u32),
    Unknown,
}

/// Privacy-safe description: no volume names, device numbers, paths or serials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyInfo {
    pub workers: usize,
    pub large_workers: usize,
    pub shared_device: bool,
    pub system_device: bool,
    pub conservative: bool,
}

pub(crate) struct Policy {
    pub info: PolicyInfo,
    pub resources: Vec<Resource>,
}

pub(crate) fn profile(sources: &[PathBuf], destination: &Path) -> Policy {
    windows::profile(sources, destination)
}

pub(crate) fn choose(sources: &[Device], destination: Device, system_disk: Option<u32>) -> Policy {
    let devices: Vec<_> = sources.iter().copied().chain(Some(destination)).collect();
    let shared_device =
        destination.disk.is_some() && sources.iter().any(|d| d.disk == destination.disk);
    let system_device = system_disk.is_some() && devices.iter().any(|d| d.disk == system_disk);
    let conservative = system_disk.is_none()
        || devices
            .iter()
            .any(|d| d.disk.is_none() || d.kind != Kind::Ssd);
    let (workers, large_workers) = if conservative {
        (1, 1)
    } else if shared_device || system_device {
        (2, 1)
    } else {
        (4, 2)
    };
    let resources: BTreeSet<_> = devices
        .iter()
        .map(|d| d.disk.map(Resource::Disk).unwrap_or(Resource::Unknown))
        .collect();
    Policy {
        info: PolicyInfo {
            workers,
            large_workers,
            shared_device,
            system_device,
            conservative,
        },
        resources: resources.into_iter().collect(),
    }
}
