use super::*;
use std::collections::HashMap;

const CLASS_ID: &str = "{11111111-2222-3333-4444-555555555555}";
const SERVER_KEY: &str = r"CLSID\{11111111-2222-3333-4444-555555555555}\InprocServer32";
const SERVER: &str = r"C:\Program Files\Example\handler.dll";

fn registered(values: &[(&str, &str)]) -> bool {
    let values: HashMap<_, _> = values.iter().copied().collect();
    registered_with(|key| values.get(key).map(|value| value.to_string()))
}

#[test]
fn shell_transfer_registration_does_not_query_default_or_confirmation_preferences() {
    let mut queries = Vec::new();
    assert!(registered_with(|key| {
        queries.push(key.to_string());
        match key {
            key if key == HANDLER_KEYS[0] => Some(CLASS_ID.into()),
            SERVER_KEY => Some(SERVER.into()),
            _ => panic!("Only handler and COM registration should be queried"),
        }
    }));
    assert_eq!(queries, [HANDLER_KEYS[0], SERVER_KEY]);
}

#[test]
fn either_directory_or_folder_handler_is_sufficient() {
    for key in HANDLER_KEYS {
        assert!(registered(&[(key, CLASS_ID), (SERVER_KEY, SERVER)]));
    }
}

#[test]
fn com_registration_without_the_copy_handler_does_not_activate_integration() {
    assert!(!registered(&[]));
    assert!(!registered(&[(SERVER_KEY, SERVER)]));
}

#[test]
fn leftover_handler_without_a_com_server_does_not_activate_integration() {
    assert!(!registered(&[(HANDLER_KEYS[0], CLASS_ID)]));
    for server in ["", " ", "\t"] {
        assert!(!registered(&[
            (HANDLER_KEYS[0], CLASS_ID),
            (SERVER_KEY, server)
        ]));
    }
}

#[test]
fn malformed_handler_class_ids_are_rejected() {
    for class in [
        "",
        "invalid",
        r"..\Other",
        "{11111111-2222-3333-4444-555555555555",
        "11111111-2222-3333-4444-555555555555}",
    ] {
        assert!(!registered(&[
            (HANDLER_KEYS[0], class),
            (SERVER_KEY, SERVER)
        ]));
    }
}

#[test]
fn a_broken_directory_handler_does_not_hide_a_valid_folder_handler() {
    assert!(registered(&[
        (HANDLER_KEYS[0], "invalid"),
        (HANDLER_KEYS[1], CLASS_ID),
        (SERVER_KEY, SERVER),
    ]));
}

#[test]
fn class_ids_are_normalized_before_com_lookup() {
    for class in [
        CLASS_ID,
        "11111111-2222-3333-4444-555555555555",
        " {11111111-2222-3333-4444-555555555555} ",
    ] {
        assert_eq!(class_server_key(class).as_deref(), Some(SERVER_KEY));
    }
}
