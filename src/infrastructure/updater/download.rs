use super::Release;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

pub(crate) struct DownloadedUpdate {
    pub directory: tempfile::TempDir,
    pub digest: String,
}

impl DownloadedUpdate {
    pub(crate) fn spawn_helper(&self) -> Result<super::HelperProcess, String> {
        let exe =
            std::env::current_exe().map_err(|_| "Could not locate the application".to_string())?;
        let source = exe
            .parent()
            .ok_or("Could not locate the updater")?
            .join("mtt-updater.exe");
        let helper = self.directory.path().join("mtt-updater.exe");
        std::fs::copy(source, &helper).map_err(|_| {
            "Could not prepare the updater; install the latest full setup manually".to_string()
        })?;
        std::process::Command::new(helper)
            .arg(std::process::id().to_string())
            .arg(exe)
            .arg(&self.digest)
            .spawn()
            .map(super::HelperProcess::new)
            .map_err(|_| "Could not start the updater".into())
    }
}

pub(crate) fn download(
    release: &Release,
    progress: impl FnMut(u64, u64),
) -> Result<DownloadedUpdate, String> {
    let directory = tempfile::Builder::new()
        .prefix("mtt-update-")
        .tempdir()
        .map_err(|_| "Could not create the update directory".to_string())?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.path().join("setup.exe"))
        .map_err(|_| "Could not create the installer file".to_string())?;
    let mut response = super::client()?
        .get(&release.url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|_| "Installer download failed".to_string())?;
    copy_verified(
        &mut response,
        &mut output,
        release.size,
        &release.digest,
        progress,
    )?;
    output
        .sync_all()
        .map_err(|_| "Could not flush the installer".to_string())?;
    Ok(DownloadedUpdate {
        directory,
        digest: release.digest.clone(),
    })
}

fn copy_verified(
    input: &mut impl Read,
    output: &mut impl Write,
    expected_size: u64,
    digest: &str,
    mut progress: impl FnMut(u64, u64),
) -> Result<(), String> {
    let mut hash = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = input
            .read(&mut buffer)
            .map_err(|_| "Installer download interrupted".to_string())?;
        if read == 0 {
            break;
        }
        count += read as u64;
        if count > expected_size {
            return Err("Installer exceeds the expected size".into());
        }
        output
            .write_all(&buffer[..read])
            .map_err(|_| "Could not save the installer".to_string())?;
        hash.update(&buffer[..read]);
        progress(count, expected_size);
    }
    if count != expected_size || !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(digest) {
        return Err("Installer integrity verification failed".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_content_and_reports_progress() {
        let data = b"installer contents";
        let digest = format!("{:x}", Sha256::digest(data));
        let mut output = Vec::new();
        let mut final_progress = (0, 0);
        copy_verified(
            &mut &data[..],
            &mut output,
            data.len() as u64,
            &digest,
            |done, size| final_progress = (done, size),
        )
        .unwrap();
        assert_eq!(output, data);
        assert_eq!(final_progress, (data.len() as u64, data.len() as u64));
    }

    #[test]
    fn rejects_truncated_oversized_and_tampered_downloads() {
        for (size, digest) in [
            (4, format!("{:x}", Sha256::digest(b"abc"))),
            (2, format!("{:x}", Sha256::digest(b"abc"))),
            (3, "0".repeat(64)),
        ] {
            assert!(
                copy_verified(&mut &b"abc"[..], &mut Vec::new(), size, &digest, |_, _| {}).is_err()
            );
        }
    }

    #[test]
    fn rejects_interrupted_read() {
        struct Interrupted;
        impl Read for Interrupted {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "interrupted",
                ))
            }
        }
        assert!(copy_verified(
            &mut Interrupted,
            &mut Vec::new(),
            3,
            &"0".repeat(64),
            |_, _| {}
        )
        .is_err());
    }
}
