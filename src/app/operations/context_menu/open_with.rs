//! Selection policy shared by initial and asynchronously loaded context-menu items.

pub(super) fn should_offer_open_with(target_is_file: bool, path_count: usize) -> bool {
    target_is_file && path_count == 1
}

pub(super) fn should_hide_shell_open_with(
    path_count: usize,
    command: Option<&str>,
    text: &str,
) -> bool {
    path_count != 1 && crate::application::context_menu::is_generic_open_with_command(command, text)
}

#[cfg(test)]
mod tests {
    use super::{should_hide_shell_open_with, should_offer_open_with};

    #[test]
    fn offers_open_with_only_for_a_single_eligible_file() {
        assert!(should_offer_open_with(true, 1));
        for path_count in [0, 2, 3, 100] {
            assert!(!should_offer_open_with(true, path_count));
        }
        for path_count in [0, 1, 2] {
            assert!(!should_offer_open_with(false, path_count));
        }
    }

    #[test]
    fn hides_generic_shell_open_with_for_multiple_targets() {
        for (command, text) in [
            (Some("openas"), "Open with"),
            (Some("OPENAS"), "Abrir com"),
            (Some("open_with_shell_fallback"), "Open with"),
            (None, "Open with..."),
            (None, "Abrir com…"),
        ] {
            assert!(!should_hide_shell_open_with(1, command, text));
            for path_count in [0, 2, 3] {
                assert!(should_hide_shell_open_with(path_count, command, text));
            }
        }
    }

    #[test]
    fn preserves_third_party_and_unrelated_shell_commands() {
        for (command, text) in [
            (Some("third_party_open"), "Open with Example App"),
            (None, "Open with Example App"),
            (Some("third_party_open"), "Open with"),
            (Some("extractall"), "Extract all"),
            (None, "Properties"),
        ] {
            assert!(!should_hide_shell_open_with(2, command, text));
        }
    }
}
