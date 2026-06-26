use crate::infrastructure::app_state_db::AppStateDb;
use crate::infrastructure::icon_disk_cache::IconDiskCache;
use crate::infrastructure::windows as windows_infra;
use crate::infrastructure::windows::is_mpeg_ts_file;
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{mpsc, Arc};

type IconRequest = (PathBuf, usize);
type IconResponse = (PathBuf, usize, Vec<u8>, u32, u32);
type MetadataRequest = (PathBuf, u64);
type MetadataResponse = (PathBuf, u64, windows_infra::MediaMetadata);
type FileHashRequestIn = (PathBuf, u64, u64);
type FileHashResponseIn = (PathBuf, u64, u64, Result<String, String>);

pub(in crate::app) fn spawn_cover_worker(
    app_state_db: Arc<AppStateDb>,
) -> (
    mpsc::Sender<PathBuf>,
    mpsc::Receiver<(PathBuf, Option<PathBuf>)>,
) {
    let (cover_req_tx, cover_req_rx) = mpsc::channel::<PathBuf>();
    let (cover_res_tx, cover_res_rx) = mpsc::channel();

    let cover_worker_db = app_state_db.clone();
    std::thread::spawn(move || {
        crate::infrastructure::io_priority::set_thread_priority(
            crate::infrastructure::io_priority::IOPriority::Background,
        );

        while let Ok(folder_path) = cover_req_rx.recv() {
            let cover = windows_infra::find_folder_preview_item(&folder_path).filter(|p| {
                // Reject .ts files that aren't real MPEG-TS video.
                // Real MPEG-TS starts with sync byte 0x47.
                if p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("ts"))
                {
                    return is_mpeg_ts_file(p);
                }
                true
            });

            if let Some(c) = &cover {
                cover_worker_db.set_folder_cover(&folder_path, c);
            }

            let _ = cover_res_tx.send((folder_path, cover));
        }
    });

    (cover_req_tx, cover_res_rx)
}

pub(in crate::app) fn spawn_async_font_loader() -> mpsc::Receiver<egui::FontDefinitions> {
    let (font_tx, font_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut fonts = eframe::egui::FontDefinitions::default();
        let mut loaded_fonts = Vec::new();
        let windows_dir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        let fonts_dir = windows_dir.join("Fonts");

        let segoe_path = fonts_dir.join("segoeui.ttf");
        if let Ok(font_data) = std::fs::read(&segoe_path) {
            fonts.font_data.insert(
                "segoe_ui".to_owned(),
                std::sync::Arc::new(eframe::egui::FontData::from_owned(font_data)),
            );
            loaded_fonts.push("segoe_ui".to_owned());
        }

        let symbol_path = fonts_dir.join("seguisym.ttf");
        if let Ok(font_data) = std::fs::read(&symbol_path) {
            fonts.font_data.insert(
                "segoe_ui_symbol".to_owned(),
                std::sync::Arc::new(eframe::egui::FontData::from_owned(font_data)),
            );
            loaded_fonts.push("segoe_ui_symbol".to_owned());
        }

        // CJK fallback: Windows Explorer uses system font fallback, but egui needs
        // explicit font files. Load one font per major CJK locale so Simplified
        // Chinese, Traditional Chinese, Japanese, and Korean filenames render.
        let cjk_fallback_groups = [
            &[("ms_yahei", "msyh.ttc")][..],
            &[("ms_jhenghei", "msjh.ttc")][..],
            &[("yu_gothic", "YuGothR.ttc"), ("ms_gothic", "msgothic.ttc")][..],
            &[("malgun", "malgun.ttf")][..],
        ];

        for group in cjk_fallback_groups {
            for (key, file_name) in group {
                let cjk_path = fonts_dir.join(file_name);
                if let Ok(font_data) = std::fs::read(&cjk_path) {
                    fonts.font_data.insert(
                        key.to_string(),
                        std::sync::Arc::new(eframe::egui::FontData::from_owned(font_data)),
                    );
                    loaded_fonts.push(key.to_string());
                    break;
                }
            }
        }

        {
            let data = crate::embedded_assets::REMIXICON_TTF.to_vec();
            fonts.font_data.insert(
                "remix_icon".to_owned(),
                std::sync::Arc::new(eframe::egui::FontData::from_owned(data)),
            );
            fonts.families.insert(
                eframe::egui::FontFamily::Name("icons".into()),
                vec!["remix_icon".to_owned()],
            );
        }

        if !loaded_fonts.is_empty() {
            if let Some(proportional) = fonts
                .families
                .get_mut(&eframe::egui::FontFamily::Proportional)
            {
                proportional.extend(loaded_fonts.clone());
            }

            if let Some(monospace) = fonts.families.get_mut(&eframe::egui::FontFamily::Monospace) {
                monospace.extend(loaded_fonts.clone());
            }
        }

        let _ = font_tx.send(fonts);
    });
    font_rx
}

