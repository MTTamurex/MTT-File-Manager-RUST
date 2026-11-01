# Installer editions

Maintain both editions on the same branch and release tag. Do not maintain a
separate copy of the application or a permanent branch for each edition.

## Cargo feature

`online-updates` is enabled by default. It controls the update state, network
client, release-note renderer, update UI, and the `mtt-updater` binary. Its
exclusive dependencies (`reqwest`, `semver`, `egui_commonmark`) are optional.
The helper declares `required-features = ["online-updates"]`, so Cargo does not
build it when that feature is disabled.

```powershell
# Standard edition (default features).
cargo build --release --workspace --locked

# No-update edition; keep filesystem watching enabled.
cargo build --release --workspace --locked --no-default-features --features notify-watcher
```

Use the installer script below for release packaging, rather than packaging
arbitrary binaries from the shared `target/release` directory.

## Build installers

Run from the repository root with the usual Rust, native-library, Inno Setup,
and runtime DLL prerequisites available:

```powershell
# Standard edition only (also the default when -Variant is omitted).
.\installer\build_installer.ps1 -Variant Standard

# No-update edition only.
.\installer\build_installer.ps1 -Variant NoUpdate

# Both installers, sequentially, from this source tree.
.\installer\build_installer.ps1 -Variant Both
```

The script uses separate Cargo target directories:

- Standard: `target/installer-standard/release`
- No-update: `target/installer-no-update/release`

Each directory receives verified runtime DLLs staged from `target/release` if
they are not already present. The existing DLL hash checks remain mandatory.
MSIX signing configuration and the classic context-menu fallback are unchanged.

The script passes the selected build directory to Inno Setup and excludes the
helper from the no-update package, even if an unrelated helper exists elsewhere.
`-SkipBuild` accepts only a prior script-produced build of the same edition whose
recorded executable/DLL hashes still match. Run without `-SkipBuild` after source
changes; it intentionally reuses existing binaries rather than recompiling them.

Outputs in `installer/output`:

- `MTT-File-Manager-Setup-<version>.exe`
- `MTT-File-Manager-Setup-<version>-no-update.exe`

Both installers use the same application identity and installation location.
Only one edition is installed at a time. Existing update preferences are left
untouched when switching; the no-update edition neither reads nor acts on them.
An old helper file from an earlier installation may remain on disk, but the
no-update application has no update code to invoke it.

## Validation and release hashes

```powershell
cargo check --workspace --all-targets --locked
cargo check --workspace --all-targets --locked --no-default-features --features notify-watcher
.\installer\tests\test_update_variants.ps1
```

The installer test uses temporary stand-in files and disables installer output;
it never installs MTT or changes Windows integration.

Rebuilding changes the artifact hashes. Recompute SHA-256 values and scan the
exact final installers before referring to their results in release notes.
`SHA256SUMS-0.2.9.txt` identifies the already-prepared v0.2.9 files only, not future
rebuilds. Renaming an existing installer does not change its hash.
