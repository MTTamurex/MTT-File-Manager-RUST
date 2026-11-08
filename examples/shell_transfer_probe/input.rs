use std::ffi::OsString;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub struct CopyInput {
    pub destination: PathBuf,
    pub sources: Vec<PathBuf>,
    pub observe: bool,
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Option<CopyInput>, String> {
    let mut args = args.into_iter().peekable();
    let observe = args.peek().is_some_and(|arg| arg == "--observe");
    if observe {
        args.next();
    }
    let Some(destination) = args.next() else {
        if observe {
            return Err("Specify a destination and source after --observe".into());
        }
        return Ok(None);
    };
    if destination == "--help" || destination == "-h" {
        return Ok(None);
    }
    let input = CopyInput {
        destination: destination.into(),
        sources: args.map(PathBuf::from).collect(),
        observe,
    };
    if input.sources.is_empty() {
        return Err("Specify a destination followed by at least one source".into());
    }
    for path in std::iter::once(&input.destination).chain(&input.sources) {
        if !path.is_absolute() || path.as_os_str().encode_wide().any(|unit| unit == 0) {
            return Err("Every argument must be an absolute filesystem path without NULs".into());
        }
    }
    Ok(Some(input))
}

fn same_or_descendant(candidate: &Path, ancestor: &Path) -> bool {
    let mut candidate = candidate.components();
    ancestor.components().all(|part| {
        candidate.next().is_some_and(|other| {
            part.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&other.as_os_str().to_string_lossy())
        })
    })
}

impl CopyInput {
    pub fn validate(self) -> Result<Self, String> {
        let destination = self
            .destination
            .canonicalize()
            .map_err(|_| "Could not resolve the destination")?;
        if !destination.is_dir() {
            return Err("The destination must be an existing filesystem folder".into());
        }
        let sources = self
            .sources
            .into_iter()
            .map(|source| {
                let source = source
                    .canonicalize()
                    .map_err(|_| "Could not resolve a source item")?;
                if source.file_name().is_none() {
                    return Err("Drive and share roots are not supported as sources".into());
                }
                if same_or_descendant(&destination, &source)
                    || source.parent().is_some_and(|parent| {
                        same_or_descendant(&destination, parent)
                            && same_or_descendant(parent, &destination)
                    })
                {
                    return Err("Copying into the source or its existing parent is not supported by this probe".into());
                }
                Ok(source)
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            destination,
            sources,
            observe: self.observe,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn no_arguments_and_help_do_not_start_a_transfer() {
        assert!(parse(args(&[])).unwrap().is_none());
        assert!(parse(args(&["--help"])).unwrap().is_none());
    }

    #[test]
    fn accepts_multiple_sources_from_different_parents() {
        let input = parse(args(&[
            r"D:\Destination",
            r"C:\Source\file.txt",
            r"C:\Other\日本語.txt",
        ]))
        .unwrap()
        .unwrap();
        assert_eq!(input.sources.len(), 2);
        assert!(!input.observe);
    }

    #[test]
    fn observation_requires_explicit_flag_and_complete_arguments() {
        let input = parse(args(&[
            "--observe",
            r"D:\Destination",
            r"C:\Source\file.txt",
        ]))
        .unwrap()
        .unwrap();
        assert!(input.observe);
        assert!(parse(args(&["--observe"])).is_err());
        assert!(parse(args(&["--observe", r"D:\Destination"])).is_err());
    }

    #[test]
    fn rejects_missing_sources_relative_paths_and_nuls() {
        assert!(parse(args(&[r"D:\Destination"])).is_err());
        assert!(parse(args(&[r"D:\Destination", "file.txt"])).is_err());
        assert!(parse(args(&["Destination", r"C:\Source\file.txt"])).is_err());
        assert!(parse(args(&[r"D:\Destination", "C:\\Source\\bad\0.txt"])).is_err());
    }

    #[test]
    fn containment_is_component_based_and_case_insensitive() {
        let source = Path::new(r"C:\Source");
        assert!(same_or_descendant(Path::new(r"c:\source\Child"), source));
        assert!(same_or_descendant(Path::new(r"C:\Source"), source));
        assert!(!same_or_descendant(Path::new(r"C:\SourceOther"), source));
        assert!(!same_or_descendant(Path::new(r"D:\Source"), source));
    }
}
