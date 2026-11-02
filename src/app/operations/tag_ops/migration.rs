use crate::domain::file_tag::FileTag;
use crate::infrastructure::app_state_db::AppStateDb;
use rustc_hash::FxHashMap;

/// Preference key recording that the implicit-parent backfill has run.
pub(crate) const ANCESTOR_BACKFILL_KEY: &str = "tag_ancestor_backfill_version";
/// Bump when the backfill rules change, so a newer build can re-run it.
pub(crate) const ANCESTOR_BACKFILL_VERSION: &str = "1";

/// Applies the implicit-parent invariant to pre-existing assignments once.
///
/// Additive only: it inserts missing ancestor links and never deletes an
/// assignment. Gated by a preference so it runs on the first startup after the
/// upgrade and not again. Returns the number of links inserted, or `None` when
/// it was already applied or the write failed (in which case it will retry on
/// the next startup, since the flag is only set after a successful write).
pub(crate) fn run_ancestor_backfill_once(
    db: &AppStateDb,
    defs: &FxHashMap<i64, FileTag>,
) -> Option<usize> {
    if db.get_preference(ANCESTOR_BACKFILL_KEY).as_deref() == Some(ANCESTOR_BACKFILL_VERSION) {
        return None;
    }

    let assignments = db.get_all_tag_assignments();
    let links = crate::domain::tag_tree::missing_ancestor_assignments(defs, &assignments);
    if !db.insert_tag_links(&links) {
        return None;
    }
    if db
        .set_preference(ANCESTOR_BACKFILL_KEY, ANCESTOR_BACKFILL_VERSION)
        .is_err()
    {
        log::warn!("[TAGS] Ancestor backfill applied but its marker could not be saved");
    }
    log::info!(
        "[TAGS] Ancestor backfill inserted {} missing tag links",
        links.len()
    );
    Some(links.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::file_tag::TagColor;
    use std::path::PathBuf;

    #[test]
    fn backfill_runs_once_and_restores_missing_ancestors() {
        let db = AppStateDb::new_in_memory().unwrap();
        let root = db.create_tag("Fotos", TagColor::Blue, None).unwrap();
        let child = db
            .create_tag("Férias", TagColor::Green, Some(root))
            .unwrap();
        let leaf = db.create_tag("2025", TagColor::Red, Some(child)).unwrap();
        let path = PathBuf::from(r"C:\Library\trip.jpg");
        // Legacy state: only the leaf was stored.
        assert!(db.assign_tags_batch(std::slice::from_ref(&path), &[leaf]));

        let defs = db
            .get_all_tags()
            .into_iter()
            .map(|tag| (tag.id, tag))
            .collect::<FxHashMap<_, _>>();

        assert_eq!(run_ancestor_backfill_once(&db, &defs), Some(2));
        let assignments = db.get_all_tag_assignments();
        let tags = assignments.get(&path).cloned().unwrap_or_default();
        assert!(tags.contains(&root) && tags.contains(&child) && tags.contains(&leaf));

        // Second startup: flag set, nothing runs.
        assert_eq!(run_ancestor_backfill_once(&db, &defs), None);
    }
}
