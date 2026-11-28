use super::*;

#[test]
fn child_command_carries_backend_and_gpu_preference_overrides() {
    for backend in [
        GraphicsBackend::Dx12,
        GraphicsBackend::Vulkan,
        GraphicsBackend::Glow,
    ] {
        let before = std::env::var_os(BACKEND_ENV);
        let before_gpu = std::env::var_os(GPU_PREFERENCE_ENV);
        let command = command_with_backend(
            Path::new("app.exe"),
            backend,
            crate::gpu_backend::GpuPreference::LowPower,
        );
        let overrides: Vec<_> = command.get_envs().collect();
        assert!(overrides.contains(&(
            std::ffi::OsStr::new(BACKEND_ENV),
            Some(std::ffi::OsStr::new(backend.key()))
        )));
        assert!(overrides.contains(&(
            std::ffi::OsStr::new(GPU_PREFERENCE_ENV),
            Some(std::ffi::OsStr::new("low_power"))
        )));
        assert_eq!(std::env::var_os(BACKEND_ENV), before);
        assert_eq!(std::env::var_os(GPU_PREFERENCE_ENV), before_gpu);
        assert_eq!(command.get_program(), "app.exe");
    }
}

#[test]
fn active_backend_wins_over_inheritance_and_saved_preference() {
    for backend in [
        GraphicsBackend::Dx12,
        GraphicsBackend::Vulkan,
        GraphicsBackend::Glow,
    ] {
        let selection = select(
            Some(backend),
            Some("invalid"),
            Some("glow"),
            Some(crate::gpu_backend::GpuPreference::LowPower),
            Some("automatic"),
            Some("high_performance"),
        )
        .unwrap();
        assert_eq!(selection.candidates, vec![backend]);
        assert!(selection.inherited);
        assert_eq!(
            selection.gpu_preference,
            crate::gpu_backend::GpuPreference::LowPower
        );
    }
}

#[test]
fn inherited_backend_never_has_an_independent_fallback() {
    for backend in [
        GraphicsBackend::Dx12,
        GraphicsBackend::Vulkan,
        GraphicsBackend::Glow,
    ] {
        let selection = select(
            None,
            Some(backend.key()),
            Some("vulkan"),
            None,
            Some("low_power"),
            Some("high_performance"),
        )
        .unwrap();
        assert_eq!(selection.candidates, vec![backend]);
        assert!(selection.inherited);
        assert_eq!(
            selection.gpu_preference,
            crate::gpu_backend::GpuPreference::LowPower
        );
    }
    assert!(select(None, Some("invalid"), Some("auto"), None, None, None).is_err());
}

#[test]
fn direct_launch_uses_the_managers_fallback_order_and_legacy_preferences() {
    use GraphicsBackend::*;
    for preference in [None, Some("auto"), Some("dx12"), Some("unknown")] {
        let selection = select(None, None, preference, None, None, None).unwrap();
        assert_eq!(selection.candidates, vec![Dx12, Vulkan, Glow]);
        assert!(!selection.inherited);
    }
    assert_eq!(
        select(None, None, Some("vulkan"), None, None, None)
            .unwrap()
            .candidates,
        vec![Vulkan, Glow]
    );
    for preference in ["glow", "gl"] {
        assert_eq!(
            select(None, None, Some(preference), None, None, None)
                .unwrap()
                .candidates,
            vec![Glow]
        );
    }
}

#[test]
fn independent_viewer_uses_saved_gpu_preference() {
    for preference in [
        crate::gpu_backend::GpuPreference::HighPerformance,
        crate::gpu_backend::GpuPreference::LowPower,
        crate::gpu_backend::GpuPreference::Automatic,
    ] {
        let selection =
            select(None, None, Some("auto"), None, None, Some(preference.key())).unwrap();
        assert_eq!(selection.gpu_preference, preference);
    }
}

#[test]
fn vulkan_first_keeps_the_other_fallbacks_in_order() {
    use GraphicsBackend::*;
    assert_eq!(
        vulkan_first(vec![Dx12, Vulkan, Glow]),
        vec![Vulkan, Dx12, Glow]
    );
    assert_eq!(vulkan_first(vec![Glow]), vec![Glow]);
}
