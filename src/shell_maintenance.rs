//! Installer-only restoration before any UI or startup migration runs.

const RESTORE_FLAG: &str = "--restore-folder-integration";

fn is_restore_request(arguments: &[std::ffi::OsString]) -> bool {
    matches!(arguments, [flag] if flag == RESTORE_FLAG)
}

pub(crate) fn run_if_requested() {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if is_restore_request(&arguments) {
        let result =
            mtt_file_manager::infrastructure::windows::default_file_manager::restore_explorer();
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_requires_the_exact_standalone_maintenance_flag() {
        assert!(is_restore_request(&[RESTORE_FLAG.into()]));
        assert!(!is_restore_request(&[]));
        assert!(!is_restore_request(&[
            "--open-folder".into(),
            RESTORE_FLAG.into()
        ]));
        assert!(!is_restore_request(&[RESTORE_FLAG.into(), "extra".into()]));
    }
}
