use super::{choose, Device, Kind, Policy};
use std::path::{Path, PathBuf};

#[cfg(windows)]
pub(super) fn profile(sources: &[PathBuf], destination: &Path) -> Policy {
    use std::collections::BTreeMap;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, GetDriveTypeW, GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
        FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Ioctl::{
        PropertyStandardQuery, StorageDeviceSeekPenaltyProperty, DEVICE_SEEK_PENALTY_DESCRIPTOR,
        IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY, STORAGE_DEVICE_NUMBER,
        STORAGE_PROPERTY_QUERY,
    };
    use windows::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;
    use windows::Win32::System::IO::DeviceIoControl;

    struct Probe {
        cache: BTreeMap<Vec<u16>, Device>,
    }
    impl Probe {
        fn device(&mut self, path: &Path) -> Device {
            let unknown = Device {
                disk: None,
                kind: Kind::Unknown,
            };
            let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let mut mount = vec![0u16; 32768];
            unsafe {
                if GetVolumePathNameW(PCWSTR(wide.as_ptr()), &mut mount).is_err() {
                    return unknown;
                }
                if GetDriveTypeW(PCWSTR(mount.as_ptr())) == 4 {
                    return Device {
                        disk: None,
                        kind: Kind::Remote,
                    };
                }
                let mut volume = vec![0u16; 64];
                if GetVolumeNameForVolumeMountPointW(PCWSTR(mount.as_ptr()), &mut volume).is_err() {
                    return unknown;
                }
                let Some(length) = volume.iter().position(|&value| value == 0) else {
                    return unknown;
                };
                volume.truncate(length);
                if volume.last() == Some(&92) {
                    volume.pop();
                }
                if let Some(device) = self.cache.get(&volume) {
                    return *device;
                }
                let mut name = volume.clone();
                name.push(0);
                let device = match CreateFileW(
                    PCWSTR(name.as_ptr()),
                    0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                ) {
                    Ok(handle) => {
                        let mut number = STORAGE_DEVICE_NUMBER::default();
                        let mut returned = 0;
                        let result = DeviceIoControl(
                            handle,
                            IOCTL_STORAGE_GET_DEVICE_NUMBER,
                            None,
                            0,
                            Some((&mut number as *mut STORAGE_DEVICE_NUMBER).cast()),
                            std::mem::size_of_val(&number) as u32,
                            Some(&mut returned),
                            None,
                        );
                        // FILE_DEVICE_DISK. Unknown/unsupported aggregate devices stay conservative.
                        let disk = (result.is_ok()
                            && returned as usize >= std::mem::size_of_val(&number)
                            && number.DeviceType == 7)
                            .then_some(number.DeviceNumber);
                        let query = STORAGE_PROPERTY_QUERY {
                            PropertyId: StorageDeviceSeekPenaltyProperty,
                            QueryType: PropertyStandardQuery,
                            ..Default::default()
                        };
                        let mut descriptor = DEVICE_SEEK_PENALTY_DESCRIPTOR::default();
                        returned = 0;
                        let result = DeviceIoControl(
                            handle,
                            IOCTL_STORAGE_QUERY_PROPERTY,
                            Some((&query as *const STORAGE_PROPERTY_QUERY).cast()),
                            std::mem::size_of_val(&query) as u32,
                            Some((&mut descriptor as *mut DEVICE_SEEK_PENALTY_DESCRIPTOR).cast()),
                            std::mem::size_of_val(&descriptor) as u32,
                            Some(&mut returned),
                            None,
                        );
                        let kind = if result.is_ok()
                            && returned as usize >= std::mem::size_of_val(&descriptor)
                        {
                            if descriptor.IncursSeekPenalty {
                                Kind::Hdd
                            } else {
                                Kind::Ssd
                            }
                        } else {
                            Kind::Unknown
                        };
                        let _ = CloseHandle(handle);
                        Device { disk, kind }
                    }
                    Err(_) => unknown,
                };
                self.cache.insert(volume, device);
                device
            }
        }
    }
    let mut probe = Probe {
        cache: BTreeMap::new(),
    };
    let mut directory = vec![0u16; 32768];
    let length = unsafe { GetSystemWindowsDirectoryW(Some(&mut directory)) } as usize;
    let system = if length > 0 && length < directory.len() {
        probe
            .device(Path::new(&std::ffi::OsString::from_wide(
                &directory[..length],
            )))
            .disk
    } else {
        None
    };
    let destination = probe.device(destination);
    let sources: Vec<_> = sources.iter().map(|path| probe.device(path)).collect();
    choose(&sources, destination, system)
}

#[cfg(not(windows))]
pub(super) fn profile(sources: &[PathBuf], _: &Path) -> Policy {
    let unknown = Device {
        disk: None,
        kind: Kind::Unknown,
    };
    choose(&vec![unknown; sources.len()], unknown, None)
}
