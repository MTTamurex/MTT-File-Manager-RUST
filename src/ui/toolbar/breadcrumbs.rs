use crate::domain::cloud_root::CloudRoot;
use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};

#[derive(Default)]
struct BreadcrumbCache {
    path: String,
    locale: String,
    root: Option<CloudRoot>,
    segments: Vec<(String, String)>,
}

thread_local! {
    static CACHE: RefCell<BreadcrumbCache> = RefCell::new(BreadcrumbCache::default());
}

fn normalized(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}

fn contains(root: &str, path: &str) -> bool {
    let root = normalized(root);
    let path = normalized(path);
    path == root
        || path
            .strip_prefix(&root)
            .is_some_and(|tail| tail.starts_with('\\'))
}

pub(super) fn breadcrumb_segments(
    current_path: &str,
    cloud_roots: &[CloudRoot],
) -> Vec<(String, String)> {
    let root = cloud_roots
        .iter()
        .filter(|root| contains(&root.path, current_path))
        .max_by_key(|root| normalized(&root.path).len());
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let locale = rust_i18n::locale().to_string();
        if cache.path != current_path || cache.locale != locale || cache.root.as_ref() != root {
            cache.segments = build_segments(current_path, root, |path| {
                crate::infrastructure::onedrive::special_folder_display_name(path)
            });
            cache.path = current_path.to_string();
            cache.locale = locale;
            cache.root = root.cloned();
        }
        cache.segments.clone()
    })
}

fn build_segments(
    current_path: &str,
    cloud_root: Option<&CloudRoot>,
    special_name: impl Fn(&Path) -> Option<String>,
) -> Vec<(String, String)> {
    if let Some(server) = crate::domain::network_path::unc_server_name(current_path) {
        return vec![(server.to_string(), current_path.to_string())];
    }
    let mut full = PathBuf::new();
    let mut segments = Vec::new();
    let mut inside_cloud = false;
    for component in Path::new(current_path).components() {
        full.push(component.as_os_str());
        if matches!(component, Component::RootDir) {
            continue;
        }
        let mut target = full.to_string_lossy().into_owned();
        if target.len() == 2 && target.ends_with(':') {
            target.push('\\');
        }
        if let Some(root) = cloud_root {
            if normalized(&target) == normalized(&root.path) {
                segments.clear();
                let provider = root.breadcrumb_provider.as_deref().unwrap_or(&root.label);
                segments.push((provider.to_string(), target.clone()));
                if let Some(account) = root
                    .breadcrumb_account
                    .as_deref()
                    .filter(|s| *s != provider)
                {
                    segments.push((account.to_string(), target));
                }
                inside_cloud = true;
                continue;
            }
        }
        let special = special_name(&full);
        // A recognized cloud root takes precedence over redirected known folders.
        if cloud_root.is_none() && special.is_some() {
            segments.clear();
        }
        let display = special.unwrap_or_else(|| {
            component
                .as_os_str()
                .to_string_lossy()
                .trim_end_matches('\\')
                .to_string()
        });
        if !display.is_empty() {
            segments.push((display, target));
        }
    }
    // A root must match a complete component, never an arbitrary string prefix.
    debug_assert!(cloud_root.is_none() || inside_cloud);
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cloud() -> CloudRoot {
        let mut root = CloudRoot::windows_cloud_files(r"C:\Cloud".into(), "Cloud".into(), None);
        root.breadcrumb_provider = Some("Provider".into());
        root.breadcrumb_account = Some("Account".into());
        root
    }

    #[test]
    fn cloud_preserves_provider_account_and_real_navigation_targets() {
        let segments = build_segments(r"c:\cloud\Documents\Project", Some(&cloud()), |path| {
            (path == Path::new(r"c:\cloud\Documents")).then(|| "Documents".into())
        });
        assert_eq!(
            segments,
            vec![
                ("Provider".into(), r"c:\cloud".into()),
                ("Account".into(), r"c:\cloud".into()),
                ("Documents".into(), r"c:\cloud\Documents".into()),
                ("Project".into(), r"c:\cloud\Documents\Project".into()),
            ]
        );
    }

    #[test]
    fn known_folder_becomes_root_only_outside_clouds() {
        let segments = build_segments(r"C:\Users\user\Documents\Project", None, |path| {
            (path == Path::new(r"C:\Users\user\Documents")).then(|| "Documents".into())
        });
        assert_eq!(
            segments,
            vec![
                ("Documents".into(), r"C:\Users\user\Documents".into()),
                ("Project".into(), r"C:\Users\user\Documents\Project".into()),
            ]
        );
    }

    #[test]
    fn roots_require_component_boundaries_and_allow_case_and_separator_variations() {
        assert!(contains(r"C:\Cloud\", "c:/cloud/Folder"));
        assert!(!contains(r"C:\Cloud", r"C:\CloudOther\Folder"));
    }

    #[test]
    fn cache_refreshes_when_cloud_metadata_arrives_or_changes() {
        let path = r"C:\Cloud\Folder";
        assert_eq!(breadcrumb_segments(path, &[])[0].0, "C:");
        let mut root = cloud();
        assert_eq!(breadcrumb_segments(path, &[root.clone()])[0].0, "Provider");
        root.breadcrumb_account = Some("Other account".into());
        assert_eq!(breadcrumb_segments(path, &[root])[1].0, "Other account");
        assert_eq!(breadcrumb_segments(path, &[])[0].0, "C:");
    }

    #[test]
    fn missing_account_metadata_does_not_invent_an_account_label() {
        let mut root = cloud();
        root.breadcrumb_account = None;
        assert_eq!(
            breadcrumb_segments(r"C:\Cloud", &[root]),
            vec![("Provider".into(), r"C:\Cloud".into())]
        );
    }

    #[test]
    fn google_drive_labels_never_replace_the_editable_filesystem_path() {
        let mut root = CloudRoot::google_drive_shortcut(
            r"D:\Folder".into(),
            "Google Drive - My Drive".into(),
            None,
            r"G:\".into(),
        );
        root.breadcrumb_provider = Some("Google Drive".into());
        root.breadcrumb_account = Some("My Drive".into());
        let path = r"D:\Folder\Project";
        assert_eq!(
            breadcrumb_segments(path, &[root]),
            vec![
                ("Google Drive".into(), r"D:\Folder".into()),
                ("My Drive".into(), r"D:\Folder".into()),
                ("Project".into(), path.into()),
            ]
        );
        assert_eq!(crate::ui::address_bar::editable_path(path, None), path);
    }

    #[test]
    fn ordinary_paths_and_unc_servers_keep_their_targets() {
        assert_eq!(
            breadcrumb_segments(r"C:\Folder", &[]),
            vec![
                ("C:".into(), r"C:\".into()),
                ("Folder".into(), r"C:\Folder".into()),
            ]
        );
        for path in [r"\\server", r"\\server\"] {
            assert_eq!(
                breadcrumb_segments(path, &[]),
                vec![("server".into(), path.into())]
            );
        }
    }
}