pub(in crate::app) fn spawn_icon_worker(
    ctx: &egui::Context,
    current_generation: Arc<AtomicUsize>,
    icon_disk_cache: Arc<IconDiskCache>,
) -> (mpsc::Sender<IconRequest>, mpsc::Receiver<IconResponse>) {
    let (icon_req_tx, icon_req_rx_thread) = mpsc::channel::<IconRequest>();
    let (fanout_tx, fanout_rx) = crossbeam_channel::bounded::<IconRequest>(256);
    let (icon_res_tx, icon_res_rx) = mpsc::channel::<IconResponse>();

    // Keep std::sync::mpsc sender API for the app state, but fan-out requests into
    // a cloneable crossbeam receiver so icon workers consume truly in parallel.
    std::thread::spawn(move || {
        while let Ok(req) = icon_req_rx_thread.recv() {
            if fanout_tx.send(req).is_err() {
                break;
            }
        }
    });

    // Shared extension icon cache across all workers. This is session-only:
    // persistent icon caching is reserved for per-file icons whose pixels vary
    // by source file (programs, shortcuts, .ico, etc.). Shared extension icons
    // should always follow the current Windows Shell state on a fresh launch.
    #[allow(clippy::type_complexity)]
    let shared_ext_cache: Arc<dashmap::DashMap<String, (Vec<u8>, u32, u32)>> =
        Arc::new(dashmap::DashMap::with_capacity(32));

    let cpu = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    // SHGetFileInfoW is IO-bound (registry + COM); ~4 STA threads are enough
    // to saturate the Shell pipeline. Higher counts add committed stack RAM
    // (each OS thread commits ~1 MB by default) without parallel throughput
    // gains, so cap at 4 and explicitly request a 256 KB stack per worker.
    let worker_count = cpu.clamp(2, 4);

    for worker_id in 0..worker_count {
        let icon_ctx = ctx.clone();
        let icon_req_rx = fanout_rx.clone();
        let icon_res_tx = icon_res_tx.clone();
        let generation_ref = current_generation.clone();
        let ext_cache = shared_ext_cache.clone();
        let disk_cache = icon_disk_cache.clone();

        let _ = std::thread::Builder::new()
            .name(format!("icon-worker-{}", worker_id))
            .stack_size(256 * 1024)
            .spawn(move || {
                use crate::domain::file_entry::IconSize;
                use crate::infrastructure::windows::{
                    extract_file_icon_by_path, extract_shell_icon, get_file_type_icon,
                };
                use windows::Win32::System::Com::{
                    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
                };

                // STA (COINIT_APARTMENTTHREADED) is required for SHGetFileInfoW to
                // correctly resolve ProgID-based icons (e.g. dllfile, sysfile, batfile).
                // Using MTA causes generic icons for those types.
                // RAII guard ensures CoUninitialize on normal exit AND panic.
                struct ComGuard { initialized: bool }
                impl Drop for ComGuard {
                    fn drop(&mut self) {
                        if self.initialized {
                            unsafe { CoUninitialize(); }
                        }
                    }
                }
                let _com = unsafe {
                    let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                    ComGuard { initialized: hr.is_ok() }
                };

                crate::infrastructure::io_priority::set_thread_priority(
                    crate::infrastructure::io_priority::IOPriority::Interactive,
                );

                while let Ok((path, req_generation)) = icon_req_rx.recv() {

                    // Drop stale requests quickly when user has already navigated away.
                    // usize::MAX = pre-warm requests (always process).
                    if req_generation != usize::MAX
                        && req_generation != generation_ref.load(AtomicOrdering::Relaxed)
                    {
                        continue;
                    }

                    let process_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {

                    // Fast path: for files that don't have unique per-file icons,
                    // use extension-based extraction (SHGFI_USEFILEATTRIBUTES, ~0.5ms)
                    // instead of real-path extraction (SHGetFileInfoW on real file, ~80ms).
                    // This matches how Windows Explorer resolves icons.
                    let ext_lower = path.extension()
                        .map(|e| e.to_string_lossy().to_lowercase());
                    let per_file_icon = ext_lower.as_deref()
                        .map(crate::infrastructure::windows::icons::is_per_file_icon_ext)
                        .unwrap_or(true);
                    let needs_real_path_shared_icon = ext_lower.as_deref()
                        .map(crate::infrastructure::windows::icons::requires_real_file_for_shared_icon)
                        .unwrap_or(false);

                    let is_virtual_archive_path = crate::domain::file_entry::is_path_inside_archive(&path);

                    // For files, prefer Jumbo (256×256 via IShellItemImageFactory)
                    // so grid icons render at high resolution instead of the
                    // blurry upscaled 48×48 Large icons.
                    let icon_result = if is_virtual_archive_path {
                        extract_shell_icon(&path, IconSize::Jumbo)
                    } else if per_file_icon {
                        if let Some(cache_key) = disk_cache.file_icon_cache_key(&path, IconSize::Jumbo) {
                            if let Some(cached) = disk_cache.load_file_icon(&cache_key) {
                                Ok(cached)
                            } else {
                                let result = extract_file_icon_by_path(&path, IconSize::Jumbo);
                                if let Ok((pixels, width, height)) = &result {
                                    disk_cache.save_file_icon(&cache_key, pixels, *width, *height);
                                }
                                result
                            }
                        } else {
                            extract_file_icon_by_path(&path, IconSize::Jumbo)
                        }
                    } else {
                        let ext_raw = ext_lower.as_deref().unwrap_or("");
                        let ext_str = crate::infrastructure::windows::icons::canonical_icon_ext(ext_raw);
                        // Check shared Jumbo cache first.
                        let jumbo_dot_ext = if ext_str.is_empty() {
                            String::new()
                        } else {
                            format!(".{}_Jumbo", ext_str)
                        };
                        if let Some(cached) = ext_cache
                            .get(&jumbo_dot_ext)
                            .map(|entry| entry.value().clone())
                        {
                            Ok(cached)
                        } else {
                            // Try Jumbo extraction via IShellItemImageFactory on the real file.
                            // Falls back to SHGetFileInfoW (48×48) if IShellItemImageFactory fails.
                            let r = if needs_real_path_shared_icon
                                || path.exists()
                            {
                                extract_file_icon_by_path(&path, IconSize::Jumbo)
                            } else {
                                get_file_type_icon(false, ext_str, IconSize::Jumbo)
                            };
                            if let Ok(ref data) = r {
                                ext_cache.insert(jumbo_dot_ext, data.clone());
                            }
                            r
                        }
                    };

                    match icon_result {
                        Ok((pixels, width, height)) => {
                            let _ = icon_res_tx.send((path, req_generation, pixels, width, height));
                        }
                        Err(_) => {
                            let _ = icon_res_tx.send((path, req_generation, Vec::new(), 0, 0));
                        }
                    }
                    icon_ctx.request_repaint();

                    })); // end catch_unwind

                    if let Err(e) = process_result {
                        let msg = if let Some(s) = e.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = e.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "unknown".to_string()
                        };
                        log::error!("[IconWorker-{}] panic: {}", worker_id, msg);
                    }
                }

                // ComGuard RAII handles CoUninitialize on drop
            });
    }

    (icon_req_tx, icon_res_rx)
}

