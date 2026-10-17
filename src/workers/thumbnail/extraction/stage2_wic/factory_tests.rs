use super::*;
use windows::core::Interface;
use windows::Win32::System::Com::{
    CoGetApartmentType, APTTYPE, APTTYPEQUALIFIER, APTTYPE_MAINSTA, APTTYPE_MTA, APTTYPE_STA,
};

fn assert_empty_cache() {
    CACHE.with(|cache| {
        let cache = cache.borrow();
        assert_eq!(cache.scopes, 0);
        assert!(cache.factory.is_none());
    });
}

fn apartment() -> (APTTYPE, APTTYPEQUALIFIER) {
    let mut kind = APTTYPE::default();
    let mut qualifier = APTTYPEQUALIFIER::default();
    unsafe { CoGetApartmentType(&mut kind, &mut qualifier) }.unwrap();
    (kind, qualifier)
}

#[test]
fn nested_decode_scopes_reuse_factory_until_the_worker_scope_ends() {
    std::thread::spawn(|| {
        assert_empty_cache();
        assert!(get_or_create_factory().is_none());
        {
            let _worker = WicContext::new().unwrap();
            let first = get_or_create_factory().unwrap();
            let pointer = first.as_raw();
            drop(first);
            {
                let _decode = WicContext::new().unwrap();
                let reused = get_or_create_factory().unwrap();
                assert_eq!(reused.as_raw(), pointer);
                CACHE.with(|cache| assert_eq!(cache.borrow().scopes, 2));
            }
            CACHE.with(|cache| assert_eq!(cache.borrow().scopes, 1));
            let reused = get_or_create_factory().unwrap();
            assert_eq!(reused.as_raw(), pointer);
        }
        assert_empty_cache();
    })
    .join()
    .unwrap();
}

#[test]
fn scopes_balance_com_references_in_existing_sta_and_mta_apartments() {
    for (mode, expected) in [
        (COINIT_APARTMENTTHREADED, APTTYPE_STA),
        (COINIT_MULTITHREADED, APTTYPE_MTA),
    ] {
        std::thread::spawn(move || {
            unsafe { CoInitializeEx(None, mode) }.ok().unwrap();
            let before = apartment();
            {
                let _scope = WicContext::new().unwrap();
                let factory = get_or_create_factory().unwrap();
                let kind = apartment().0;
                assert!(kind == expected || (expected == APTTYPE_STA && kind == APTTYPE_MAINSTA));
                drop(factory);
            }
            assert_empty_cache();
            assert_eq!(apartment(), before);
            unsafe { CoUninitialize() };
        })
        .join()
        .unwrap();
    }
}

#[test]
fn implicit_mta_callers_acquire_their_own_explicit_reference() {
    use windows::Win32::System::Com::{APTTYPEQUALIFIER_IMPLICIT_MTA, APTTYPEQUALIFIER_NONE};
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel::<()>();
    let anchor = std::thread::spawn(move || {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .unwrap();
        ready_tx.send(()).unwrap();
        let _ = finish_rx.recv();
        unsafe { CoUninitialize() };
    });
    ready_rx.recv().unwrap();
    let result = std::thread::spawn(|| {
        assert_eq!(apartment(), (APTTYPE_MTA, APTTYPEQUALIFIER_IMPLICIT_MTA));
        {
            let _scope = WicContext::new().unwrap();
            assert_eq!(apartment(), (APTTYPE_MTA, APTTYPEQUALIFIER_NONE));
            drop(get_or_create_factory().unwrap());
        }
        assert_empty_cache();
        assert_eq!(apartment(), (APTTYPE_MTA, APTTYPEQUALIFIER_IMPLICIT_MTA));
    })
    .join();
    drop(finish_tx);
    anchor.join().unwrap();
    result.unwrap();
}

#[test]
fn panic_releases_factory_and_scope_before_thread_exit() {
    std::thread::spawn(|| {
        let result = std::panic::catch_unwind(|| {
            let _scope = WicContext::new().unwrap();
            let _factory = get_or_create_factory().unwrap();
            panic!("injected decoding panic");
        });
        assert!(result.is_err());
        assert_empty_cache();
        let _scope = WicContext::new().unwrap();
        assert!(get_or_create_factory().is_some());
    })
    .join()
    .unwrap();
}

#[test]
fn standalone_viewer_decodes_leave_no_interfaces_for_tls_destruction() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("image.png");
    image::RgbaImage::from_pixel(8, 4, image::Rgba([10, 20, 30, 255]))
        .save(&path)
        .unwrap();
    // Repeat on independent threads: a successful test body is not sufficient
    // if its native COM cleanup crashes later in the thread's TLS callback.
    for _ in 0..8 {
        let path = path.clone();
        std::thread::spawn(move || {
            use crate::image_viewer::loader;
            let frame = loader::decode_full_frame(&path).unwrap();
            assert_eq!((frame.width, frame.height), (8, 4));
            assert_empty_cache();
            let frame = loader::decode_export_frame(&path).unwrap();
            assert_eq!((frame.width, frame.height), (8, 4));
            assert_empty_cache();
            let frame = super::super::extract_to_size_with_original_size(&path, Some(4)).unwrap();
            assert_eq!((frame.1, frame.2, frame.3, frame.4), (4, 2, 8, 4));
            assert_empty_cache();
        })
        .join()
        .unwrap();
    }
}

#[test]
fn decoder_failure_also_releases_factory_before_returning() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("image.png");
    std::fs::write(&path, b"invalid image").unwrap();
    std::thread::spawn(move || {
        assert!(super::super::extract(&path).is_none());
        assert_empty_cache();
    })
    .join()
    .unwrap();
}

#[test]
fn context_cannot_move_com_ownership_to_another_thread() {
    trait AmbiguousIfSend<A> {
        fn assert_not_send() {}
    }
    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    struct Invalid;
    impl<T: ?Sized + Send> AmbiguousIfSend<Invalid> for T {}
    let _ = <WicContext as AmbiguousIfSend<_>>::assert_not_send;
}
