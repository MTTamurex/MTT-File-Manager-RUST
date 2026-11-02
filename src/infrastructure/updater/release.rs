use semver::Version;
use serde::Deserialize;
use std::io::Read;

const API: &str = "https://api.github.com/repos/MTTamurex/MTT-File-Manager-RUST/releases/latest";
const RELEASE_PREFIX: &str = "https://github.com/MTTamurex/MTT-File-Manager-RUST/releases/tag/";

#[derive(Clone, Debug)]
pub(crate) struct Release {
    pub version: Version,
    pub notes: String,
    pub url: String,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    body: Option<String>,
    draft: bool,
    prerelease: bool,
}

pub(crate) fn check() -> Result<Option<Release>, String> {
    let response = super::client()?
        .get(API)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .map_err(|_| "Could not contact GitHub".to_string())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let response = response
        .error_for_status()
        .map_err(|_| "GitHub update request failed (possibly rate limited)".to_string())?;
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read the release information".to_string())?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("Release information exceeds the size limit".into());
    }
    parse(&bytes, env!("CARGO_PKG_VERSION"))
}

fn parse(bytes: &[u8], current: &str) -> Result<Option<Release>, String> {
    let release: ApiRelease = serde_json::from_slice(bytes)
        .map_err(|_| "Invalid GitHub release information".to_string())?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .map_err(|_| "Invalid release version".to_string())?;
    if !version
        .cmp_precedence(
            &Version::parse(current).map_err(|_| "Invalid application version".to_string())?,
        )
        .is_gt()
    {
        return Ok(None);
    }
    // Construct the page URL from the fixed official repository and the
    // validated version tag; do not open URLs supplied by the release body.
    Ok(Some(Release {
        version,
        notes: release.body.unwrap_or_default(),
        url: format!("{RELEASE_PREFIX}{}", release.tag_name),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> serde_json::Value {
        serde_json::json!({"tag_name": tag, "body": "Release notes", "draft": false,
            "prerelease": false})
    }

    #[test]
    fn compares_versions_numerically_and_accepts_normal_beta_releases() {
        let bytes = serde_json::to_vec(&fixture("v0.2.10")).unwrap();
        assert!(parse(&bytes, "0.2.9").unwrap().is_some());
        assert!(parse(&bytes, "0.2.10").unwrap().is_none());
        assert!(parse(&bytes, "0.3.0").unwrap().is_none());
    }

    #[test]
    fn ignores_drafts_and_prereleases() {
        for flag in ["draft", "prerelease"] {
            let mut data = fixture("0.3.0");
            data[flag] = true.into();
            assert!(parse(&serde_json::to_vec(&data).unwrap(), "0.2.8")
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn build_metadata_does_not_create_an_upgrade_or_downgrade() {
        for (tag, current) in [
            ("v0.3.0+build.2", "0.3.0+build.1"),
            ("v0.3.0+build.1", "0.3.0+build.2"),
            ("v0.3.0+build.1", "0.3.0"),
        ] {
            assert!(parse(&serde_json::to_vec(&fixture(tag)).unwrap(), current)
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn opens_the_official_release_page_without_requiring_download_assets() {
        let bytes = serde_json::to_vec(&fixture("v0.3.0")).unwrap();
        let release = parse(&bytes, "0.2.10").unwrap().unwrap();
        assert_eq!(release.url, format!("{RELEASE_PREFIX}v0.3.0"));
        assert_eq!(release.notes, "Release notes");
    }

    #[test]
    fn ignores_api_supplied_page_urls() {
        let mut data = fixture("v0.3.0");
        data["html_url"] = "https://example.invalid/untrusted".into();
        let release = parse(&serde_json::to_vec(&data).unwrap(), "0.2.10")
            .unwrap()
            .unwrap();
        assert_eq!(release.url, format!("{RELEASE_PREFIX}v0.3.0"));
    }

    #[test]
    fn rejects_invalid_tags_before_constructing_a_browser_url() {
        for tag in [
            "../../other",
            "v0.3.0?redirect=other",
            "https://example.invalid",
            "v0.2.9.1",
        ] {
            assert!(parse(&serde_json::to_vec(&fixture(tag)).unwrap(), "0.2.10").is_err());
        }
    }
}
