//! Static system specification snapshot (Windows 11 "System > About" style).
//!
//! Collected once per process on first access via native Windows APIs:
//! registry (CPU/OS), `GetComputerNameW` (device name),
//! `GlobalMemoryStatusEx` (RAM) and DXGI (GPU). All queries are fast,
//! non-blocking system calls; failures degrade to `None`/empty fields.

use std::sync::OnceLock;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, RRF_RT_REG_DWORD,
    RRF_RT_REG_SZ,
};
use windows::Win32::System::SystemInformation::{
    FirmwareTypeBios, FirmwareTypeUefi, GetFirmwareType, GlobalMemoryStatusEx, FIRMWARE_TYPE,
    MEMORYSTATUSEX,
};
use windows::Win32::System::WindowsProgramming::GetComputerNameW;

const CPU_KEY: &str = "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0";
const OS_KEY: &str = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
const BIOS_KEY: &str = "HARDWARE\\DESCRIPTION\\System\\BIOS";

/// Immutable snapshot of the machine's hardware/OS identity.
#[derive(Default)]
pub struct SystemSpec {
    pub device_name: String,
    pub cpu_name: String,
    pub cpu_clock_mhz: Option<u32>,
    pub total_ram_bytes: u64,
    pub gpu_name: Option<String>,
    pub gpu_vram_bytes: Option<u64>,
    pub os_edition: String,
    pub os_version: String,
    pub os_build: String,
    pub board_manufacturer: String,
    pub board_product: String,
    pub bios_vendor: String,
    pub bios_version: String,
    pub bios_release_date: String,
    /// `Some(true)` = UEFI, `Some(false)` = legacy BIOS, `None` = unknown.
    pub firmware_is_uefi: Option<bool>,
}

static SYSTEM_SPEC: OnceLock<SystemSpec> = OnceLock::new();

/// Returns the process-wide system spec snapshot, collecting it on first use.
pub fn get_system_spec() -> &'static SystemSpec {
    SYSTEM_SPEC.get_or_init(collect_system_spec)
}

impl SystemSpec {
    /// "13th Gen Intel(R) Core(TM) i5-13400 (2.50 GHz)"
    pub fn cpu_display(&self) -> String {
        match (self.cpu_name.is_empty(), self.cpu_clock_mhz) {
            (false, Some(mhz)) => format!("{} ({:.2} GHz)", self.cpu_name, mhz as f64 / 1000.0),
            (false, None) => self.cpu_name.clone(),
            (true, Some(mhz)) => format!("{:.2} GHz", mhz as f64 / 1000.0),
            (true, None) => String::new(),
        }
    }

    /// "NVIDIA GeForce RTX 3060 Ti (8 GB)"
    pub fn gpu_display(&self) -> Option<String> {
        let name = self.gpu_name.as_ref()?;
        Some(match self.gpu_vram_bytes {
            Some(bytes) if bytes > 0 => format!("{} ({} GB)", name, rounded_gb(bytes)),
            _ => name.clone(),
        })
    }
}

fn rounded_gb(bytes: u64) -> u64 {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    ((bytes as f64 / GB).round() as u64).max(1)
}

fn collect_system_spec() -> SystemSpec {
    let cpu_name = read_reg_sz(CPU_KEY, "ProcessorNameString")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    let cpu_clock_mhz = read_reg_dword(CPU_KEY, "~MHz");

    let os_build_number = read_reg_sz(OS_KEY, "CurrentBuild")
        .and_then(|b| b.parse::<u32>().ok())
        .unwrap_or(0);

    let mut os_edition = read_reg_sz(OS_KEY, "ProductName")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    // The registry misreports "Windows 10" on Windows 11 machines;
    // build >= 22000 is the authoritative Windows 11 marker.
    if os_build_number >= 22000 {
        os_edition = os_edition.replace("Windows 10", "Windows 11");
    }

    let os_version = read_reg_sz(OS_KEY, "DisplayVersion")
        .or_else(|| read_reg_sz(OS_KEY, "ReleaseId"))
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    let os_build = match (
        read_reg_sz(OS_KEY, "CurrentBuild"),
        read_reg_dword(OS_KEY, "UBR"),
    ) {
        (Some(build), Some(ubr)) => format!("{}.{}", build.trim(), ubr),
        (Some(build), None) => build.trim().to_string(),
        (None, _) => String::new(),
    };

    SystemSpec {
        device_name: read_device_name(),
        cpu_name,
        cpu_clock_mhz,
        total_ram_bytes: read_total_ram(),
        gpu_name: None,
        gpu_vram_bytes: None,
        board_manufacturer: read_reg_trim(BIOS_KEY, "BaseBoardManufacturer"),
        board_product: read_reg_trim(BIOS_KEY, "BaseBoardProduct"),
        bios_vendor: read_reg_trim(BIOS_KEY, "BIOSVendor"),
        bios_version: read_reg_trim(BIOS_KEY, "BIOSVersion"),
        bios_release_date: read_reg_trim(BIOS_KEY, "BIOSReleaseDate"),
        firmware_is_uefi: read_firmware_type(),
        ..Default::default()
    }
    .with_gpu(read_primary_gpu())
    .with_os(os_edition, os_version, os_build)
}

