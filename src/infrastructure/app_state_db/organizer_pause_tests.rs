use super::*;
use crate::domain::organizer_rule::OrganizerRule;

fn seed_rules(db: &AppStateDb, root: &std::path::Path) -> Vec<i64> {
    [("txt", true), ("pdf", true), ("zip", false)]
        .into_iter()
        .map(|(extension, enabled)| {
            let rule = OrganizerRule::from_persisted(
                0,
                root.join("source"),
                root.join("destination"),
                vec![extension.into()],
                enabled,
            )
            .unwrap();
            db.save_organizer_rule(&rule).unwrap()
        })
        .collect()
}

#[test]
fn global_switch_persists_and_legacy_pauses_become_disabled_rules_on_reopen() {
    let root = tempfile::tempdir().unwrap();
    let db_path = root.path().join("state");
    {
        let db = AppStateDb::new(db_path.clone()).unwrap();
        assert!(!db.organizer_globally_paused().unwrap());
        let ids = seed_rules(&db, root.path());
        db.set_preference(PAUSED_RULES_KEY, &format!("[{},999]", ids[1]))
            .unwrap();
        db.set_organizer_globally_paused(true).unwrap();
    }
    {
        let db = AppStateDb::new(db_path.clone()).unwrap();
        let rules = db.get_organizer_rules();
        assert_eq!(
            rules.iter().map(|r| r.enabled).collect::<Vec<_>>(),
            vec![true, false, false]
        );
        assert!(db.get_preference(PAUSED_RULES_KEY).is_none());
        assert!(db.organizer_globally_paused().unwrap());
        db.set_organizer_globally_paused(false).unwrap();
        assert!(!db.get_organizer_rules()[1].enabled);
        let mut rule = rules[1].clone();
        rule.enabled = true;
        db.save_organizer_rule(&rule).unwrap();
    }
    let db = AppStateDb::new(db_path).unwrap();
    assert!(!db.organizer_globally_paused().unwrap());
    assert!(db.get_organizer_rules()[1].enabled);
    assert!(!db.get_organizer_rules()[2].enabled);
}

#[test]
fn legacy_pause_migration_is_atomic_when_consuming_the_preference_fails() {
    let root = tempfile::tempdir().unwrap();
    let db = AppStateDb::new_in_memory().unwrap();
    let ids = seed_rules(&db, root.path());
    let legacy = format!("[{},{}]", ids[0], ids[1]);
    db.set_preference(PAUSED_RULES_KEY, &legacy).unwrap();
    {
        let conn = db.test_hold_writer();
        conn.execute_batch(
            "CREATE TRIGGER reject_legacy_delete BEFORE DELETE ON user_preferences
             BEGIN SELECT RAISE(FAIL, 'test failure'); END;",
        )
        .unwrap();
        assert!(migrate_individual_pauses(&conn).is_err());
    }
    assert!(db.get_organizer_rules()[0].enabled);
    assert!(db.get_organizer_rules()[1].enabled);
    assert_eq!(db.get_preference(PAUSED_RULES_KEY), Some(legacy));
}

#[test]
fn invalid_preferences_are_reported_without_discarding_legacy_data() {
    let db = AppStateDb::new_in_memory().unwrap();
    db.set_preference(GLOBAL_PAUSE_KEY, "invalid").unwrap();
    assert!(db.organizer_globally_paused().is_err());
    db.set_preference(PAUSED_RULES_KEY, "invalid").unwrap();
    assert!(migrate_individual_pauses(&db.test_hold_writer()).is_err());
    assert_eq!(
        db.get_preference(PAUSED_RULES_KEY).as_deref(),
        Some("invalid")
    );
}
