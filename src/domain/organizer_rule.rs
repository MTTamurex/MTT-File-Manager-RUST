use std::path::{Path, PathBuf};

/// A persisted rule that moves matching files from one folder to another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganizerRule {
    pub id: i64,
    pub source_folder: PathBuf,
    pub destination_folder: PathBuf,
    pub extensions: Vec<String>,
    pub enabled: bool,
}

impl OrganizerRule {
    pub fn new(
        id: i64,
        source_folder: PathBuf,
        destination_folder: PathBuf,
        extensions: Vec<String>,
        enabled: bool,
    ) -> Result<Self, String> {
        let extensions = normalize_extensions(&extensions)?;
        validate_folders(&source_folder, &destination_folder)?;
        Ok(Self {
            id,
            source_folder,
            destination_folder,
            extensions,
            enabled,
        })
    }

    /// Restores a persisted rule even if a removable or network folder is
    /// temporarily unavailable. Validation runs again when the user edits it.
    pub fn from_persisted(
        id: i64,
        source_folder: PathBuf,
        destination_folder: PathBuf,
        extensions: Vec<String>,
        enabled: bool,
    ) -> Result<Self, String> {
        Ok(Self {
            id,
            source_folder,
            destination_folder,
            extensions: normalize_extensions(&extensions)?,
            enabled,
        })
    }

    pub fn matches(&self, path: &Path) -> bool {
        path.is_file()
            && path
                .parent()
                .is_some_and(|parent| paths_equal(parent, &self.source_folder))
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    self.extensions
                        .iter()
                        .any(|allowed| allowed.eq_ignore_ascii_case(extension))
                })
    }

    pub fn extensions_csv(&self) -> String {
        self.extensions.join(",")
    }
}

pub fn parse_extensions(input: &str) -> Result<Vec<String>, String> {
    let extensions: Vec<String> = input
        .split(|character: char| character == ',' || character.is_whitespace())
        .filter(|extension| !extension.trim().is_empty())
        .map(|extension| extension.trim().trim_start_matches('.').to_string())
        .collect();
    normalize_extensions(&extensions)
}

pub fn preview_rule(rule: &OrganizerRule) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(&rule.source_folder) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| rule.matches(path))
        .collect()
}

fn normalize_extensions(extensions: &[String]) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for extension in extensions {
        let extension = extension.trim().trim_start_matches('.');
        if extension.is_empty()
            || extension.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|'])
        {
            return Err("Extensões inválidas".to_string());
        }
        if !normalized
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(extension))
        {
            normalized.push(extension.to_ascii_lowercase());
        }
    }
    if normalized.is_empty() {
        return Err("Informe ao menos uma extensão".to_string());
    }
    Ok(normalized)
}

fn validate_folders(source: &Path, destination: &Path) -> Result<(), String> {
    if !source.is_absolute() || !destination.is_absolute() {
        return Err("As pastas devem usar caminhos absolutos".to_string());
    }
    if !source.is_dir() {
        return Err("A pasta de origem não existe".to_string());
    }
    if !destination.is_dir() {
        return Err("A pasta de destino não existe".to_string());
    }
    if paths_equal(destination, source) {
        return Err("A pasta de destino deve ser diferente da origem".to_string());
    }
    Ok(())
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    normalize_path(left) == normalize_path(right)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_extensions_without_duplicates_or_dots() {
        assert_eq!(
            parse_extensions(".JPG, png jpg").expect("valid extensions"),
            vec!["jpg", "png"]
        );
    }

    #[test]
    fn preview_only_returns_matching_files_at_source_root() {
        let source = tempfile::tempdir().expect("source tempdir");
        let destination = tempfile::tempdir().expect("destination tempdir");
        std::fs::write(source.path().join("photo.JPG"), b"x").expect("write matching file");
        std::fs::write(source.path().join("note.txt"), b"x").expect("write nonmatching file");
        std::fs::create_dir(source.path().join("nested")).expect("create nested directory");
        let rule = OrganizerRule::new(
            1,
            source.path().to_path_buf(),
            destination.path().to_path_buf(),
            vec!["jpg".to_string()],
            true,
        )
        .expect("valid rule");
        assert_eq!(preview_rule(&rule), vec![source.path().join("photo.JPG")]);
    }

    #[test]
    fn allows_destination_inside_non_recursive_source() {
        let source = tempfile::tempdir().expect("source tempdir");
        let destination = source.path().join("Images");
        std::fs::create_dir(&destination).expect("destination directory");
        assert!(OrganizerRule::new(
            0,
            source.path().to_path_buf(),
            destination,
            vec!["jpg".to_string()],
            true
        )
        .is_ok());
    }

    #[test]
    fn rejects_identical_source_and_destination() {
        let source = tempfile::tempdir().expect("source tempdir");
        assert!(OrganizerRule::new(
            0,
            source.path().to_path_buf(),
            source.path().to_path_buf(),
            vec!["jpg".to_string()],
            true
        )
        .is_err());
    }
}
