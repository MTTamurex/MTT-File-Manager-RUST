mod ata;
mod nvme;
mod pcie;
mod protocol_io;
mod windows_io;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use mtt_search_protocol::{DriveHealthSnapshot, DriveHealthState};

use protocol_io::{query_protocol_data, ProtocolQuery};
use windows_io::{BUS_TYPE_ATA, BUS_TYPE_ATAPI, BUS_TYPE_NVME, BUS_TYPE_SATA, BUS_TYPE_USB};

const STORAGE_ADAPTER_PROTOCOL_SPECIFIC_PROPERTY: u32 = 49;
const STORAGE_DEVICE_PROTOCOL_SPECIFIC_PROPERTY: u32 = 50;
const PROTOCOL_TYPE_ATA: u32 = 2;
const PROTOCOL_TYPE_NVME: u32 = 3;
const DATA_TYPE_IDENTIFY: u32 = 1;
const NVME_DATA_TYPE_LOG_PAGE: u32 = 2;
const QUERY_TIMEOUT: Duration = Duration::from_secs(19);
static QUERY_ACTIVE: AtomicBool = AtomicBool::new(false);
static QUERY_BACKGROUND: AtomicBool = AtomicBool::new(false);

fn query_is_background() -> bool {
    QUERY_BACKGROUND.load(Ordering::Acquire)
}

fn set_background_thread_priority() {
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN, THREAD_PRIORITY_LOWEST,
    };

    unsafe {
        let thread = GetCurrentThread();
        let _ = SetThreadPriority(thread, THREAD_MODE_BACKGROUND_BEGIN);
        let _ = SetThreadPriority(thread, THREAD_PRIORITY_LOWEST);
    }
}

pub(crate) fn query(drive_letter: char, background: bool) -> Result<DriveHealthSnapshot, String> {
    if QUERY_ACTIVE
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("another drive health query is still active".to_string());
    }
    QUERY_BACKGROUND.store(background, Ordering::Release);

    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let worker = std::thread::Builder::new()
        .name("drive-health-query".to_string())
        .spawn(move || {
            if background {
                set_background_thread_priority();
            }
            struct QueryPermit;

            impl Drop for QueryPermit {
                fn drop(&mut self) {
                    QUERY_BACKGROUND.store(false, Ordering::Release);
                    QUERY_ACTIVE.store(false, Ordering::Release);
                }
            }

            let _permit = QueryPermit;
            let _ = result_tx.send(query_inner(drive_letter));
        })
        .map_err(|error| {
            QUERY_BACKGROUND.store(false, Ordering::Release);
            QUERY_ACTIVE.store(false, Ordering::Release);
            format!("drive health worker creation failed: {error}")
        })?;

    match result_rx.recv_timeout(QUERY_TIMEOUT) {
        Ok(result) => {
            let _ = worker.join();
            result
        }
        // If an overlapped driver ignores cancellation, this worker keeps
        // QUERY_ACTIVE set and owns every pending buffer/handle. Pass-through has
        // its own singleton permit for the same fail-closed guarantee. In either
        // case, the IPC handler stops waiting before its 20-second budget.
        Err(mpsc::RecvTimeoutError::Timeout) => Err("drive health query timed out".to_string()),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = worker.join();
            Err("drive health worker stopped unexpectedly".to_string())
        }
    }
}

fn query_inner(drive_letter: char) -> Result<DriveHealthSnapshot, String> {
    let drive_letter = drive_letter.to_ascii_uppercase();
    let device = windows_io::open_device(drive_letter)?;
    let snapshot = match device.bus_type {
        BUS_TYPE_NVME => query_nvme(drive_letter, &device)?,
        BUS_TYPE_ATAPI | BUS_TYPE_ATA | BUS_TYPE_USB | BUS_TYPE_SATA => {
            query_ata(drive_letter, &device)?
        }
        bus_type => return Err(format!("unsupported storage bus type {bus_type}")),
    };
    snapshot
        .validate()
        .map_err(|error| format!("generated drive health response is invalid: {error}"))?;
    Ok(snapshot)
}

