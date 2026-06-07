use exif::{Reader as ExifReader, Tag};
use image::ImageReader;
use std::path::Path;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

use super::property_keys::*;
use super::utils::*;
use super::MediaMetadata;

#[derive(Default)]
struct ExifSummary {
    camera_maker: Option<String>,
    camera_model: Option<String>,
    f_stop: Option<String>,
    exposure_time: Option<String>,
    iso_speed: Option<u32>,
    focal_length: Option<String>,
    max_aperture: Option<String>,
    metering_mode: Option<String>,
    flash_mode: Option<String>,
    date_taken: Option<String>,
    subject: Option<String>,
}

impl ExifSummary {
    fn into_media_metadata(self) -> MediaMetadata {
        MediaMetadata {
            camera_maker: self.camera_maker,
            camera_model: self.camera_model,
            f_stop: self.f_stop,
            exposure_time: self.exposure_time,
            iso_speed: self.iso_speed,
            focal_length: self.focal_length,
            max_aperture: self.max_aperture,
            metering_mode: self.metering_mode,
            flash_mode: self.flash_mode,
            date_taken: self.date_taken,
            subject: self.subject,
            ..Default::default()
        }
    }
}

fn read_exif_summary(exifreader: &exif::Exif) -> ExifSummary {
    let mut summary = ExifSummary::default();

    for field in exifreader.fields() {
        match field.tag {
            Tag::Make if summary.camera_maker.is_none() => {
                summary.camera_maker = Some(field.display_value().to_string());
            }
            Tag::Model if summary.camera_model.is_none() => {
                summary.camera_model = Some(field.display_value().to_string());
            }
            Tag::FNumber if summary.f_stop.is_none() => {
                summary.f_stop = Some(format!("f/{}", field.display_value()));
            }
            Tag::ExposureTime if summary.exposure_time.is_none() => {
                summary.exposure_time = Some(format!("{} sec.", field.display_value()));
            }
            Tag::PhotographicSensitivity if summary.iso_speed.is_none() => {
                if let exif::Value::Short(ref v) = field.value {
                    if let Some(value) = v.first() {
                        summary.iso_speed = Some(*value as u32);
                    }
                }
            }
            Tag::FocalLength if summary.focal_length.is_none() => {
                summary.focal_length = Some(format!("{} mm", field.display_value()));
            }
            Tag::MaxApertureValue if summary.max_aperture.is_none() => {
                summary.max_aperture = Some(field.display_value().to_string());
            }
            Tag::MeteringMode if summary.metering_mode.is_none() => {
                summary.metering_mode = Some(field.display_value().to_string());
            }
            Tag::Flash if summary.flash_mode.is_none() => {
                summary.flash_mode = Some(field.display_value().to_string());
            }
            Tag::DateTimeOriginal if summary.date_taken.is_none() => {
                summary.date_taken = Some(field.display_value().to_string());
            }
            Tag::DateTimeDigitized if summary.date_taken.is_none() => {
                summary.date_taken = Some(field.display_value().to_string());
            }
            Tag::DateTime if summary.date_taken.is_none() => {
                summary.date_taken = Some(field.display_value().to_string());
            }
            Tag::ImageDescription if summary.subject.is_none() => {
                summary.subject = Some(field.display_value().to_string());
            }
            _ => {}
        }
    }

    summary
}

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
        format: format_label,
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
        ..Default::default()
    })
}

pub fn read_image_exif_metadata(path: &Path) -> Result<MediaMetadata, windows::core::Error> {
    log::trace!("[EXIF DEBUG] Reading EXIF for: {:?}", path.file_name());

    let mut exif_summary = ExifSummary::default();

    if let Ok(file) = std::fs::File::open(path) {
        let mut bufreader = std::io::BufReader::new(&file);
        if let Ok(exifreader) = ExifReader::new().read_from_container(&mut bufreader) {
            log::trace!("  [EXIF] Successfully parsed EXIF data");
            exif_summary = read_exif_summary(&exifreader);
        } else {
            log::debug!("  [EXIF] Failed to parse EXIF data, trying Property Store fallback");

            let _com_guard = ComGuard::new();
            if let Ok(store) = unsafe { open_property_store(path) } {
                exif_summary.camera_maker = unsafe { read_string(&store, &PKEY_IMAGE_CAMERAMAKER) };
                exif_summary.camera_model = unsafe { read_string(&store, &PKEY_IMAGE_CAMERAMODEL) };
                exif_summary.f_stop = unsafe { read_f_number(&store, &PKEY_IMAGE_FNUMBER) };
                exif_summary.exposure_time =
                    unsafe { read_exposure_time(&store, &PKEY_IMAGE_EXPOSURETIME) };
                exif_summary.iso_speed = unsafe { read_u32(&store, &PKEY_IMAGE_ISOSPEED) };
                exif_summary.focal_length =
                    unsafe { read_focal_length(&store, &PKEY_IMAGE_FOCALLENGTH) };
                exif_summary.max_aperture =
                    unsafe { read_aperture(&store, &PKEY_IMAGE_MAXAPERTURE) };
                exif_summary.metering_mode =
                    unsafe { read_metering_mode(&store, &PKEY_IMAGE_METERINGMODE) };
                exif_summary.flash_mode = unsafe { read_flash_mode(&store, &PKEY_IMAGE_FLASH) };
                exif_summary.date_taken = unsafe { read_string(&store, &PKEY_IMAGE_DATETAKEN) };
                exif_summary.subject = unsafe { read_string(&store, &PKEY_IMAGE_SUBJECT) };
            }
        }
    }

    Ok(exif_summary.into_media_metadata())
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
