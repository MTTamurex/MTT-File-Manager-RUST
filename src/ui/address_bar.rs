use crate::domain::special_paths::{COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use rust_i18n::t;

pub(crate) fn editable_path(current_path: &str, display_override: Option<&str>) -> String {
    if let Some(display) = display_override {
        display.to_string()
    } else if current_path == COMPUTER_VIEW_ID {
        t!("nav.computer").to_string()
    } else if current_path == RECYCLE_BIN_VIEW_ID {
        t!("nav.recycle_bin").to_string()
    } else {
        current_path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_virtual_paths_for_editing() {
        assert_eq!(editable_path(COMPUTER_VIEW_ID, None), t!("nav.computer"));
        assert_eq!(
            editable_path(RECYCLE_BIN_VIEW_ID, None),
            t!("nav.recycle_bin")
        );
        assert_eq!(editable_path("::tag::42", Some("Tag: Work")), "Tag: Work");
    }

    #[test]
    fn preserves_filesystem_paths_for_editing() {
        assert_eq!(editable_path(r"C:\Users", None), r"C:\Users");
    }
}
