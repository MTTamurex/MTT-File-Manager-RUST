use std::collections::HashSet;
use std::path::PathBuf;

pub(super) fn query(
    states: &[(PathBuf, Option<PathBuf>)],
    cloud: bool,
    threshold: u32,
) -> Result<Vec<PathBuf>, String> {
    query_with(
        states,
        cloud,
        threshold,
        crate::infrastructure::global_search::check_paths_modified,
    )
}

fn query_with(
    states: &[(PathBuf, Option<PathBuf>)],
    cloud: bool,
    threshold: u32,
    check: impl FnOnce(&[String], u32) -> Result<Vec<String>, String>,
) -> Result<Vec<PathBuf>, String> {
    if cloud || states.is_empty() {
        return Ok(Vec::new());
    }
    let candidates: Vec<String> = states
        .iter()
        .map(|(path, _)| path.to_string_lossy().into_owned())
        .collect();
    let modified = check(&candidates, threshold.max(5))?;
    let modified: HashSet<_> = modified.into_iter().collect();
    Ok(states
        .iter()
        .filter(|(path, _)| modified.contains(path.to_string_lossy().as_ref()))
        .map(|(path, _)| path.clone())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_only_requested_changes_and_preserves_continuous_notifications() {
        let states = vec![
            (PathBuf::from("folder/child"), None),
            (PathBuf::from("folder/other"), None),
        ];
        for _ in 0..3 {
            let changed = query_with(&states, false, 3, |paths, threshold| {
                assert_eq!(threshold, 5);
                assert_eq!(paths.len(), 2);
                Ok(vec![paths[0].clone(), "unrelated".into()])
            })
            .unwrap();
            assert_eq!(changed, vec![states[0].0.clone()]);
        }
    }

    #[test]
    fn unavailable_service_cloud_or_empty_candidates_do_not_trigger_disk_fallback() {
        let states = vec![(PathBuf::from("folder/child"), None)];
        assert!(query_with(&states, true, 5, |_, _| panic!("cloud query"))
            .unwrap()
            .is_empty());
        assert!(query_with(&[], false, 5, |_, _| panic!("empty query"))
            .unwrap()
            .is_empty());
        assert!(query_with(&states, false, 5, |_, _| Err("unavailable".into())).is_err());
    }
}
