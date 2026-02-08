use exif::{In, Reader as ExifReader, Tag};
use image::ImageReader;
use std::path::Path;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

use super::property_keys::*;
use super::utils::*;
use super::MediaMetadata;

pub fn is_image_extension(ext: &str) -> bool {
    // Use Windows Perceived Type API for dynamic detection
    crate::infrastructure::windows::file_type::is_image_extension(ext)
}

pub fn read_image_metadata(path: &Path) -> Result<MediaMetadata, image::ImageError> {
    // Uses image crate headers only; does not decode the full image.
    let reader = ImageReader::open(path)?;
    let reader = reader.with_guessed_format()?;
    let format_label = reader.format().map(|f| format!("{:?}", f).to_uppercase());
    let (width, height) = reader.into_dimensions()?;

    // Try to also read EXIF data from property store if available
    let exif_metadata = read_image_exif_metadata(path).unwrap_or_default();

    Ok(MediaMetadata {
        width: Some(width),
        height: Some(height),
        duration_100ns: None,
        frame_rate: None,
        bitrate: None,
        format: format_label,
        color_depth: None,
        camera_maker: exif_metadata.camera_maker,
        camera_model: exif_metadata.camera_model,
        f_stop: exif_metadata.f_stop,
        exposure_time: exif_metadata.exposure_time,
        iso_speed: exif_metadata.iso_speed,
        focal_length: exif_metadata.focal_length,
        max_aperture: exif_metadata.max_aperture,
        metering_mode: exif_metadata.metering_mode,
        flash_mode: exif_metadata.flash_mode,
        date_taken: exif_metadata.date_taken,
        subject: exif_metadata.subject,
        video_codec: None,
        audio_codec: None,
        audio_bitrate: None,
        audio_channels: None,
    })
}

pub fn read_image_exif_metadata(path: &Path) -> Result<MediaMetadata, windows::core::Error> {
    eprintln!("[EXIF DEBUG] Reading EXIF for: {:?}", path.file_name());

    let mut camera_maker = None;
    let mut camera_model = None;
    let mut f_stop = None;
    let mut exposure_time = None;
    let mut iso_speed = None;
    let mut focal_length = None;
    let mut max_aperture = None;
    let mut metering_mode = None;
    let mut flash_mode = None;
    let mut date_taken = None;
    let mut subject = None;

    if let Ok(file) = std::fs::File::open(path) {
        let mut bufreader = std::io::BufReader::new(&file);
        if let Ok(exifreader) = ExifReader::new().read_from_container(&mut bufreader) {
            eprintln!("  [EXIF] Successfully parsed EXIF data");

            if let Some(field) = exifreader.get_field(Tag::Make, In::PRIMARY) {
                camera_maker = Some(field.display_value().to_string());
            }

            if let Some(field) = exifreader.get_field(Tag::Model, In::PRIMARY) {
                camera_model = Some(field.display_value().to_string());
            }

            if let Some(field) = exifreader.get_field(Tag::FNumber, In::PRIMARY) {
                f_stop = Some(format!("f/{}", field.display_value()));
            }

            if let Some(field) = exifreader.get_field(Tag::ExposureTime, In::PRIMARY) {
                exposure_time = Some(format!("{} sec.", field.display_value()));
            }

            if let Some(field) = exifreader.get_field(Tag::PhotographicSensitivity, In::PRIMARY) {
                if let exif::Value::Short(ref v) = field.value {
                    if !v.is_empty() {
                        iso_speed = Some(v[0] as u32);
                    }
                }
            }

            if let Some(field) = exifreader.get_field(Tag::FocalLength, In::PRIMARY) {
                focal_length = Some(format!("{} mm", field.display_value()));
            }

            if let Some(field) = exifreader.get_field(Tag::MaxApertureValue, In::PRIMARY) {
                max_aperture = Some(field.display_value().to_string());
            }

            if let Some(field) = exifreader.get_field(Tag::MeteringMode, In::PRIMARY) {
                metering_mode = Some(field.display_value().to_string());
            }

            if let Some(field) = exifreader.get_field(Tag::Flash, In::PRIMARY) {
                flash_mode = Some(field.display_value().to_string());
            }

            if let Some(field) = exifreader.get_field(Tag::DateTime, In::PRIMARY) {
                date_taken = Some(field.display_value().to_string());
            }

            if let Some(field) = exifreader.get_field(Tag::ImageDescription, In::PRIMARY) {
                subject = Some(field.display_value().to_string());
            }
        } else {
            eprintln!("  [EXIF] Failed to parse EXIF data, trying Property Store fallback");

            let _com_guard = ComGuard::new();
            if let Ok(store) = unsafe { open_property_store(path) } {
                camera_maker = unsafe { read_string(&store, &PKEY_IMAGE_CAMERAMAKER) };
                camera_model = unsafe { read_string(&store, &PKEY_IMAGE_CAMERAMODEL) };
                f_stop = unsafe { read_f_number(&store, &PKEY_IMAGE_FNUMBER) };
                exposure_time = unsafe { read_exposure_time(&store, &PKEY_IMAGE_EXPOSURETIME) };
                iso_speed = unsafe { read_u32(&store, &PKEY_IMAGE_ISOSPEED) };
                focal_length = unsafe { read_focal_length(&store, &PKEY_IMAGE_FOCALLENGTH) };
                max_aperture = unsafe { read_aperture(&store, &PKEY_IMAGE_MAXAPERTURE) };
                metering_mode = unsafe { read_metering_mode(&store, &PKEY_IMAGE_METERINGMODE) };
                flash_mode = unsafe { read_flash_mode(&store, &PKEY_IMAGE_FLASH) };
                date_taken = unsafe { read_string(&store, &PKEY_IMAGE_DATETAKEN) };
                subject = unsafe { read_string(&store, &PKEY_IMAGE_SUBJECT) };
            }
        }
    }

    Ok(MediaMetadata {
        camera_maker,
        camera_model,
        f_stop,
        exposure_time,
        iso_speed,
        focal_length,
        max_aperture,
        metering_mode,
        flash_mode,
        date_taken,
        subject,
        ..Default::default()
    })
}

