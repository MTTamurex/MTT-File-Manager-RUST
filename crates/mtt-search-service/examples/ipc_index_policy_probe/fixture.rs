use mtt_search_protocol::SearchRequest;

use super::checks::{Check, Kind};

pub(super) struct Fixture {
    root: tempfile::TempDir,
    marker_name: String,
}

impl Fixture {
    pub fn new() -> Result<Self, &'static str> {
        let root = tempfile::Builder::new()
            .prefix("mtt-ipc-policy-")
            .tempdir()
            .map_err(|_| "cannot create temporary synthetic directory")?;
        let root_name = root
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("synthetic directory name is not valid UTF-8")?;
        let marker_name = format!("{root_name}-marker.txt");
        std::fs::write(
            root.path().join(&marker_name),
            b"synthetic IPC diagnostic fixture\n",
        )
        .map_err(|_| "cannot write synthetic marker")?;
        // Empty target prevents a regressed FolderSize gate from requesting
        // size repair for a nonempty synthetic subtree with pending metrics.
        std::fs::create_dir(root.path().join("empty-folder"))
            .map_err(|_| "cannot create empty synthetic folder")?;
        Ok(Self { root, marker_name })
    }

    pub fn checks(&self) -> Result<[Check; 3], &'static str> {
        let root = self
            .root
            .path()
            .to_str()
            .ok_or("synthetic fixture path is not valid UTF-8")?;
        let bytes = root.as_bytes();
        if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || &bytes[1..3] != b":\\" {
            return Err("synthetic fixture requires a local drive-rooted temporary directory");
        }
        let empty = self
            .root
            .path()
            .join("empty-folder")
            .to_str()
            .ok_or("synthetic folder path is not valid UTF-8")?
            .to_owned();
        let checks = [
            Check {
                kind: Kind::Query,
                request: SearchRequest::Query {
                    text: self.marker_name.clone(),
                    offset: 0,
                    limit: 1,
                },
            },
            Check {
                kind: Kind::FolderSize,
                request: SearchRequest::FolderSize { path: empty },
            },
            Check {
                kind: Kind::CheckPathsModified,
                request: SearchRequest::CheckPathsModified {
                    paths: vec![root.to_owned()],
                    threshold_secs: 60,
                },
            },
        ];
        for check in &checks {
            check
                .request
                .validate()
                .map_err(|_| "synthetic request validation failed")?;
        }
        Ok(checks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_narrow_and_only_reference_the_owned_fixture() {
        let fixture = Fixture::new().unwrap();
        let [query, folder, modified] = fixture.checks().unwrap();
        match query.request {
            SearchRequest::Query {
                text,
                offset,
                limit,
            } => {
                assert_eq!(text, fixture.marker_name);
                assert!(fixture.root.path().join(text).is_file());
                assert_eq!((offset, limit), (0, 1));
            }
            _ => panic!("expected marker query"),
        }
        match folder.request {
            SearchRequest::FolderSize { path } => {
                assert!(std::path::Path::new(&path).parent() == Some(fixture.root.path()));
                assert!(std::fs::read_dir(path).unwrap().next().is_none());
            }
            _ => panic!("expected empty-folder request"),
        }
        match modified.request {
            SearchRequest::CheckPathsModified {
                paths,
                threshold_secs,
            } => {
                assert_eq!(paths.len(), 1);
                assert!(std::path::Path::new(&paths[0]) == fixture.root.path());
                assert_eq!(threshold_secs, 60);
            }
            _ => panic!("expected synthetic-root activity request"),
        }
    }

    #[test]
    fn separate_fixtures_use_different_search_markers() {
        let first = Fixture::new().unwrap();
        let second = Fixture::new().unwrap();
        assert_ne!(first.marker_name, second.marker_name);
    }
}