fn query_nvme(
    drive_letter: char,
    device: &windows_io::DeviceContext,
) -> Result<DriveHealthSnapshot, String> {
    let identify = query_protocol_data(
        device.physical_handle.raw(),
        ProtocolQuery {
            property_id: STORAGE_ADAPTER_PROTOCOL_SPECIFIC_PROPERTY,
            protocol_type: PROTOCOL_TYPE_NVME,
            data_type: DATA_TYPE_IDENTIFY,
            request_value: 1,
            request_subvalue: 0,
            data_len: 4096,
        },
    )?;
    let identity = nvme::parse_identify(&identify)?;
    let health = [
        STORAGE_DEVICE_PROTOCOL_SPECIFIC_PROPERTY,
        STORAGE_ADAPTER_PROTOCOL_SPECIFIC_PROPERTY,
    ]
    .into_iter()
    .find_map(|property_id| {
        [0, u32::MAX].into_iter().find_map(|subvalue| {
            query_protocol_data(
                device.physical_handle.raw(),
                ProtocolQuery {
                    property_id,
                    protocol_type: PROTOCOL_TYPE_NVME,
                    data_type: NVME_DATA_TYPE_LOG_PAGE,
                    request_value: 0x02,
                    request_subvalue: subvalue,
                    data_len: 512,
                },
            )
            .ok()
            .and_then(|data| nvme::parse_health(&data).ok())
        })
    });

    let mut features = identity.features;
    if health.is_some() {
        features.push("SMART".to_string());
    }
    let link_modes = pcie::query(device.physical_disk_number);
    let mut snapshot = match health {
        Some(health) => DriveHealthSnapshot {
            drive_letter,
            physical_disk_number: device.physical_disk_number,
            model: identity.model.or_else(|| device.descriptor_model.clone()),
            serial_number: identity.serial.or_else(|| device.descriptor_serial.clone()),
            firmware_revision: identity
                .firmware
                .or_else(|| device.descriptor_firmware.clone()),
            interface: Some(device.interface.clone()),
            temperature_celsius: health.temperature_celsius,
            life_remaining_percent: health.life_remaining_percent,
            total_host_reads_bytes: Some(health.total_host_reads_bytes),
            total_host_writes_bytes: Some(health.total_host_writes_bytes),
            power_cycle_count: Some(health.power_cycle_count),
            power_on_hours: Some(health.power_on_hours),
            rotation: nvme::rotation(),
            current_transfer_mode: None,
            max_transfer_mode: None,
            standard: identity.standard,
            features,
            health_state: health.health_state,
            smart_available: true,
        },
        None => DriveHealthSnapshot {
            drive_letter,
            physical_disk_number: device.physical_disk_number,
            model: identity.model.or_else(|| device.descriptor_model.clone()),
            serial_number: identity.serial.or_else(|| device.descriptor_serial.clone()),
            firmware_revision: identity
                .firmware
                .or_else(|| device.descriptor_firmware.clone()),
            interface: Some(device.interface.clone()),
            temperature_celsius: None,
            life_remaining_percent: None,
            total_host_reads_bytes: None,
            total_host_writes_bytes: None,
            power_cycle_count: None,
            power_on_hours: None,
            rotation: nvme::rotation(),
            current_transfer_mode: None,
            max_transfer_mode: None,
            standard: identity.standard,
            features,
            health_state: DriveHealthState::Unknown,
            smart_available: false,
        },
    };
    if let Some(link_modes) = link_modes {
        snapshot.current_transfer_mode = link_modes.current;
        snapshot.max_transfer_mode = link_modes.maximum;
    }
    Ok(snapshot)
}

fn query_ata(
    drive_letter: char,
    device: &windows_io::DeviceContext,
) -> Result<DriveHealthSnapshot, String> {
    let identity = query_protocol_data(
        device.physical_handle.raw(),
        ProtocolQuery {
            property_id: STORAGE_DEVICE_PROTOCOL_SPECIFIC_PROPERTY,
            protocol_type: PROTOCOL_TYPE_ATA,
            data_type: DATA_TYPE_IDENTIFY,
            request_value: 0,
            request_subvalue: 0,
            data_len: 512,
        },
    )
    .and_then(|identify| ata::parse_identify(&identify))
    .or_else(|_| {
        if device.bus_type == BUS_TYPE_USB {
            protocol_io::sat_identify(drive_letter)
        } else {
            protocol_io::ata_identify(device.physical_disk_number)
        }
        .and_then(|identify| ata::parse_identify(&identify))
    })?;

    let smart_data = if device.bus_type == BUS_TYPE_USB {
        protocol_io::sat_smart_read(drive_letter, 0xD0)
    } else {
        protocol_io::ata_smart_read(device.physical_disk_number, 0xD0)
    };
    let health = smart_data.ok().and_then(|data| {
        let threshold_data = if device.bus_type == BUS_TYPE_USB {
            protocol_io::sat_smart_read(drive_letter, 0xD1)
        } else {
            protocol_io::ata_smart_read(device.physical_disk_number, 0xD1)
        };
        ata::parse_smart(
            &data,
            threshold_data.as_ref().ok().map(|v| &v[..]),
            &identity.rotation,
        )
        .ok()
    });
    let smart_available = health.is_some();

    Ok(DriveHealthSnapshot {
        drive_letter,
        physical_disk_number: device.physical_disk_number,
        model: identity.model.or_else(|| device.descriptor_model.clone()),
        serial_number: identity.serial.or_else(|| device.descriptor_serial.clone()),
        firmware_revision: identity
            .firmware
            .or_else(|| device.descriptor_firmware.clone()),
        interface: Some(device.interface.clone()),
        temperature_celsius: health
            .as_ref()
            .and_then(|health| health.temperature_celsius),
        life_remaining_percent: None,
        total_host_reads_bytes: None,
        total_host_writes_bytes: None,
        power_cycle_count: health.as_ref().and_then(|health| health.power_cycle_count),
        power_on_hours: health.as_ref().and_then(|health| health.power_on_hours),
        rotation: identity.rotation,
        current_transfer_mode: identity.current_transfer_mode,
        max_transfer_mode: identity.max_transfer_mode,
        standard: identity.standard,
        features: identity.features,
        health_state: health
            .map(|health| health.health_state)
            .unwrap_or(DriveHealthState::Unknown),
        smart_available,
    })
}
