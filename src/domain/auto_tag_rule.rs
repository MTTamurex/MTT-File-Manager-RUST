use rustc_hash::{FxHashMap, FxHashSet};
use std::path::{Path, PathBuf};

use super::file_tag::{normalize_tag_path_key, FileTag};
use super::tag_tree::tags_to_assign;

/// A folder-to-tag rule: items that appear inside `folder_path` automatically
/// receive `tag_id`. With `include_subfolders` the rule also covers anything
/// deeper; otherwise only direct children match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AutoTagRule {
    pub id: i64,
    pub folder_path: PathBuf,
    pub tag_id: i64,
    pub enabled: bool,
    pub include_subfolders: bool,
    pub position: i64,
}

/// Rules whose path scope covers `path`: the path may be the watched folder
/// itself, a direct child, or anything deeper (recursive rules only).
/// Case-insensitive, like every tag path key. Disabled rules never match;
/// Duplicate tags collapse, preferring recursive coverage when scopes overlap.
pub fn matching_rules<'a>(rules: &'a [AutoTagRule], path: &Path) -> Vec<&'a AutoTagRule> {
    let key = normalize_tag_path_key(path);
    let mut indices = FxHashMap::default();
    let mut matched: Vec<&AutoTagRule> = Vec::new();
    for rule in rules.iter().filter(|rule| rule.enabled) {
        if !rule_matches_path(&key, rule) {
            continue;
        }
        if let Some(&index) = indices.get(&rule.tag_id) {
            if rule.include_subfolders {
                matched[index] = rule;
            }
        } else {
            indices.insert(rule.tag_id, matched.len());
            matched.push(rule);
        }
    }
    matched
}

fn rule_matches_path(item_key: &str, rule: &AutoTagRule) -> bool {
    let folder_key = normalize_tag_path_key(&rule.folder_path);
    if item_key == folder_key {
        return true;
    }
    let folder_key = folder_key.trim_end_matches('\\');
    let Some(suffix) = item_key.strip_prefix(folder_key) else {
        return false;
    };
    if !suffix.starts_with('\\') {
        return false; // sibling name ("C:\fotos2" is not inside "C:\fotos")
    }
    if rule.include_subfolders {
        return true;
    }
    // Non-recursive: only direct children — the item's parent must be the
    // watched folder itself.
    suffix[1..].find('\\').is_none()
}

/// Tag IDs (with their implicit ancestor chains) to write when the given
/// rule tags reach a file that currently carries `existing`.
pub fn chain_for_rule_tag(
    defs: &rustc_hash::FxHashMap<i64, FileTag>,
    tag_id: i64,
    existing: &FxHashSet<i64>,
) -> Vec<i64> {
    tags_to_assign(defs, tag_id)
        .into_iter()
        .filter(|id| !existing.contains(id))
        .collect()
}

