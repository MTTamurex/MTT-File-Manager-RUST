use std::sync::Arc;
use std::thread::JoinHandle;

use crate::indexing_progress::IndexingProgress;
use crate::volume_indices::VolumeIndexHandle;
use crate::volume_lifecycle::VolumeStopSignal;

pub(super) fn spawn_background_size_refresh(
    drive_letter: char,
    bg_handle: VolumeIndexHandle,
    indexing_progress: Arc<IndexingProgress>,
    stop: VolumeStopSignal,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        if stop.is_stopped() {
            return;
        }
        let _active_operation =
            crate::memory_trim::begin_active_operation("background MFT size extraction");
        let Some(_build_permit) = super::super::wait_for_index_build(&stop) else {
            return;
        };
        crate::memory_trim::log_process_memory(&format!(
            "{}:\\ before background MFT size extraction",
            drive_letter
        ));
        eprintln!(
            "[MFT-SIZE] {}:\\ Background size extraction via sizes-only MFT read...",
            drive_letter,
        );
        let start = std::time::Instant::now();
        let files_indexed = bg_handle.read().records.len() as u64;
        let bulk_result = super::volume_io::with_volume(drive_letter, &stop, |bg_volume| {
            crate::mft_reader::read_mft_sizes_bulk(
                bg_volume,
                drive_letter,
                |done, total| {
                    indexing_progress.update(
                        drive_letter,
                        "scanning",
                        files_indexed,
                        "loading_sizes",
                        Some(done),
                        Some(total),
                    );
                },
                || stop.is_stopped(),
            )
        });
        if stop.is_stopped() {
            return;
        }

        match bulk_result {
            Ok(size_updates) => {
                let mut applied = 0u64;
                let mut sizes_marked = false;
                if let Some(mut vol) =
                    bg_handle.try_write_for(std::time::Duration::from_millis(250))
                {
                    let mut changed = false;
                    vol.records.prepare_bulk_mutation(size_updates.len());
                    for (position, (frn, metrics)) in size_updates.iter().enumerate() {
                        if position.is_multiple_of(1024) && stop.is_stopped() {
                            return;
                        }
                        if let Some(rec) = vol.records.get_mut(frn) {
                            let new_size = rec.size.max(metrics.logical_size);
                            let new_allocated = rec.allocated_size.max(metrics.allocated_size);
                            if (rec.size, rec.allocated_size) != (new_size, new_allocated) {
                                rec.size = new_size;
                                rec.allocated_size = new_allocated;
                                applied += 1;
                                changed = true;
                            }
                        }
                    }
                    if !vol.sizes_loaded {
                        changed = true;
                    }
                    vol.sizes_loaded = true;
                    if changed {
                        vol.binary_dirty = true;
                    }
                    sizes_marked = true;
                }
                if !sizes_marked {
                    loop {
                        if stop.is_stopped() {
                            break;
                        }
                        if let Some(mut vol) =
                            bg_handle.try_write_for(std::time::Duration::from_millis(250))
                        {
                            let mut changed = false;
                            vol.records.prepare_bulk_mutation(size_updates.len());
                            for (position, (frn, metrics)) in size_updates.iter().enumerate() {
                                if position.is_multiple_of(1024) && stop.is_stopped() {
                                    return;
                                }
                                if let Some(rec) = vol.records.get_mut(frn) {
                                    let new_size = rec.size.max(metrics.logical_size);
                                    let new_allocated =
                                        rec.allocated_size.max(metrics.allocated_size);
                                    if (rec.size, rec.allocated_size) != (new_size, new_allocated) {
                                        rec.size = new_size;
                                        rec.allocated_size = new_allocated;
                                        applied += 1;
                                        changed = true;
                                    }
                                }
                            }
                            if !vol.sizes_loaded {
                                changed = true;
                            }
                            vol.sizes_loaded = true;
                            if changed {
                                vol.binary_dirty = true;
                            }
                            break;
                        }
                    }
                }
                if stop.is_stopped() {
                    return;
                }
                eprintln!(
                    "[MFT-SIZE] {}:\\ Bulk size extraction complete: {} sizes updated in {:.2}s",
                    drive_letter,
                    applied,
                    start.elapsed().as_secs_f64()
                );
                if stop.is_stopped() {
                    return;
                }
                super::persistence::flush_binary_snapshot_if_dirty(&bg_handle, drive_letter);
            }
            Err(e) => eprintln!(
                "[MFT-SIZE] {}:\\ Bulk size extraction failed: {}, keeping sizes unloaded",
                drive_letter, e
            ),
        }

        crate::memory_trim::request_trim(format!("{}:\\ background size extraction", drive_letter));
        crate::memory_trim::log_process_memory(&format!(
            "{}:\\ after background MFT size extraction",
            drive_letter
        ));
        indexing_progress.clear(drive_letter);
    })
}
