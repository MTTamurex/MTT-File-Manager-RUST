//! Keep installer builds independent of the retired signed MSIX integration.

#[test]
fn workspace_and_installer_do_not_build_the_explorer_extension() {
    let workspace = include_str!("../Cargo.toml");
    let installer = include_str!("../installer/build_installer.ps1");
    assert!(!workspace.contains("mtt-explorer-command"));
    assert!(!installer.contains("mtt_explorer_command.dll"));
    assert!(!installer.contains("build_sparse_package.ps1"));
    assert!(!installer.contains("unregister_sparse_package.ps1"));
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
    let unregister_references: Vec<_> = setup
        .lines()
        .filter(|line| line.contains("unregister_sparse_package.ps1"))
        .collect();
    assert_eq!(
        unregister_references,
        ["Type: files; Name: \"{app}\\unregister_sparse_package.ps1\""]
    );
    assert!(setup.contains("\\cleanup_context_menu.ps1"));
    assert!(setup.contains("\\restore_folder_integration.ps1"));
}

#[test]
fn obsolete_uninstall_command_is_superseded_without_running_a_removed_script() {
    let setup = include_str!("../installer/setup.iss");
    let overrides: Vec<_> = setup
        .lines()
        .filter(|line| line.contains("RunOnceId: \"UnregisterMttSparsePackage\""))
        .collect();
    assert_eq!(
        overrides,
        ["Filename: \"{sys}\\cmd.exe\"; Parameters: \"/d /c exit /b 0\"; RunOnceId: \"UnregisterMttSparsePackage\"; Flags: runhidden waituntilterminated"]
    );
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
