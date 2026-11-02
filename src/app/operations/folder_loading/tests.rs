use super::preserve_empty_listing_on_reload;

#[test]
fn completed_empty_listing_is_preserved_during_reload() {
    assert!(preserve_empty_listing_on_reload(false, false, false));
}

#[test]
fn repeated_reload_keeps_the_preserved_empty_listing() {
    assert!(preserve_empty_listing_on_reload(true, true, false));
}

#[test]
fn restarting_initial_load_does_not_claim_a_complete_listing() {
    assert!(!preserve_empty_listing_on_reload(true, false, false));
}

#[test]
fn retrying_failed_load_does_not_preserve_an_empty_listing() {
    assert!(!preserve_empty_listing_on_reload(false, false, true));
    assert!(!preserve_empty_listing_on_reload(true, true, true));
}
