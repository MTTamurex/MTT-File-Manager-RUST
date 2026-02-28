use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::icon_disk_cache::IconDiskCache;
use crate::infrastructure::windows as windows_infra;
use crate::infrastructure::windows::is_mpeg_ts_file;
use eframe::egui;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{mpsc, Arc, Mutex};

type IconRequest = (PathBuf, usize);
type IconResponse = (PathBuf, usize, Vec<u8>, u32, u32);
type MetadataRequest = (PathBuf, u64);
type MetadataResponse = (PathBuf, u64, windows_infra::MediaMetadata);

pub(in crate::app) fn spawn_cover_worker(
    disk_cache: Arc<ThumbnailDiskCache>,
) -> (
    mpsc::Sender<PathBuf>,
    mpsc::Receiver<(PathBuf, Option<PathBuf>)>,
) {
    let (cover_req_tx, cover_req_rx) = mpsc::channel::<PathBuf>();
    let (cover_res_tx, cover_res_rx) = mpsc::channel();

    let cover_worker_cache = disk_cache.clone();
    std::thread::spawn(move || {
        crate::infrastructure::io_priority::set_thread_priority(
            crate::infrastructure::io_priority::IOPriority::Background,
        );

        while let Ok(folder_path) = cover_req_rx.recv() {
            let cover = windows_infra::find_folder_preview_item(&folder_path)
                .filter(|p| {
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
                cover_worker_cache.set_folder_cover(&folder_path, c);
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

        let arial_path = fonts_dir.join("ARIALUNI.TTF");
        if let Ok(font_data) = std::fs::read(&arial_path) {
            fonts.font_data.insert(
                "arial_unicode".to_owned(),
                std::sync::Arc::new(eframe::egui::FontData::from_owned(font_data)),
            );
            loaded_fonts.push("arial_unicode".to_owned());
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

            if let Some(monospace) = fonts
                .families
                .get_mut(&eframe::egui::FontFamily::Monospace)
            {
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
    preloaded_icons: &std::collections::HashMap<String, (Vec<u8>, u32, u32)>,
) -> (mpsc::Sender<IconRequest>, mpsc::Receiver<IconResponse>) {
    let (icon_req_tx, icon_req_rx_thread) = mpsc::channel::<IconRequest>();
    let (fanout_tx, fanout_rx) = crossbeam_channel::unbounded::<IconRequest>();
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

    // Shared extension icon cache across all workers.
    // Pre-populated with disk-cached data so workers never call SHGetFileInfoW
    // for already-known extensions.
    let shared_ext_cache: Arc<Mutex<std::collections::HashMap<String, (Vec<u8>, u32, u32)>>> = {
        let mut initial = std::collections::HashMap::with_capacity(128);
        for (ext, data) in preloaded_icons {
            let dot_ext = format!(".{}", ext);
            initial.insert(dot_ext, data.clone());
        }
        Arc::new(Mutex::new(initial))
    };

    let cpu = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    // SHGetFileInfoW is IO-bound (registry + COM), not CPU-bound.
    // More threads = more parallel cold-extension lookups.
    let worker_count = cpu.clamp(2, 16);

    for worker_id in 0..worker_count {
        let icon_ctx = ctx.clone();
        let icon_req_rx = fanout_rx.clone();
        let icon_res_tx = icon_res_tx.clone();
        let generation_ref = current_generation.clone();
        let ext_cache = shared_ext_cache.clone();
        let disk_cache = icon_disk_cache.clone();

        let _ = std::thread::Builder::new()
            .name(format!("icon-worker-{}", worker_id))
            .spawn(move || {
                use crate::domain::file_entry::IconSize;
                use crate::infrastructure::windows::{extract_file_icon_by_path, get_file_type_icon};
                use windows::Win32::System::Com::{
                    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
                };

                // Extensions that may have unique per-file icons (embedded resources).
                const UNIQUE_ICON_EXTS: &[&str] = &["exe", "lnk", "ico", "cur", "ani", "com", "scr", "url"];

                // STA (COINIT_APARTMENTTHREADED) is required for SHGetFileInfoW to
                // correctly resolve ProgID-based icons (e.g. dllfile, sysfile, batfile).
                // Using MTA causes generic icons for those types.
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                }

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

                    // Fast path: for files that don't have unique per-file icons,
                    // use extension-based extraction (SHGFI_USEFILEATTRIBUTES, ~0.5ms)
                    // instead of real-path extraction (SHGetFileInfoW on real file, ~80ms).
                    // This matches how Windows Explorer resolves icons.
                    let ext_lower = path.extension()
                        .map(|e| e.to_string_lossy().to_lowercase());
                    let needs_real_path = ext_lower.as_deref()
                        .map(|e| UNIQUE_ICON_EXTS.contains(&e))
                        .unwrap_or(false);

                    let icon_result = if needs_real_path {
                        extract_file_icon_by_path(&path, IconSize::Large)
                    } else {
                        let ext_raw = ext_lower.as_deref().unwrap_or("");
                        // Map extensions that share the same shell icon (sys→dll etc.)
                        // so all variants share a single cache entry.
                        let ext_str = crate::infrastructure::windows::icons::canonical_icon_ext(ext_raw);
                        let dot_ext = if ext_str.is_empty() {
                            String::new()
                        } else {
                            format!(".{}", ext_str)
                        };
                        // Check shared cache first — another worker may have
                        // already extracted this extension's icon.
                        if let Some(cached) = ext_cache
                            .lock()
                            .ok()
                            .and_then(|c| c.get(&dot_ext).cloned())
                        {
                            Ok(cached)
                        } else {
                            // Use get_file_type_icon (with internal CoInitialize)
                            // instead of extract_file_icon — the latter produces
                            // generic icons for ProgID-based types (dll, sys, bat)
                            // on worker threads.
                            let r = get_file_type_icon(false, ext_str, IconSize::Large);
                            if let Ok(ref data) = r {
                                if let Ok(mut cache) = ext_cache.lock() {
                                    cache.insert(dot_ext, data.clone());
                                }
                                // Persist to disk for instant loading on next app launch.
                                disk_cache.save(ext_str, &data.0, data.1, data.2);
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
                }

                unsafe {
                    CoUninitialize();
                }
            });
    }

    (icon_req_tx, icon_res_rx)
}

pub(in crate::app) fn spawn_metadata_worker(
    ctx: &egui::Context,
) -> (mpsc::Sender<MetadataRequest>, mpsc::Receiver<MetadataResponse>) {
    let (meta_req_tx, meta_req_rx) = mpsc::channel::<MetadataRequest>();
    let (meta_res_tx, meta_res_rx) = mpsc::channel();
    let meta_ctx = ctx.clone();

    std::thread::spawn(move || {
        crate::infrastructure::io_priority::set_thread_priority(
            crate::infrastructure::io_priority::IOPriority::Background,
        );

        while let Ok((path, mtime)) = meta_req_rx.recv() {
            let meta = windows_infra::extract_media_metadata(&path);
            let _ = meta_res_tx.send((path, mtime, meta));
            meta_ctx.request_repaint();
        }
    });

    (meta_req_tx, meta_res_rx)
}