pub(in crate::app) fn spawn_metadata_worker(
    ctx: &egui::Context,
) -> (
    mpsc::Sender<MetadataRequest>,
    mpsc::Receiver<MetadataResponse>,
) {
    let (meta_req_tx, meta_req_rx) = mpsc::channel::<MetadataRequest>();
    let (latest_req_tx, latest_req_rx) = crossbeam_channel::bounded::<MetadataRequest>(1);
    let latest_req_rx_for_replace = latest_req_rx.clone();
    let (meta_res_tx, meta_res_rx) = mpsc::channel();
    let meta_ctx = ctx.clone();

    let _ = std::thread::Builder::new()
        .name("metadata-dispatcher".to_owned())
        .spawn(move || {
            while let Ok(mut latest) = meta_req_rx.recv() {
                while let Ok(next) = meta_req_rx.try_recv() {
                    latest = next;
                }

                loop {
                    match latest_req_tx.try_send(latest) {
                        Ok(()) => break,
                        Err(crossbeam_channel::TrySendError::Full(returned)) => {
                            let _ = latest_req_rx_for_replace.try_recv();
                            latest = returned;
                        }
                        Err(crossbeam_channel::TrySendError::Disconnected(_)) => return,
                    }
                }
            }
        });

    let _ = std::thread::Builder::new()
        .name("metadata-worker".to_owned())
        .spawn(move || {
            crate::infrastructure::io_priority::set_thread_priority(
                crate::infrastructure::io_priority::IOPriority::Background,
            );

            while let Ok((path, mtime)) = latest_req_rx.recv() {
                let meta = windows_infra::extract_media_metadata(&path);
                if meta_res_tx.send((path, mtime, meta)).is_err() {
                    break;
                }
                meta_ctx.request_repaint();
            }
        });

    (meta_req_tx, meta_res_rx)
}

