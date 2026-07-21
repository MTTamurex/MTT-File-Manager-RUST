use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrganizerExtensionPreset {
    Documents,
    Images,
    Videos,
    Audio,
    Archives,
    Executables,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrganizerRuleError {
    InvalidExtensions,
    MissingExtensions,
    RelativeFolder,
    SourceFolderMissing,
    DestinationFolderMissing,
    SameFolders,
    RuleCycle,
}

impl OrganizerExtensionPreset {
    pub const ALL: [Self; 6] = [
        Self::Documents,
        Self::Images,
        Self::Videos,
        Self::Audio,
        Self::Archives,
        Self::Executables,
    ];

    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Documents => &[
                "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "txt", "md", "rtf", "odt",
                "csv",
            ],
            Self::Images => &[
                "jpg", "jpeg", "png", "gif", "bmp", "webp", "tiff", "tif", "svg", "heic", "avif",
                "ico",
            ],
            Self::Videos => &[
                "mp4", "mkv", "avi", "mov", "wmv", "webm", "flv", "m4v", "mpg", "mpeg", "3gp",
                "ogv", "ogm", "ts", "m2ts",
            ],
            Self::Audio => &["mp3", "wav", "flac", "aac", "ogg", "wma", "m4a", "opus"],
            Self::Archives => &["zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst"],
            Self::Executables => &["exe", "msi", "msix", "appx", "com", "scr"],
        }
    }
}

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
    ) -> Result<Self, OrganizerRuleError> {
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
    ) -> Result<Self, OrganizerRuleError> {
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

pub fn parse_extensions(input: &str) -> Result<Vec<String>, OrganizerRuleError> {
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

pub fn validate_rule_set(rules: &[OrganizerRule]) -> Result<(), OrganizerRuleError> {
    use std::collections::{HashMap, HashSet};

    let mut graph: HashMap<String, Vec<String>> = HashMap::new();
    let mut matched_sources = HashSet::new();
    for rule in rules.iter().filter(|rule| rule.enabled) {
        let source = folder_identity(&rule.source_folder);
        let destination = folder_identity(&rule.destination_folder);
        for extension in &rule.extensions {
            let source_extension = format!("{source}\0{extension}");
            if !matched_sources.insert(source_extension.clone()) {
                continue;
            }
            graph
                .entry(source_extension)
                .or_default()
                .push(format!("{destination}\0{extension}"));
        }
    }

    fn visits_cycle(
        node: &str,
        graph: &HashMap<String, Vec<String>>,
        visiting: &mut HashSet<String>,
        visited: &mut HashSet<String>,
    ) -> bool {
        if visited.contains(node) {
            return false;
        }
        if !visiting.insert(node.to_string()) {
            return true;
        }
        if graph.get(node).is_some_and(|destinations| {
            destinations
                .iter()
                .any(|destination| visits_cycle(destination, graph, visiting, visited))
        }) {
            return true;
        }
        visiting.remove(node);
        visited.insert(node.to_string());
        false
    }

    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    if graph
        .keys()
        .any(|source| visits_cycle(source, &graph, &mut visiting, &mut visited))
    {
        return Err(OrganizerRuleError::RuleCycle);
    }
    Ok(())
}

fn folder_identity(path: &Path) -> String {
    normalize_path(&path.canonicalize().unwrap_or_else(|_| path.to_path_buf()))
}

fn normalize_extensions(extensions: &[String]) -> Result<Vec<String>, OrganizerRuleError> {
    let mut normalized = Vec::new();
    for extension in extensions {
        let extension = extension.trim().trim_start_matches('.');
        if extension.is_empty()
            || extension.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|'])
        {
            return Err(OrganizerRuleError::InvalidExtensions);
        }
        if !normalized
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(extension))
        {
            normalized.push(extension.to_ascii_lowercase());
        }
    }
    if normalized.is_empty() {
        return Err(OrganizerRuleError::MissingExtensions);
    }
    Ok(normalized)
}

fn validate_folders(source: &Path, destination: &Path) -> Result<(), OrganizerRuleError> {
    if !source.is_absolute() || !destination.is_absolute() {
        return Err(OrganizerRuleError::RelativeFolder);
    }
    if !source.is_dir() {
        return Err(OrganizerRuleError::SourceFolderMissing);
    }
    if !destination.is_dir() {
        return Err(OrganizerRuleError::DestinationFolderMissing);
    }
    if paths_equal(destination, source) {
        return Err(OrganizerRuleError::SameFolders);
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
    fn executable_preset_contains_common_windows_executables() {
        assert_eq!(
            OrganizerExtensionPreset::Executables.extensions(),
            ["exe", "msi", "msix", "appx", "com", "scr"]
        );
    }

    #[test]
    fn every_preset_contains_valid_extensions() {
        for preset in OrganizerExtensionPreset::ALL {
            let input = preset.extensions().join(", ");
            assert!(
                parse_extensions(&input).is_ok(),
                "invalid preset: {preset:?}"
            );
        }
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

    #[test]
    fn rejects_enabled_rule_cycles() {
        let folder_a = tempfile::tempdir().expect("folder a");
        let folder_b = tempfile::tempdir().expect("folder b");
        let rules = vec![
            OrganizerRule::new(
                1,
                folder_a.path().to_path_buf(),
                folder_b.path().to_path_buf(),
                vec!["pdf".to_string()],
                true,
            )
            .expect("rule a to b"),
            OrganizerRule::new(
                2,
                folder_b.path().to_path_buf(),
                folder_a.path().to_path_buf(),
                vec!["pdf".to_string()],
                true,
            )
            .expect("rule b to a"),
        ];

        assert_eq!(
            validate_rule_set(&rules),
            Err(OrganizerRuleError::RuleCycle)
        );
    }

    #[test]
    fn allows_acyclic_rule_chains() {
        let folder_a = tempfile::tempdir().expect("folder a");
        let folder_b = tempfile::tempdir().expect("folder b");
        let folder_c = tempfile::tempdir().expect("folder c");
        let rules = vec![
            OrganizerRule::new(
                1,
                folder_a.path().to_path_buf(),
                folder_b.path().to_path_buf(),
                vec!["pdf".to_string()],
                true,
            )
            .expect("rule a to b"),
            OrganizerRule::new(
                2,
                folder_b.path().to_path_buf(),
                folder_c.path().to_path_buf(),
                vec!["pdf".to_string()],
                true,
            )
            .expect("rule b to c"),
        ];

        assert_eq!(validate_rule_set(&rules), Ok(()));
    }

    #[test]
    fn allows_folder_cycles_when_extensions_do_not_overlap() {
        let folder_a = tempfile::tempdir().expect("folder a");
        let folder_b = tempfile::tempdir().expect("folder b");
        let rules = vec![
            OrganizerRule::new(
                1,
                folder_a.path().to_path_buf(),
                folder_b.path().to_path_buf(),
                vec!["pdf".to_string()],
                true,
            )
            .expect("pdf rule"),
            OrganizerRule::new(
                2,
                folder_b.path().to_path_buf(),
                folder_a.path().to_path_buf(),
                vec!["jpg".to_string()],
                true,
            )
            .expect("jpg rule"),
        ];

        assert_eq!(validate_rule_set(&rules), Ok(()));
    }

    #[test]
    fn ignores_shadowed_rules_when_detecting_cycles() {
        let folder_a = tempfile::tempdir().expect("folder a");
        let folder_b = tempfile::tempdir().expect("folder b");
        let folder_c = tempfile::tempdir().expect("folder c");
        let rules = vec![
            OrganizerRule::new(
                1,
                folder_a.path().to_path_buf(),
                folder_c.path().to_path_buf(),
                vec!["pdf".to_string()],
                true,
            )
            .expect("first matching rule"),
            OrganizerRule::new(
                2,
                folder_a.path().to_path_buf(),
                folder_b.path().to_path_buf(),
                vec!["pdf".to_string()],
                true,
            )
            .expect("shadowed rule"),
            OrganizerRule::new(
                3,
                folder_b.path().to_path_buf(),
                folder_a.path().to_path_buf(),
                vec!["pdf".to_string()],
                true,
            )
            .expect("return rule"),
        ];

        assert_eq!(validate_rule_set(&rules), Ok(()));
    }
}