/// Folder rewrites needed when a watched folder (or one of its ancestors) is
/// renamed or moved: `(rule_id, new_folder_path)` for every affected rule.
pub fn remap_rule_folders(
    rules: &[AutoTagRule],
    old_root: &Path,
    new_root: &Path,
) -> Vec<(i64, PathBuf)> {
    let old_key = normalize_tag_path_key(old_root);
    let mut remaps = Vec::new();
    for rule in rules {
        let folder_key = normalize_tag_path_key(&rule.folder_path);
        if folder_key == old_key {
            remaps.push((rule.id, new_root.to_path_buf()));
        } else if let Some(suffix) = folder_key.strip_prefix(&old_key) {
            if suffix.starts_with('\\') {
                remaps.push((
                    rule.id,
                    PathBuf::from(format!("{}{}", normalize_tag_path_key(new_root), suffix)),
                ));
            }
        }
    }
    remaps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_tag::FileTag;
    use crate::domain::file_tag::TagColor;
    use rustc_hash::FxHashMap;

    fn rule(id: i64, folder: &str, tag_id: i64, enabled: bool) -> AutoTagRule {
        AutoTagRule {
            id,
            folder_path: PathBuf::from(folder),
            tag_id,
            enabled,
            include_subfolders: true,
            position: id,
        }
    }

    fn non_recursive_rule(id: i64, folder: &str, tag_id: i64) -> AutoTagRule {
        AutoTagRule {
            include_subfolders: false,
            ..rule(id, folder, tag_id, true)
        }
    }

    fn matched_tag_ids(rules: &[AutoTagRule], path: &Path) -> Vec<i64> {
        matching_rules(rules, path)
            .into_iter()
            .map(|rule| rule.tag_id)
            .collect()
    }

    #[test]
    fn matching_covers_folder_itself_children_and_depth() {
        let rules = vec![rule(1, r"C:\Media\Fotos", 7, true)];
        assert_eq!(
            matched_tag_ids(&rules, Path::new(r"C:\Media\Fotos")),
            vec![7]
        );
        assert_eq!(
            matched_tag_ids(&rules, Path::new(r"C:\Media\Fotos\trip.jpg")),
            vec![7]
        );
        assert_eq!(
            matched_tag_ids(&rules, Path::new(r"C:\Media\Fotos\2025\day\raw.png")),
            vec![7]
        );
        assert!(matched_tag_ids(&rules, Path::new(r"C:\Media\Músicas")).is_empty());
    }

    #[test]
    fn non_recursive_rules_match_only_direct_children() {
        let rules = vec![non_recursive_rule(1, r"C:\Media\Fotos", 7)];
        assert_eq!(
            matched_tag_ids(&rules, Path::new(r"C:\Media\Fotos\trip.jpg")),
            vec![7]
        );
        // A subfolder placed directly in the watched folder is a direct child
        // and matches the path scope; whether it gets tagged also depends on
        // the caller's file/folder check (subfolders are excluded when the
        // rule does not include them).
        assert_eq!(
            matched_tag_ids(&rules, Path::new(r"C:\Media\Fotos\2025")),
            vec![7]
        );
        assert!(matched_tag_ids(&rules, Path::new(r"C:\Media\Fotos\2025\raw.png")).is_empty());
    }

    #[test]
    fn sibling_prefixes_and_case_do_not_confuse_matching() {
        let rules = vec![rule(1, r"C:\Media\Fotos", 7, true)];
        assert!(matched_tag_ids(&rules, Path::new(r"C:\Media\Fotos2\x.jpg")).is_empty());
        assert_eq!(
            matched_tag_ids(&rules, Path::new(r"c:\MEDIA\FOTOS\a.jpg")),
            vec![7]
        );
    }

    #[test]
    fn disabled_rules_never_match_and_duplicates_collapse() {
        let rules = vec![
            rule(1, r"C:\Media", 7, false),
            rule(2, r"C:\Media", 7, true),
            rule(3, r"C:\Media", 8, true),
        ];
        assert_eq!(
            matched_tag_ids(&rules, Path::new(r"C:\Media\file.jpg")),
            vec![7, 8]
        );
    }

    #[test]
    fn unrelated_rule_does_not_hide_matching_rule_for_the_same_tag() {
        let rules = vec![
            rule(1, r"C:\Library\First", 7, true),
            rule(2, r"C:\Library\Second", 7, true),
        ];
        assert_eq!(
            matched_tag_ids(&rules, Path::new(r"C:\Library\Second\file.txt")),
            vec![7]
        );
    }

    #[test]
    fn recursive_scope_wins_when_the_same_tag_has_overlapping_rules() {
        let rules = [
            non_recursive_rule(1, r"C:\Library", 7),
            rule(2, r"C:\Library", 7, true),
        ];
        let matched = matching_rules(&rules, Path::new(r"C:\Library\Subfolder"));
        assert_eq!(matched.len(), 1);
        assert!(matched[0].include_subfolders);
    }

    #[test]
    fn drive_and_unc_roots_match_children_with_or_without_trailing_separator() {
        for root in [r"C:\", r"\\server\share\", r"\\server\share"] {
            let child = PathBuf::from(format!("{}\\file.txt", root.trim_end_matches('\\')));
            assert_eq!(matched_tag_ids(&[rule(1, root, 7, true)], &child), vec![7]);
            assert_eq!(
                matched_tag_ids(&[non_recursive_rule(1, root, 7)], &child),
                vec![7]
            );
            let nested =
                PathBuf::from(format!("{}\\folder\\file.txt", root.trim_end_matches('\\')));
            assert!(matched_tag_ids(&[non_recursive_rule(1, root, 7)], &nested).is_empty());
        }
    }

    #[test]
    fn chain_for_rule_tag_completes_ancestors_already_missing() {
        let defs: FxHashMap<i64, FileTag> = [
            (
                1,
                FileTag {
                    id: 1,
                    name: "Fotos".into(),
                    color: TagColor::Blue,
                    position: 0,
                    parent_id: None,
                },
            ),
            (
                2,
                FileTag {
                    id: 2,
                    name: "Férias".into(),
                    color: TagColor::Blue,
                    position: 1,
                    parent_id: Some(1),
                },
            ),
        ]
        .into_iter()
        .collect();

        let empty = FxHashSet::default();
        assert_eq!(chain_for_rule_tag(&defs, 2, &empty), vec![2, 1]);

        let with_parent: FxHashSet<i64> = [1].into_iter().collect();
        assert_eq!(chain_for_rule_tag(&defs, 2, &with_parent), vec![2]);
    }

    #[test]
    fn remap_follows_renamed_folder_and_its_subfolders() {
        let rules = vec![
            rule(1, r"C:\Media\Fotos", 7, true),
            rule(2, r"C:\Media\Fotos\2025", 8, true),
            rule(3, r"C:\Media\Músicas", 9, true),
        ];
        let remaps = remap_rule_folders(
            &rules,
            Path::new(r"C:\Media\Fotos"),
            Path::new(r"D:\Acervo"),
        );
        assert_eq!(
            remaps,
            vec![
                (1, PathBuf::from(r"D:\Acervo")),
                (
                    2,
                    PathBuf::from(format!(
                        "{}\\2025",
                        normalize_tag_path_key(Path::new(r"D:\Acervo"))
                    ))
                ),
            ]
        );
    }
}