// EXIF helper: Convert raw F-number value to f-stop string
unsafe fn read_f_number(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let raw = read_f64(store, key)?;
    Some(format!("f/{:.1}", raw))
}

// EXIF helper: Convert exposure time to 1/x format
unsafe fn read_exposure_time(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let raw = read_f64(store, key)?;
    if raw == 0.0 {
        return None;
    }
    if raw < 1.0 {
        Some(format!("1/{} sec.", (1.0 / raw).round() as u32))
    } else {
        Some(format!("{:.2} sec.", raw))
    }
}

// EXIF helper: Focal length in mm
unsafe fn read_focal_length(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let raw = read_f64(store, key)?;
    Some(format!("{:.0} mm", raw))
}

// EXIF helper: Max aperture F-number
unsafe fn read_aperture(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let raw = read_f64(store, key)?;
    Some(format!("{:.1}", raw))
}

// EXIF helper: Metering mode friendly name
unsafe fn read_metering_mode(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let raw = read_u32(store, key)?;
    let mode_name = match raw {
        0 => "Unknown",
        1 => "Average",
        2 => "Center Weighted",
        3 => "Spot",
        4 => "Multi-spot",
        5 => "Pattern",
        6 => "Partial",
        _ => "Other",
    };
    Some(mode_name.to_string())
}

// EXIF helper: Flash mode friendly name
unsafe fn read_flash_mode(store: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let raw = read_u32(store, key)?;
    let flash_name = match raw {
        0 => "No flash, compulsory",
        1 => "Flash fired",
        5 => "Flash fired, return light not detected",
        7 => "Flash fired, return light detected",
        8 => "No flash, return light detected",
        16 => "No flash, compulsory",
        24 => "No flash, auto",
        32 => "Flash fired, auto",
        _ => "Unknown flash mode",
    };
    Some(flash_name.to_string())
}
