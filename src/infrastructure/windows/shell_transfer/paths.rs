use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, Prefix};

pub(super) fn parsing_name(path: &Path) -> Vec<u16> {
    // Filesystem validation can produce verbatim names. Convert only DOS/UNC
    // prefixes at the Shell boundary, without lossy UTF-16 conversions.
    let encoded: Vec<u16> = path.as_os_str().encode_wide().collect();
    let mut name = match path.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(_) => encoded[4..].to_vec(),
            Prefix::VerbatimUNC(_, _) => [vec![b'\\' as u16; 2], encoded[8..].to_vec()].concat(),
            _ => encoded,
        },
        _ => encoded,
    };
    name.push(0);
    name
}
