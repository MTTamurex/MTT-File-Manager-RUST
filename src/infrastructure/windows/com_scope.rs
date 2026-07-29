use std::marker::PhantomData;
use std::rc::Rc;

use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

/// Must be created and dropped on the same thread. The `Rc` marker keeps it `!Send`.
#[derive(Debug)]
pub struct ComScope {
    initialized: bool,
    _not_send: PhantomData<Rc<()>>,
}

impl ComScope {
    pub fn sta() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self {
            initialized: hr.is_ok(),
            _not_send: PhantomData,
        }
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
}

impl Drop for ComScope {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                CoUninitialize();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ComScope;

    #[test]
    fn sta_scope_constructs_and_drops() {
        let scope = ComScope::sta();
        let _ = scope.is_initialized();
    }

    #[test]
    fn sta_scope_is_not_send() {
        trait AmbiguousIfSend<A> {
            fn assert_not_send() {}
        }
        impl<T: ?Sized> AmbiguousIfSend<()> for T {}
        struct Invalid;
        impl<T: ?Sized + Send> AmbiguousIfSend<Invalid> for T {}

        let _ = <ComScope as AmbiguousIfSend<_>>::assert_not_send;
    }
}
