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
    InvalidConflictFolder,
    RuleCycle,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum OrganizerConflictPolicy {
    #[default]
    Ask,
    Skip,
    AutoRenameSource,
    MoveToConflictFolder(PathBuf),
}

impl OrganizerConflictPolicy {
    pub fn storage_key(&self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Skip => "skip",
            Self::AutoRenameSource => "auto_rename_source",
            Self::MoveToConflictFolder(_) => "move_to_conflict_folder",
        }
    }

    pub fn conflict_folder(&self) -> Option<&Path> {
        match self {
            Self::MoveToConflictFolder(folder) => Some(folder),
            _ => None,
        }
    }

    pub fn from_persisted(policy: &str, conflict_folder: Option<PathBuf>) -> Self {
        match policy {
            "skip" => Self::Skip,
            "auto_rename_source" => Self::AutoRenameSource,
            "move_to_conflict_folder" => conflict_folder
                .filter(|folder| is_valid_conflict_folder_path(folder))
                .map(Self::MoveToConflictFolder)
                .unwrap_or(Self::Ask),
            _ => Self::Ask,
        }
    }
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
    pub conflict_policy: OrganizerConflictPolicy,
}

impl OrganizerRule {
    pub fn new(
        id: i64,
        source_folder: PathBuf,
        destination_folder: PathBuf,
        extensions: Vec<String>,
        enabled: bool,
    ) -> Result<Self, OrganizerRuleError> {
        Self::new_with_conflict_policy(
            id,
            source_folder,
            destination_folder,
            extensions,
            enabled,
            OrganizerConflictPolicy::default(),
        )
    }

