use std::os::windows::ffi::OsStringExt;
use std::path::{Component, Path, PathBuf, Prefix};

use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath, KF_FLAG_DONT_VERIFY};

use crate::domain::file_entry::IconSize;

pub fn extract_handler_icon(
    icon_location: Option<&(String, i32)>,
    executable: Option<&Path>,
) -> Option<(Vec<u8>, u32, u32)> {
    if let Some((location, index)) = icon_location {
        let packaged_location = location.starts_with("@{");
        let image_path = resolve_packaged_icon_path(location).or_else(|| {
            (is_raster_image(location) && is_safe_local_path(Path::new(location)))
                .then(|| PathBuf::from(location))
        });
        if let Some(path) = image_path {
            if let Some(icon) = resolve_raster_variant(&path)
                .as_deref()
                .and_then(decode_raster_icon)
            {
                return Some(icon);
            }
        }

        if !packaged_location && is_safe_local_path(Path::new(location)) {
            let resource = format!("{},{}", location, index);
            if let Ok(icon) = super::icons::extract_icon_resource(&resource, IconSize::Small) {
                return Some(icon);
            }
        }
    }

    executable
        .filter(|path| is_safe_local_path(path))
        .and_then(|path| super::icons::extract_file_icon_by_path(path, IconSize::Small).ok())
}

fn resolve_packaged_icon_path(location: &str) -> Option<PathBuf> {
    let indirect = location.strip_prefix("@{")?.strip_suffix('}')?;
    let (package_full_name, resource_uri) = indirect.split_once("?ms-resource://")?;
    let mut package_components = Path::new(package_full_name).components();
    if !matches!(package_components.next(), Some(Component::Normal(_)))
        || package_components.next().is_some()
    {
        return None;
    }

    let (_, relative) = resource_uri.split_once("/Files/")?;
    let relative = Path::new(relative);
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }

    let program_files = program_files_path()?;
    Some(
        program_files
            .join("WindowsApps")
            .join(package_full_name)
            .join(relative),
    )
}

fn program_files_path() -> Option<PathBuf> {
    let path =
        unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KF_FLAG_DONT_VERIFY, None).ok()? };
    let result = unsafe {
        let mut len = 0;
        while *path.0.add(len) != 0 {
            len += 1;
        }
        PathBuf::from(std::ffi::OsString::from_wide(std::slice::from_raw_parts(
            path.0, len,
        )))
    };
    unsafe { CoTaskMemFree(Some(path.0.cast())) };
    Some(result)
}

fn is_safe_local_path(path: &Path) -> bool {
    let mut components = path.components();
    let local_prefix = matches!(
        components.next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    );
    local_prefix
        && matches!(components.next(), Some(Component::RootDir))
        && components.all(|component| matches!(component, Component::Normal(_)))
}

fn is_raster_image(location: &str) -> bool {
    Path::new(location)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp"
            )
        })
}

fn decode_raster_icon(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > 8 * 1024 * 1024 {
        return None;
    }
    let mut reader = image::ImageReader::open(path).ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let image = image.resize(32, 32, image::imageops::FilterType::Lanczos3);
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some((rgba.into_raw(), width, height))
}

fn resolve_raster_variant(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    let base_name = path.file_stem()?.to_str()?.to_ascii_lowercase();
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let qualified = std::fs::read_dir(parent).ok().and_then(|entries| {
        entries
            .take(256)
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let file_name = entry.file_name();
                let file_name = file_name.to_str()?.to_ascii_lowercase();
                let score = raster_variant_score(&file_name, &base_name, &extension)?;
                Some((score, entry.path()))
            })
            .min_by_key(|(score, _)| *score)
            .map(|(_, path)| path)
    });
    qualified.or_else(|| path.is_file().then(|| path.to_path_buf()))
}

fn raster_variant_score(file_name: &str, base_name: &str, extension: &str) -> Option<u32> {
    if !file_name.starts_with(&format!("{}.", base_name))
        || !file_name.ends_with(&format!(".{}", extension))
    {
        return None;
    }

    let size_rank = parse_qualifier(file_name, ".targetsize-").map_or_else(
        || {
            parse_qualifier(file_name, ".scale-")
                .map(|scale| 1_000 + scale.abs_diff(100))
                .unwrap_or(2_000)
        },
        |size| {
            if size >= 32 {
                size - 32
            } else {
                10_000 + (32 - size)
            }
        },
    );
    let style_penalty = if file_name.contains("_contrast-") {
        2_000
    } else if file_name.contains("_altform-") {
        1_000
    } else {
        0
    };
    Some(size_rank + style_penalty)
}

fn parse_qualifier(file_name: &str, marker: &str) -> Option<u32> {
    let value = file_name.split_once(marker)?.1;
    let digits = value
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_packaged_ms_resource_icon_path() {
        let location = concat!(
            "@{Microsoft.Windows.Photos_1.2.3.0_x64__8wekyb3d8bbwe?",
            "ms-resource://Microsoft.Windows.Photos/Files/Assets/PhotosAppList.png}"
        );
        let path = resolve_packaged_icon_path(location).expect("packaged icon path");

        assert!(path.ends_with(
            Path::new("Microsoft.Windows.Photos_1.2.3.0_x64__8wekyb3d8bbwe")
                .join("Assets")
                .join("PhotosAppList.png")
        ));
    }

    #[test]
    fn rejects_traversal_in_packaged_icon_reference() {
        let location = "@{Package_1.0_x64__publisher?ms-resource://Package/Files/../secret.png}";
        assert!(resolve_packaged_icon_path(location).is_none());
        let prefixed = "@{C:?ms-resource://Package/Files/secret.png}";
        assert!(resolve_packaged_icon_path(prefixed).is_none());
    }

    #[test]
    fn accepts_only_absolute_local_disk_paths() {
        assert!(is_safe_local_path(Path::new(
            r"C:\Program Files\App\app.exe"
        )));
        assert!(!is_safe_local_path(Path::new(r"\\server\share\app.exe")));
        assert!(!is_safe_local_path(Path::new(
            r"\\?\UNC\server\share\app.exe"
        )));
        assert!(!is_safe_local_path(Path::new(r"relative\app.exe")));
        assert!(!is_safe_local_path(Path::new(r"C:\safe\..\app.exe")));
    }

    #[test]
    fn prefers_plain_targetsize_32_packaged_icon() {
        let base = "photosapplist";
        let extension = "png";
        let plain = raster_variant_score("photosapplist.targetsize-32.png", base, extension);
        let small = raster_variant_score("photosapplist.targetsize-16.png", base, extension);
        let styled = raster_variant_score(
            "photosapplist.targetsize-32_altform-unplated.png",
            base,
            extension,
        );

        assert!(plain < small);
        assert!(styled < small);
        assert_eq!(
            raster_variant_score("other.targetsize-32.png", base, extension),
            None
        );
    }
}
