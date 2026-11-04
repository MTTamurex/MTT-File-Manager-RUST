//! Keep installer builds independent of the retired signed MSIX integration.

#[test]
fn workspace_and_installer_do_not_build_the_explorer_extension() {
    let workspace = include_str!("../Cargo.toml");
    let installer = include_str!("../installer/build_installer.ps1");
    assert!(!workspace.contains("mtt-explorer-command"));
    assert!(!installer.contains("mtt_explorer_command.dll"));
    assert!(!installer.contains("build_sparse_package.ps1"));
    assert!(!installer.contains("MTT_MSIX_"));
    assert!(!installer.contains("ModernMenuPackageEnabled"));
    assert!(installer.contains("'mtt-file-manager.exe', 'mtt-search-service.exe'"));
}

#[test]
fn installer_preserves_legacy_cleanup_without_registering_a_new_package() {
    let setup = include_str!("../installer/setup.iss");
    assert!(!setup.contains("ModernMenuPackageEnabled"));
    assert!(!setup.contains("\\register_sparse_package.ps1"));
    assert!(!setup.contains("Source: \"{#BuildOutputDir}\\mtt_explorer_command.dll\""));
    assert!(setup.contains("Type: files; Name: \"{app}\\mtt_explorer_command.dll\""));
    assert!(setup.contains("Type: files; Name: \"{app}\\mtt-file-manager.identity.msix\""));
    assert!(setup.contains("\\unregister_sparse_package.ps1"));
    assert!(setup.contains("\\cleanup_context_menu.ps1"));
    assert!(setup.contains("\\restore_folder_integration.ps1"));
}

#[test]
fn native_manifest_retains_display_settings_without_msix_identity() {
    let manifest = include_str!("../app.manifest");
    let build = include_str!("../build.rs");
    assert!(!manifest.contains("<msix "));
    assert!(!build.contains("MTT_MSIX_"));
    assert!(manifest.contains("PerMonitorV2,PerMonitor"));
    assert!(manifest.contains("<longPathAware"));
    assert!(manifest.contains("Microsoft.Windows.Common-Controls"));
    assert!(manifest.contains("<supportedOS"));
}
