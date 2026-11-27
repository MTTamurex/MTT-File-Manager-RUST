use super::{
    drive_scope_matches, merge_changed_drive_info, record_successful_virtual_drive_info_query,
    should_query_drive_info, DRIVE_INFO_REPAINT_COALESCE,
};
use crate::app::drive_state::{
    DriveInfoRefreshEntry, DriveInfoRefreshResult, DriveInfoRefreshScope, HardwareSchedule,
};
use crate::domain::file_entry::DriveInfo;
use crate::infrastructure::windows::{self, DriveType};
use eframe::egui;
use std::sync::{mpsc, Arc};
use std::time::Instant;

pub(super) struct Refresh {
    pub scope: DriveInfoRefreshScope,
    pub generation: u64,
    pub disks: Vec<(String, Option<DriveInfo>)>,
    pub tx: mpsc::Sender<DriveInfoRefreshResult>,
    pub ctx: egui::Context,
    pub hardware: Arc<parking_lot::Mutex<HardwareSchedule>>,
    /// Periodic refresh: completion without any published change does not wake
    /// the UI. The next scheduled pass consumes it, so an idle window does not
    /// render (and present on the GPU) an identical frame.
    pub quiet: bool,
}

pub(super) fn spawn(refresh: Refresh) {
    let tx = refresh.tx.clone();
    let ctx = refresh.ctx.clone();
    let scope = refresh.scope;
    let generation = refresh.generation;
    if let Err(error) = std::thread::Builder::new()
        .name("drive-info-refresh".into())
        .spawn(move || refresh.run(&NativeQueries))
    {
        log::error!("[DRIVE-REFRESH] Failed to start worker: {error}");
        complete(&tx, &ctx, scope, generation, true);
    }
}

trait Queries {
    fn drive_type(&self, path: &str) -> DriveType;
    fn capacity(&self, path: &str) -> windows::VolumeInfo;
    fn hardware(
        &self,
        path: &str,
        drive_type: DriveType,
    ) -> windows::physical_drive_info::HardwareFields;
}

struct NativeQueries;

impl Queries for NativeQueries {
    fn drive_type(&self, path: &str) -> DriveType {
        windows::detect_drive_type(path)
    }
    fn capacity(&self, path: &str) -> windows::VolumeInfo {
        windows::get_volume_info(path)
    }
    fn hardware(
        &self,
        path: &str,
        drive_type: DriveType,
    ) -> windows::physical_drive_info::HardwareFields {
        windows::query_hardware_fields(path, drive_type)
    }
}

impl Refresh {
    fn run(self, queries: &impl Queries) {
        let published_any =
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.query(queries))) {
                Ok(published_any) => published_any,
                Err(_) => {
                    log::error!("[DRIVE-REFRESH] Volume info worker panicked");
                    true
                }
            };
        // Completion is required even without visual changes or after panic:
        // consuming it releases the tracker slot and unblocks startup/preloads.
        complete(
            &self.tx,
            &self.ctx,
            self.scope,
            self.generation,
            completion_wakes_ui(self.quiet, published_any),
        );
    }

    fn publish(&self, entry: DriveInfoRefreshEntry) -> bool {
        if self
            .tx
            .send(DriveInfoRefreshResult {
                scope: self.scope,
                generation: self.generation,
                entries: vec![entry],
                complete: false,
            })
            .is_err()
        {
            return false;
        }
        self.ctx.request_repaint_after(DRIVE_INFO_REPAINT_COALESCE);
        true
    }

    /// Returns whether any changed entry was published.
    fn query(&self, queries: &impl Queries) -> bool {
        let mut published_any = false;
        let mut hardware_jobs = Vec::new();
        for (path, cached) in &self.disks {
            let drive_type = queries.drive_type(path);
            if !drive_scope_matches(self.scope, drive_type) {
                continue;
            }
            let has_capacity = cached.as_ref().is_some_and(|info| info.total_space > 0);
            if !should_query_drive_info(path, has_capacity, Instant::now()) {
                continue;
            }
            let start = Instant::now();
            let volume = queries.capacity(path);
            if volume.capacity_query_succeeded {
                record_successful_virtual_drive_info_query(path, Instant::now());
            }
            record_query_time(start);
            let entry = DriveInfoRefreshEntry {
                path: path.clone(),
                capacity_query_succeeded: volume.capacity_query_succeeded,
                info: DriveInfo {
                    file_system: volume.file_system,
                    total_space: volume.total_space,
                    free_space: volume.free_space,
                    ..empty_info(drive_type)
                },
            };
            if merge_changed_drive_info(
                cached.as_ref(),
                entry.info.clone(),
                entry.capacity_query_succeeded,
            )
            .is_some()
            {
                if !self.publish(entry) {
                    return published_any;
                }
                published_any = true;
            }
            // Metadata cadence is independent of completeness of cached fields.
            // Empty/partial responses therefore recover, while complete ones
            // are still refreshed occasionally rather than frozen forever.
            if matches!(drive_type, DriveType::Fixed | DriveType::Removable) {
                hardware_jobs.push((path, cached, drive_type));
            }
        }
        for (path, cached, drive_type) in hardware_jobs {
            if !self
                .hardware
                .lock()
                .claim(path, self.generation, Instant::now())
            {
                continue;
            }
            let start = Instant::now();
            let fields = queries.hardware(path, drive_type);
            record_query_time(start);
            let entry = DriveInfoRefreshEntry {
                path: path.clone(),
                capacity_query_succeeded: false,
                info: DriveInfo {
                    model: fields.model,
                    serial_number: fields.serial_number,
                    firmware_revision: fields.firmware_revision,
                    bus_type: fields.bus_type,
                    ..empty_info(drive_type)
                },
            };
            // Compare merged values, but transmit only queried fields: including
            // the old cache would overwrite newer capacity or health metadata.
            if merge_changed_drive_info(cached.as_ref(), entry.info.clone(), false).is_some() {
                if !self.publish(entry) {
                    return published_any;
                }
                published_any = true;
            }
        }
        published_any
    }
}

fn completion_wakes_ui(quiet: bool, published_any: bool) -> bool {
    !quiet || published_any
}

fn empty_info(drive_type: DriveType) -> DriveInfo {
    DriveInfo {
        file_system: String::new(),
        total_space: 0,
        free_space: 0,
        drive_type,
        model: None,
        serial_number: None,
        firmware_revision: None,
        bus_type: None,
        health: None,
    }
}

fn record_query_time(start: Instant) {
    crate::infrastructure::io_trace::record_drive_info_query(start.elapsed().as_millis() as u64);
}

fn complete(
    tx: &mpsc::Sender<DriveInfoRefreshResult>,
    ctx: &egui::Context,
    scope: DriveInfoRefreshScope,
    generation: u64,
    wake_ui: bool,
) {
    if tx
        .send(DriveInfoRefreshResult {
            scope,
            generation,
            entries: Vec::new(),
            complete: true,
        })
        .is_ok()
        && wake_ui
    {
        ctx.request_repaint_after(DRIVE_INFO_REPAINT_COALESCE);
    }
}

#[cfg(test)]
mod tests;
