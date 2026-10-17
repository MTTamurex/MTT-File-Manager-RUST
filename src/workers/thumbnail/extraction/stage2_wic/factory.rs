use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, IWICImagingFactory};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
};

struct FactoryCache {
    scopes: usize,
    factory: Option<IWICImagingFactory>,
}

thread_local! {
    // All interfaces are removed by the final stack scope, before thread exit.
    static CACHE: RefCell<FactoryCache> = const {
        RefCell::new(FactoryCache { scopes: 0, factory: None })
    };
}

/// Keep this on the stack of a decoding worker to reuse its WIC factory.
/// COM must not be initialized/uninitialized by a Windows TLS destructor.
pub(crate) struct WicContext {
    _same_thread: PhantomData<Rc<()>>,
}

impl WicContext {
    pub(crate) fn new() -> Option<Self> {
        let mut result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result == RPC_E_CHANGED_MODE {
            // Respect an existing STA and acquire our own balanced reference.
            result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        }
        if result.is_err() {
            return None;
        }
        CACHE.with(|cache| cache.borrow_mut().scopes += 1);
        Some(Self {
            _same_thread: PhantomData,
        })
    }
}

impl Drop for WicContext {
    fn drop(&mut self) {
        let factory = CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            cache.scopes -= 1;
            if cache.scopes == 0 {
                cache.factory.take()
            } else {
                None
            }
        });
        // Release while COM is still initialized, outside the RefCell borrow.
        drop(factory);
        unsafe { CoUninitialize() };
    }
}

pub(super) fn get_or_create_factory() -> Option<IWICImagingFactory> {
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.scopes == 0 {
            return None;
        }
        if let Some(factory) = cache.factory.as_ref() {
            return Some(factory.clone());
        }
        let factory: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()? };
        cache.factory = Some(factory.clone());
        Some(factory)
    })
}

#[cfg(test)]
#[path = "factory_tests.rs"]
mod tests;