    pub fn new_with_conflict_policy(
        id: i64,
        source_folder: PathBuf,
        destination_folder: PathBuf,
        extensions: Vec<String>,
        enabled: bool,
        conflict_policy: OrganizerConflictPolicy,
    ) -> Result<Self, OrganizerRuleError> {
        let extensions = normalize_extensions(&extensions)?;
        validate_folders(&source_folder, &destination_folder)?;
        validate_conflict_policy(&conflict_policy, &source_folder, &destination_folder)?;
        validate_conflict_folder_exists(&conflict_policy)?;
        Ok(Self {
            id,
            source_folder,
            destination_folder,
            extensions,
            enabled,
            conflict_policy,
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
        Self::from_persisted_with_policy(
            id,
            source_folder,
            destination_folder,
            extensions,
            enabled,
            OrganizerConflictPolicy::default(),
        )
    }

    pub fn from_persisted_with_policy(
        id: i64,
        source_folder: PathBuf,
        destination_folder: PathBuf,
        extensions: Vec<String>,
        enabled: bool,
        conflict_policy: OrganizerConflictPolicy,
    ) -> Result<Self, OrganizerRuleError> {
        validate_conflict_policy(&conflict_policy, &source_folder, &destination_folder)?;
        Ok(Self {
            id,
            source_folder,
            destination_folder,
            extensions: normalize_extensions(&extensions)?,
            enabled,
            conflict_policy,
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
        let mut destinations = vec![folder_identity(&rule.destination_folder)];
        if let OrganizerConflictPolicy::MoveToConflictFolder(folder) = &rule.conflict_policy {
            destinations.push(folder_identity(folder));
        }
        for extension in &rule.extensions {
            let source_extension = format!("{source}\0{extension}");
            if !matched_sources.insert(source_extension.clone()) {
                continue;
            }
            graph.entry(source_extension).or_default().extend(
                destinations
                    .iter()
                    .map(|destination| format!("{destination}\0{extension}")),
            );
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

fn validate_conflict_policy(
    policy: &OrganizerConflictPolicy,
    source: &Path,
    destination: &Path,
) -> Result<(), OrganizerRuleError> {
    let Some(folder) = policy.conflict_folder() else {
        return Ok(());
    };
    if !is_valid_conflict_folder_path(folder)
        || paths_equal(folder, source)
        || paths_equal(folder, destination)
    {
        return Err(OrganizerRuleError::InvalidConflictFolder);
    }
    Ok(())
}

fn validate_conflict_folder_exists(
    policy: &OrganizerConflictPolicy,
) -> Result<(), OrganizerRuleError> {
    if policy
        .conflict_folder()
        .is_some_and(|folder| !folder.is_dir())
    {
        return Err(OrganizerRuleError::InvalidConflictFolder);
    }
    Ok(())
}

fn is_valid_conflict_folder_path(path: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    if path.components().any(|component| {
        matches!(
            component,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )
    }) {
        return false;
    }
    let text = path.to_string_lossy();
    !text.starts_with(r"\\?\") && !text.starts_with(r"\\.\") && !contains_reparse_point(path)
}

fn contains_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    let mut current = Some(path);
    while let Some(candidate) = current {
        if std::fs::symlink_metadata(candidate)
            .is_ok_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        {
            return true;
        }
        current = candidate.parent();
    }
    false
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
    fn conflict_policy_defaults_to_ask_and_round_trips_storage_values() {
        assert_eq!(
            OrganizerConflictPolicy::default(),
            OrganizerConflictPolicy::Ask
        );
        assert_eq!(OrganizerConflictPolicy::Ask.storage_key(), "ask");
        assert_eq!(OrganizerConflictPolicy::Skip.storage_key(), "skip");
        assert_eq!(
            OrganizerConflictPolicy::AutoRenameSource.storage_key(),
            "auto_rename_source"
        );

        let folder = PathBuf::from(r"C:\Conflicts");
        assert_eq!(
            OrganizerConflictPolicy::from_persisted(
                "move_to_conflict_folder",
                Some(folder.clone())
            ),
            OrganizerConflictPolicy::MoveToConflictFolder(folder)
        );
        assert_eq!(
            OrganizerConflictPolicy::from_persisted("unknown", None),
            OrganizerConflictPolicy::Ask
        );
    }

    #[test]
    fn rejects_conflict_folder_that_is_relative_or_matches_a_rule_folder() {
        let source = tempfile::tempdir().expect("source tempdir");
        let destination = tempfile::tempdir().expect("destination tempdir");
        let missing_folder = source.path().join("missing-conflicts");

        assert_eq!(
            OrganizerRule::new_with_conflict_policy(
                1,
                source.path().to_path_buf(),
                destination.path().to_path_buf(),
                vec!["txt".to_string()],
                true,
                OrganizerConflictPolicy::MoveToConflictFolder(PathBuf::from("relative")),
            )
            .expect_err("relative conflict folder must be rejected"),
            OrganizerRuleError::InvalidConflictFolder
        );
        assert_eq!(
            OrganizerRule::new_with_conflict_policy(
                1,
                source.path().to_path_buf(),
                destination.path().to_path_buf(),
                vec!["txt".to_string()],
                true,
                OrganizerConflictPolicy::MoveToConflictFolder(source.path().to_path_buf()),
            )
            .expect_err("source cannot also be the conflict folder"),
            OrganizerRuleError::InvalidConflictFolder
        );
        assert_eq!(
            OrganizerRule::new_with_conflict_policy(
                1,
                source.path().to_path_buf(),
                destination.path().to_path_buf(),
                vec!["txt".to_string()],
                true,
                OrganizerConflictPolicy::MoveToConflictFolder(missing_folder.clone()),
            )
            .expect_err("missing conflict folder must be rejected for new rules"),
            OrganizerRuleError::InvalidConflictFolder
        );
        assert!(OrganizerRule::from_persisted_with_policy(
            1,
            source.path().to_path_buf(),
            destination.path().to_path_buf(),
            vec!["txt".to_string()],
            true,
            OrganizerConflictPolicy::MoveToConflictFolder(missing_folder),
        )
        .is_ok());
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
    fn rejects_cycles_through_a_conflict_folder() {
        let folder_a = tempfile::tempdir().expect("folder a");
        let normal_destination = tempfile::tempdir().expect("normal destination");
        let conflict_folder = tempfile::tempdir().expect("conflict folder");
        let rules = vec![
            OrganizerRule::new_with_conflict_policy(
                1,
                folder_a.path().to_path_buf(),
                normal_destination.path().to_path_buf(),
                vec!["pdf".to_string()],
                true,
                OrganizerConflictPolicy::MoveToConflictFolder(conflict_folder.path().to_path_buf()),
            )
            .expect("rule with conflict folder"),
            OrganizerRule::new(
                2,
                conflict_folder.path().to_path_buf(),
                folder_a.path().to_path_buf(),
                vec!["pdf".to_string()],
                true,
            )
            .expect("return rule"),
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
