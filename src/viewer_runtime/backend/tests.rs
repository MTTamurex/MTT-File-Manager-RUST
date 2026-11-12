use super::*;

#[test]
fn child_command_carries_only_the_selected_backend_override() {
    for backend in [
        GraphicsBackend::Dx12,
        GraphicsBackend::Vulkan,
        GraphicsBackend::Glow,
    ] {
        let before = std::env::var_os(BACKEND_ENV);
        let command = command_with_backend(Path::new("app.exe"), backend);
        let overrides: Vec<_> = command.get_envs().collect();
        assert_eq!(
            overrides,
            vec![(
                std::ffi::OsStr::new(BACKEND_ENV),
                Some(std::ffi::OsStr::new(backend.key()))
            )]
        );
        assert_eq!(std::env::var_os(BACKEND_ENV), before);
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
        let selection = select(Some(backend), Some("invalid"), Some("glow")).unwrap();
        assert_eq!(selection.candidates, vec![backend]);
        assert!(selection.inherited);
    }
}

#[test]
fn inherited_backend_never_has_an_independent_fallback() {
    for backend in [
        GraphicsBackend::Dx12,
        GraphicsBackend::Vulkan,
        GraphicsBackend::Glow,
    ] {
        let selection = select(None, Some(backend.key()), Some("vulkan")).unwrap();
        assert_eq!(selection.candidates, vec![backend]);
        assert!(selection.inherited);
    }
    assert!(select(None, Some("invalid"), Some("auto")).is_err());
}

#[test]
fn direct_launch_uses_the_managers_fallback_order_and_legacy_preferences() {
    use GraphicsBackend::*;
    for preference in [None, Some("auto"), Some("dx12"), Some("unknown")] {
        let selection = select(None, None, preference).unwrap();
        assert_eq!(selection.candidates, vec![Dx12, Vulkan, Glow]);
        assert!(!selection.inherited);
    }
    assert_eq!(
        select(None, None, Some("vulkan")).unwrap().candidates,
        vec![Vulkan, Glow]
    );
    for preference in ["glow", "gl"] {
        assert_eq!(
            select(None, None, Some(preference)).unwrap().candidates,
            vec![Glow]
        );
    }
}
