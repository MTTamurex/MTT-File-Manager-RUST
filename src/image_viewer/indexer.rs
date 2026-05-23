use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ImageSequence {
    pub entries: Vec<PathBuf>,
    pub current_index: usize,
}

impl ImageSequence {
    pub fn single(path: PathBuf) -> Self {
        Self {
            entries: vec![path],
            current_index: 0,
        }
    }
}

pub fn build_sequence(open_path: &Path) -> io::Result<ImageSequence> {
    // Archive virtual paths and explicit shell namespace paths do not support simple
    // filesystem enumeration reliably. Fallback to single-file mode.
    let path_lower = open_path.to_string_lossy().to_ascii_lowercase();
    if crate::domain::file_entry::path_contains_archive_segment(&path_lower)
        || crate::infrastructure::security::classify_shell_namespace_path(open_path).is_some()
    {
        return Ok(ImageSequence::single(open_path.to_path_buf()));
    }

    if open_path.is_file() {
        let parent = open_path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "file has no parent directory")
        })?;
        let entries = collect_image_entries(parent)?;

        if entries.is_empty() {
            return Ok(ImageSequence::single(open_path.to_path_buf()));
        }

        let current_index = entries
            .iter()
            .position(|p| paths_eq_case_insensitive(p, open_path))
            .unwrap_or(0);

        return Ok(ImageSequence {
            entries,
            current_index,
        });
    }

    if open_path.is_dir() {
        let entries = collect_image_entries(open_path)?;
        if entries.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "directory does not contain supported images",
            ));
        }

        return Ok(ImageSequence {
            entries,
            current_index: 0,
        });
    }

    Ok(ImageSequence::single(open_path.to_path_buf()))
}

fn collect_image_entries(dir: &Path) -> io::Result<Vec<PathBuf>> {
    const MAX_IMAGE_ENTRIES: usize = 10_000;

    let mut paths = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = match entry {
            Ok(v) => v,
            Err(_) => continue,
        };

        let path = entry.path();
        if !is_supported_image(&path) {
            continue;
        }

        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }

        paths.push(path);
        if paths.len() >= MAX_IMAGE_ENTRIES {
            break;
        }
    }

    paths.sort_by(|a, b| {
        let a_name = a.file_name().and_then(|v| v.to_str()).unwrap_or_default();
        let b_name = b.file_name().and_then(|v| v.to_str()).unwrap_or_default();
        natord::compare_ignore_case(a_name, b_name)
    });

    Ok(paths)
}

fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(crate::infrastructure::windows::is_image_extension)
        .unwrap_or(false)
}

fn paths_eq_case_insensitive(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn touch(path: &Path) {
        fs::write(path, b"test").expect("write test file");
    }

    fn file_names(sequence: &ImageSequence) -> Vec<String> {
        sequence
            .entries
            .iter()
            .map(|path| {
                path.file_name()
                    .expect("file name")
                    .to_string_lossy()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn image_sequence_ignores_non_images_and_image_named_directories() {
        let dir = tempdir().expect("create temp dir");
        touch(&dir.path().join("b.png"));
        touch(&dir.path().join("a.jpg"));
        touch(&dir.path().join("notes.txt"));
        fs::create_dir(dir.path().join("folder.jpg")).expect("create image-named dir");

        let entries = collect_image_entries(dir.path()).expect("collect entries");
        let names: Vec<String> = entries
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert_eq!(names, vec!["a.jpg", "b.png"]);
    }

    #[test]
    fn image_sequence_preserves_natural_sort_and_current_index() {
        let dir = tempdir().expect("create temp dir");
        let image_10 = dir.path().join("image10.jpg");

        touch(&image_10);
        touch(&dir.path().join("image2.jpg"));
        touch(&dir.path().join("image1.jpg"));

        let sequence = build_sequence(&image_10).expect("build image sequence");

        assert_eq!(file_names(&sequence), vec!["image1.jpg", "image2.jpg", "image10.jpg"]);
        assert_eq!(sequence.current_index, 2);
    }
}
