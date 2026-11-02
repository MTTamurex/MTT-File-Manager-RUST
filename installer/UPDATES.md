# Browser-based application updates (experiment)

This branch keeps update checks and release notes but removes installer downloads,
automatic installation, and the separate update executable. The original flow is
preserved on `v0_2_10`.

## Release requirements

- Publish a normal, non-draft GitHub release in the official repository.
- Use a semantic version tag, optionally prefixed with `v`.
- The application checks only the latest normal release and notifies only when
  its version is newer. It does not select, download, or execute release assets.
- **Download update** opens the official release page in the default browser;
  the user chooses an installer and runs it manually.
- See [variant build instructions](VARIANTS.md). The experiment produces
  `MTT-File-Manager-Setup-<version>-browser-update.exe` without overwriting the
  original standard installer. Existing integrated-updater clients still require
  their original asset naming; this experimental installer is for manual testing.

## Behavior and trust

Startup checking defaults to enabled and can be disabled in **Settings > About**.
The existing startup-check preference is retained. The old automatic-install
preference is not read or acted upon, and no preference data is deleted.

Only release metadata is requested over HTTPS without credentials. The release
page URL is constructed from the official repository and a validated version tag,
not taken from arbitrary response URLs. Notes retain the existing Markdown
renderer, with HTML execution and automatic image/network loading disabled.

Opening the release page does not close MTT, stop file operations, request
elevation, start an installer, or reopen the application. Installation is manual.
No updater executable is built or bundled; any old helper from a previous
installation is left untouched and is not invoked.

## Manual Windows validation

1. Run the standard edition and confirm **Settings > About** shows startup checks
   and manual checks, without an automatic-install toggle.
2. Check against a newer normal release. Confirm the notes and version appear;
   no browser opens automatically.
3. Click **Download update**. Confirm the browser opens the matching official
   release page and MTT remains running.
4. Click **Later**. Confirm no installer or helper starts and release details can
   be reopened.
5. Confirm startup-check preferences persist across restarts. A previously saved
   automatic-install setting must not cause a download or installation.
6. Confirm network errors leave the UI responsive and permit a later retry.
7. Test the no-update edition and confirm all update controls are absent.

Use `installer/tests/test_update_variants.ps1` to validate packaging without
installing MTT. Scan the exact final experimental installer and record its hash;
this change does not guarantee a particular antivirus result.

## Uninstall after a VM downgrade test

Inno Setup accumulates uninstall actions across installations. The existing
folder-restoration action uses a guarded script with the SHA-256 of the packaged
application. It runs maintenance only if the installed executable still matches;
the native installer restoration remains available on mismatch. This protection
and its existing RunOnceId are unchanged by the browser-update experiment.
