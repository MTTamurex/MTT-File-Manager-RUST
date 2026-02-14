use crate::infrastructure::disk_cache::ThumbnailDiskCache;
use crate::infrastructure::windows as windows_infra;
use eframe::egui;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};

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
            let cover = windows_infra::find_folder_preview_item(&folder_path);

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
            fonts
                .families
                .get_mut(&eframe::egui::FontFamily::Proportional)
                .unwrap()
                .extend(loaded_fonts.clone());

            fonts
                .families
                .get_mut(&eframe::egui::FontFamily::Monospace)
                .unwrap()
                .extend(loaded_fonts.clone());
        }

        let _ = font_tx.send(fonts);
    });
    font_rx
}

pub(in crate::app) fn spawn_icon_worker(
    ctx: &egui::Context,
) -> (
    mpsc::Sender<PathBuf>,
    mpsc::Receiver<(PathBuf, Vec<u8>, u32, u32)>,
) {
    let (icon_req_tx, icon_req_rx) = mpsc::channel::<PathBuf>();
    let (icon_res_tx, icon_res_rx) = mpsc::channel::<(PathBuf, Vec<u8>, u32, u32)>();
    let icon_ctx = ctx.clone();

    std::thread::spawn(move || {
        use crate::domain::file_entry::IconSize;
        use crate::infrastructure::windows::extract_file_icon_by_path;
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        crate::infrastructure::io_priority::set_thread_priority(
            crate::infrastructure::io_priority::IOPriority::Background,
        );

        while let Ok(path) = icon_req_rx.recv() {
            match extract_file_icon_by_path(&path, IconSize::Jumbo) {
                Ok((pixels, width, height)) => {
                    let _ = icon_res_tx.send((path, pixels, width, height));
                }
                Err(_) => {
                    let _ = icon_res_tx.send((path, Vec::new(), 0, 0));
                }
            }
            icon_ctx.request_repaint();
        }

        unsafe {
            CoUninitialize();
        }
    });

    (icon_req_tx, icon_res_rx)
}

pub(in crate::app) fn spawn_metadata_worker(
    ctx: &egui::Context,
) -> (
    mpsc::Sender<(PathBuf, u64)>,
    mpsc::Receiver<(PathBuf, u64, windows_infra::MediaMetadata)>,
) {
    let (meta_req_tx, meta_req_rx) = mpsc::channel::<(PathBuf, u64)>();
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