pub(in crate::app) fn spawn_live_file_size_worker(
    ctx: &egui::Context,
) -> (
    mpsc::Sender<crate::app::live_file_size::LiveFileSizeRequest>,
    mpsc::Receiver<crate::app::live_file_size::LiveFileSizeResponse>,
) {
    let (size_req_tx, size_req_rx) =
        mpsc::channel::<crate::app::live_file_size::LiveFileSizeRequest>();
    let (size_res_tx, size_res_rx) =
        mpsc::channel::<crate::app::live_file_size::LiveFileSizeResponse>();
    let size_ctx = ctx.clone();

    std::thread::spawn(move || {
        crate::infrastructure::io_priority::set_thread_priority(
            crate::infrastructure::io_priority::IOPriority::Background,
        );

        while let Ok((path, mtime)) = size_req_rx.recv() {
            let live_size = if crate::app::live_file_size::should_probe_live_file_size(&path, mtime)
            {
                std::fs::metadata(&path)
                    .ok()
                    .filter(|meta| meta.is_file())
                    .map(|meta| meta.len())
            } else {
                None
            };

            let _ = size_res_tx.send((path, mtime, live_size));
            size_ctx.request_repaint();
        }
    });

    (size_req_tx, size_res_rx)
}

pub(in crate::app) fn spawn_file_hash_worker(
    ctx: &egui::Context,
) -> (
    mpsc::Sender<FileHashRequestIn>,
    mpsc::Receiver<FileHashResponseIn>,
) {
    let (hash_req_tx, hash_req_rx) = mpsc::channel::<FileHashRequestIn>();
    let (hash_res_tx, hash_res_rx) = mpsc::channel::<FileHashResponseIn>();
    let hash_ctx = ctx.clone();

    std::thread::spawn(move || {
        crate::infrastructure::io_priority::set_thread_priority(
            crate::infrastructure::io_priority::IOPriority::Background,
        );

        while let Ok((path, modified, size)) = hash_req_rx.recv() {
            let result = crate::app::file_hash::compute_sha256_streaming(&path);
            if hash_res_tx.send((path, modified, size, result)).is_err() {
                break;
            }
            hash_ctx.request_repaint();
        }
    });

    (hash_req_tx, hash_res_rx)
}
