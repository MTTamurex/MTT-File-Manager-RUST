use semver::Version;
use serde::Deserialize;
use std::io::Read;

const API: &str = "https://api.github.com/repos/MTTamurex/MTT-File-Manager-RUST/releases/latest";
const ASSET_PREFIX: &str = "https://github.com/MTTamurex/MTT-File-Manager-RUST/releases/download/";

#[derive(Clone, Debug)]
pub(crate) struct Release {
    pub version: Version,
    pub notes: String,
    pub url: String,
    pub size: u64,
    pub digest: String,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    body: Option<String>,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
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
    let name = format!("MTT-File-Manager-Setup-{version}.exe");
    let mut matching = release
        .assets
        .into_iter()
        .filter(|asset| asset.name == name);
    let asset = matching
        .next()
        .ok_or("The release has no compatible installer")?;
    let asset_url = reqwest::Url::parse(&asset.browser_download_url)
        .map_err(|_| "Invalid release installer URL".to_string())?;
    if matching.next().is_some()
        || !asset.browser_download_url.starts_with(ASSET_PREFIX)
        || !asset_url
            .path()
            .starts_with("/MTTamurex/MTT-File-Manager-RUST/releases/download/")
        || asset.size == 0
        || asset.size > 2 * 1024 * 1024 * 1024
    {
        return Err("Invalid release installer".into());
    }
    let digest = asset
        .digest
        .and_then(|value| value.strip_prefix("sha256:").map(str::to_owned))
        .filter(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("The installer has no valid GitHub SHA-256 digest")?;
    Ok(Some(Release {
        version,
        notes: release.body.unwrap_or_default(),
        url: asset.browser_download_url,
        size: asset.size,
        digest,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> serde_json::Value {
        serde_json::json!({"tag_name": tag, "body": "Release notes", "draft": false,
            "prerelease": false, "assets": [{"name": format!("MTT-File-Manager-Setup-{}.exe", tag.trim_start_matches('v')),
            "browser_download_url": format!("{ASSET_PREFIX}{tag}/setup.exe"), "size": 123,
            "digest": format!("sha256:{}", "a".repeat(64))}]})
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
    fn rejects_missing_digest_untrusted_url_and_missing_installer() {
        for field in ["digest", "browser_download_url", "name"] {
            let mut data = fixture("0.3.0");
            data["assets"][0][field] = "invalid".into();
            assert!(parse(&serde_json::to_vec(&data).unwrap(), "0.2.8").is_err());
        }
    }

    #[test]
    fn rejects_ambiguous_or_oversized_installers() {
        let mut data = fixture("0.3.0");
        let duplicate = data["assets"][0].clone();
        data["assets"].as_array_mut().unwrap().push(duplicate);
        assert!(parse(&serde_json::to_vec(&data).unwrap(), "0.2.8").is_err());
        for size in [0u64, 2 * 1024 * 1024 * 1024 + 1] {
            let mut data = fixture("0.3.0");
            data["assets"][0]["size"] = size.into();
            assert!(parse(&serde_json::to_vec(&data).unwrap(), "0.2.8").is_err());
        }
    }

    #[test]
    fn rejects_path_traversal_outside_the_official_release_directory() {
        let mut data = fixture("0.3.0");
        data["assets"][0]["browser_download_url"] =
            format!("{ASSET_PREFIX}../../../../other/setup.exe").into();
        assert!(parse(&serde_json::to_vec(&data).unwrap(), "0.2.8").is_err());
    }
}