// Small builder-style helpers keep collect_system_spec readable.
impl SystemSpec {
    fn with_gpu(mut self, gpu: Option<(String, u64)>) -> Self {
        if let Some((name, vram)) = gpu {
            self.gpu_name = Some(name);
            self.gpu_vram_bytes = Some(vram);
        }
        self
    }

    fn with_os(mut self, edition: String, version: String, build: String) -> Self {
        self.os_edition = edition;
        self.os_version = version;
        self.os_build = build;
        self
    }
}

fn read_reg_trim(key_path: &str, value_name: &str) -> String {
    read_reg_sz(key_path, value_name)
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn read_firmware_type() -> Option<bool> {
    unsafe {
        let mut firmware_type = FIRMWARE_TYPE::default();
        GetFirmwareType(&mut firmware_type).ok()?;
        if firmware_type == FirmwareTypeUefi {
            Some(true)
        } else if firmware_type == FirmwareTypeBios {
            Some(false)
        } else {
            None
        }
    }
}

fn read_device_name() -> String {
    unsafe {
        let mut buffer = [0u16; 32];
        let mut size = buffer.len() as u32;
        if GetComputerNameW(Some(PWSTR(buffer.as_mut_ptr())), &mut size).is_ok() {
            return String::from_utf16_lossy(&buffer[..size as usize]);
        }
        String::new()
    }
}

fn read_total_ram() -> u64 {
    unsafe {
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        if GlobalMemoryStatusEx(&mut status).is_ok() {
            status.ullTotalPhys
        } else {
            0
        }
    }
}

/// First non-software DXGI adapter (description, dedicated VRAM bytes).
fn read_primary_gpu() -> Option<(String, u64)> {
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let mut index = 0u32;
        loop {
            let adapter = match factory.EnumAdapters1(index) {
                Ok(adapter) => adapter,
                Err(err) if err.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(_) => break,
            };
            index += 1;
            let desc = match adapter.GetDesc1() {
                Ok(desc) => desc,
                Err(_) => continue,
            };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            let end = desc
                .Description
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(desc.Description.len());
            let name = String::from_utf16_lossy(&desc.Description[..end]);
            if name.is_empty() {
                continue;
            }
            return Some((name, desc.DedicatedVideoMemory as u64));
        }
        None
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open_hklm(key_path: &str) -> Option<HKEY> {
    unsafe {
        let key_wide = wide(key_path);
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key_wide.as_ptr()),
            Some(0),
            KEY_READ,
            &mut hkey,
        )
        .is_err()
        {
            return None;
        }
        Some(hkey)
    }
}

fn read_reg_sz(key_path: &str, value_name: &str) -> Option<String> {
    unsafe {
        let hkey = open_hklm(key_path)?;
        let value_wide = wide(value_name);

        let mut size = 0u32;
        let result = RegGetValueW(
            hkey,
            PCWSTR::null(),
            PCWSTR(value_wide.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        );
        if result.is_err() || size == 0 {
            let _ = RegCloseKey(hkey);
            return None;
        }

        let mut buffer: Vec<u16> = vec![0; (size / 2) as usize];
        let result = RegGetValueW(
            hkey,
            PCWSTR::null(),
            PCWSTR(value_wide.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr() as *mut _),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);
        if result.is_err() {
            return None;
        }

        let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        Some(String::from_utf16_lossy(&buffer[..len]))
    }
}

fn read_reg_dword(key_path: &str, value_name: &str) -> Option<u32> {
    unsafe {
        let hkey = open_hklm(key_path)?;
        let value_wide = wide(value_name);

        let mut value = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let result = RegGetValueW(
            hkey,
            PCWSTR::null(),
            PCWSTR(value_wide.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey);
        if result.is_err() {
            return None;
        }
        Some(value)
    }
}
