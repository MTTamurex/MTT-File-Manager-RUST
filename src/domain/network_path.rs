//! UNC server destinations are containers of shares, not filesystem directories.

use std::path::{Component, Path, PathBuf, Prefix};

/// Recognize a server-only UNC address without performing network I/O.
pub fn unc_server_name(path: &str) -> Option<&str> {
    let server = path.strip_prefix(r"\\")?;
    let server = server.strip_suffix('\\').unwrap_or(server);
    if server.is_empty()
        || server == "."
        || server == ".."
        || server.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
    {
        return None;
    }
    Some(server)
}

/// Rust treats a UNC share as a filesystem root; its navigable parent is the server.
pub fn unc_share_server_path(path: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    let Component::Prefix(prefix) = components.next()? else {
        return None;
    };
    let server = match prefix.kind() {
        Prefix::UNC(server, _) | Prefix::VerbatimUNC(server, _) => server,
        _ => return None,
    };
    if components.any(|component| !matches!(component, Component::RootDir)) {
        return None;
    }
    Some(PathBuf::from(format!(r"\\{}", server.to_str()?)))
}

/// Server containers and share roots cannot be moved, renamed or deleted as files.
pub fn is_network_location_root(path: &Path) -> bool {
    unc_server_name(&path.to_string_lossy()).is_some() || unc_share_server_path(path).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_locations_are_distinct_from_items_inside_shares() {
        for path in [
            r"\\server",
            r"\\server\",
            r"\\server\share",
            r"\\server\share\",
            r"\\?\UNC\server\share",
        ] {
            assert!(is_network_location_root(Path::new(path)), "{path}");
        }
        for path in [
            r"C:\",
            r"C:\Folder",
            r"\\server\share\folder",
            r"\\server\share\file.txt",
            r"\\?\UNC\server\share\folder",
        ] {
            assert!(!is_network_location_root(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn recognizes_only_server_destinations() {
        for path in [
            r"\\server",
            r"\\server\",
            r"\\192.0.2.1",
            r"\\server.example",
        ] {
            assert!(unc_server_name(path).is_some(), "{path}");
        }
        for path in [
            "",
            r"C:\",
            r"relative",
            r"\server",
            r"\\",
            r"\\server\share",
            r"\\server\share\folder",
            r"\\?\C:\",
            r"\\?\UNC\server\share",
            r"\\.\",
            r"\\..",
            r"\\server\\",
            "\\\\server\0",
            r"\\server*",
        ] {
            assert_eq!(unc_server_name(path), None, "{path:?}");
        }
        assert_eq!(unc_server_name(r"\\server\"), Some("server"));
    }

    #[test]
    fn share_roots_navigate_up_to_their_server() {
        for path in [
            r"\\server\share",
            r"\\server\share\",
            r"\\?\UNC\server\share\",
        ] {
            assert_eq!(
                unc_share_server_path(Path::new(path)),
                Some(PathBuf::from(r"\\server"))
            );
        }
        for path in [r"C:\", r"\\server", r"\\server\share\folder"] {
            assert_eq!(unc_share_server_path(Path::new(path)), None);
        }
    }
}
