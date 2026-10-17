use super::*;
use windows_crate::Win32::System::Com::{
    CoGetApartmentType, CoInitializeEx, CoUninitialize, APTTYPE, APTTYPEQUALIFIER,
    APTTYPEQUALIFIER_IMPLICIT_MTA, APTTYPE_MAINSTA, APTTYPE_MTA, APTTYPE_STA, COINIT_MULTITHREADED,
};

#[test]
fn icon_guard_owns_an_explicit_sta_on_a_fresh_worker() {
    std::thread::spawn(|| {
        let _com = ComStaGuard::new();
        assert!(_com.needs_uninit);
        let mut kind = APTTYPE::default();
        let mut qualifier = APTTYPEQUALIFIER::default();
        unsafe { CoGetApartmentType(&mut kind, &mut qualifier) }.unwrap();
        assert!(kind == APTTYPE_STA || kind == APTTYPE_MAINSTA);
        assert_ne!(qualifier, APTTYPEQUALIFIER_IMPLICIT_MTA);
    })
    .join()
    .unwrap();
}

#[test]
fn icon_guard_does_not_uninitialize_an_existing_mta() {
    std::thread::spawn(|| {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .unwrap();
        {
            let com = ComStaGuard::new();
            assert!(!com.needs_uninit);
        }
        let mut kind = APTTYPE::default();
        let mut qualifier = APTTYPEQUALIFIER::default();
        unsafe { CoGetApartmentType(&mut kind, &mut qualifier) }.unwrap();
        assert_eq!(kind, APTTYPE_MTA);
        assert_ne!(qualifier, APTTYPEQUALIFIER_IMPLICIT_MTA);
        unsafe { CoUninitialize() };
    })
    .join()
    .unwrap();
}

#[test]
fn drive_and_folder_workers_finish_when_unrelated_mta_threads_exit() {
    let directory = tempfile::tempdir().unwrap();
    let drive_path = directory.path().join("drive-folder");
    let folder_path = directory.path().join("folder");
    std::fs::create_dir(&drive_path).unwrap();
    std::fs::create_dir(&folder_path).unwrap();
    let drive_path = drive_path.to_str().unwrap();
    let folder_path = folder_path.to_str().unwrap();
    let ctx = egui::Context::default();
    for _ in 0..8 {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let unrelated = std::thread::spawn(move || {
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
                .ok()
                .unwrap();
            ready_tx.send(()).unwrap();
            let _ = release_rx.recv();
            unsafe { CoUninitialize() };
        });
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        let mut loader = IconLoader::new();
        loader.get_or_load_drive_icon(&ctx, drive_path);
        loader.get_or_load_folder_path_icon(&ctx, folder_path);
        // The extraction threads must own COM, not borrow this MTA's lifetime.
        drop(release_tx);
        unrelated.join().unwrap();
        let first = loader
            .icon_result_rx
            .recv_timeout(Duration::from_secs(20))
            .unwrap();
        let second = loader
            .icon_result_rx
            .recv_timeout(Duration::from_secs(20))
            .unwrap();
        assert_ne!(first.key, second.key);
        for result in [first, second] {
            // Shell extraction can legitimately fail (including on headless
            // hosts); the contract is a response and safe COM cleanup either way.
            if let Some((pixels, width, height)) = result.data {
                assert!(width > 0 && height > 0);
                assert_eq!(pixels.len(), width as usize * height as usize * 4);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        while loader.auxiliary_icon_threads.load(Ordering::Acquire) != 0 {
            assert!(
                Instant::now() < deadline,
                "icon worker did not finish COM cleanup"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
