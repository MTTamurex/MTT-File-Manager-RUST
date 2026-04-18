use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

/// Must be created and dropped on the same thread.
#[derive(Debug)]
pub struct ComScope {
    initialized: bool,
}

impl ComScope {
    pub fn sta() -> Self {
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Self {
            initialized: hr.is_ok(),
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
}