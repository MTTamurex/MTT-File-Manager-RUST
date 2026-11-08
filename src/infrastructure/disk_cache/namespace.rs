use std::path::PathBuf;

pub(super) fn directory(base: PathBuf) -> PathBuf {
    #[cfg(feature = "mpv-thumbnails-poc")]
    {
        base.join("mpv-poc-v3")
    }
    #[cfg(not(feature = "mpv-thumbnails-poc"))]
    {
        base
    }
}

pub(super) fn fallback_file_name() -> &'static str {
    if cfg!(feature = "mpv-thumbnails-poc") {
        "thumbnails_fallback_mpv_poc_v3.db"
    } else {
        "thumbnails_fallback.db"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn experiment_never_opens_the_normal_thumbnail_store() {
        let base = PathBuf::from("cache");
        if cfg!(feature = "mpv-thumbnails-poc") {
            assert_eq!(directory(base.clone()), base.join("mpv-poc-v3"));
            assert_ne!(directory(base.clone()), base.join("mpv-poc-v1"));
            assert_ne!(directory(base.clone()), base.join("mpv-poc-v2"));
            assert_ne!(fallback_file_name(), "thumbnails_fallback.db");
            assert_ne!(fallback_file_name(), "thumbnails_fallback_mpv_poc_v1.db");
            assert_ne!(fallback_file_name(), "thumbnails_fallback_mpv_poc_v2.db");
        } else {
            assert_eq!(directory(base.clone()), base);
            assert_eq!(fallback_file_name(), "thumbnails_fallback.db");
        }
    }

    #[test]
    fn thumbnail_migration_uses_the_constructor_namespace() {
        let directory = tempfile::tempdir().unwrap();
        let cache = super::super::ThumbnailDiskCache::new(directory.path().to_path_buf()).unwrap();
        assert_eq!(
            cache.primary_database_path(),
            self::directory(directory.path().to_path_buf()).join("thumbnails.db")
        );
    }
}
