//! Presentation only: leave operation paths and non-DOS device names intact.
pub(super) fn text(value: &str) -> String {
    value.split('\n').map(line).collect::<Vec<_>>().join("\n")
}

fn line(value: &str) -> String {
    if let Some(rest) = value.strip_prefix(r"\\?\") {
        if rest
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"UNC\"))
        {
            return format!(r"\\{}", &rest[4..]);
        }
        let bytes = rest.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'\\'
        {
            return rest.to_owned();
        }
    }
    value.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_display_prefixes_are_changed() {
        for (input, expected) in [
            (r"\\?\C:\Documents\file.txt", r"C:\Documents\file.txt"),
            (r"\\?\UNC\server\share\file.txt", r"\\server\share\file.txt"),
            (r"\\?\unc\server\share", r"\\server\share"),
            (
                r"\\?\Volume{example}\file.txt",
                r"\\?\Volume{example}\file.txt",
            ),
            (r"C:\Documents\file.txt", r"C:\Documents\file.txt"),
            (r"\\server\share", r"\\server\share"),
            ("source/file.txt", "source/file.txt"),
        ] {
            assert_eq!(text(input), expected);
        }
        assert_eq!(
            text("\\\\?\\C:\\Documents\\a.txt\n\\\\?\\D:\\Documents\\b.txt"),
            "C:\\Documents\\a.txt\nD:\\Documents\\b.txt"
        );
    }
}
