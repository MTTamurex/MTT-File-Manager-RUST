//! Optional end-to-end routing checks; no production failure-injection switches.
use super::*;
use crate::infrastructure::mpv_thumbnails::Extractor;
use std::{cell::Cell, path::PathBuf};
use windows::Win32::{
    Media::MediaFoundation::{MFShutdown, MFStartup, MFSTARTUP_NOSOCKET},
    System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
};

struct NativeInit {
    mf: bool,
}

impl NativeInit {
    fn new() -> Self {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .unwrap();
        let mut init = Self { mf: false };
        unsafe { MFStartup(0x00020070, MFSTARTUP_NOSOCKET) }.unwrap();
        init.mf = true;
        init
    }
}

impl Drop for NativeInit {
    fn drop(&mut self) {
        unsafe {
            if self.mf {
                let _ = MFShutdown();
            }
            CoUninitialize();
        }
    }
}

#[test]
#[ignore = "Requires MTT_MPV_TEST_APP, a synthetic H.264 MP4 in MTT_MPV_TEST_VIDEO, bundled DLL, and Windows thumbnail providers"]
fn real_helper_success_skips_windows_and_real_windows_recovers_from_primary_failures() {
    let _ = env_logger::Builder::new()
        .filter_module(
            "mtt_file_manager::workers::thumbnail::extraction",
            log::LevelFilter::Trace,
        )
        .is_test(true)
        .try_init();
    let _native = NativeInit::new();
    let app = PathBuf::from(std::env::var_os("MTT_MPV_TEST_APP").expect("Set MTT_MPV_TEST_APP"));
    let source =
        PathBuf::from(std::env::var_os("MTT_MPV_TEST_VIDEO").expect("Set MTT_MPV_TEST_VIDEO"));
    let mut extractor = Extractor::new(app.canonicalize().unwrap());
    // Windows Shell/MF do not uniformly accept canonicalize()'s verbatim prefix.
    // Use the regular Win32 path, as supplied by normal folder enumeration.
    let source = std::path::absolute(source).unwrap();
    let result = run(
        || extractor.extract(&source, 256, &|| false),
        || panic!("Successful libmpv extraction must skip Windows"),
        &|| false,
        || windows_fallback::check_source(&source, &|| false),
    );
    assert!(matches!(result, Outcome::Success(_)));

    for error in [
        Error::Decode,
        Error::Unavailable,
        Error::Transport,
        Error::Timeout,
    ] {
        let called = Cell::new(false);
        let result = run(
            || Err(error),
            || {
                called.set(true);
                windows_fallback::extract(&source, Some(256), &|| false)
            },
            &|| false,
            || windows_fallback::check_source(&source, &|| false),
        );
        assert!(called.get(), "{error:?}");
        let Outcome::Success((pixels, width, height)) = result else {
            panic!("Windows provider must support the synthetic H.264 fixture: {result:?}");
        };
        assert!(width > 0 && height > 0);
        assert_eq!(pixels.len(), width as usize * height as usize * 4);
    }
}
