mod download;
mod helper;
pub(crate) mod platform;
mod release;

pub(crate) use download::{download, DownloadedUpdate};
pub(crate) use helper::HelperProcess;
pub(crate) use release::{check, Release};

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("MTT-File-Manager/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(600))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let host = attempt.url().host_str().unwrap_or_default();
            if attempt.previous().len() < 10
                && (host == "github.com" || host.ends_with(".githubusercontent.com"))
            {
                attempt.follow()
            } else {
                attempt.error("Untrusted update redirect")
            }
        }))
        .build()
        .map_err(|_| "Could not initialize the update connection".into())
}
