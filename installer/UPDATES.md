# Application updates

## Release requirements

- Publish a normal, non-draft GitHub release in the official repository. The
  application's beta status does not require marking releases as prereleases.
- Use a semantic version tag, optionally prefixed with `v`.
- Upload `MTT-File-Manager-Setup-<version>.exe`. The version must match the tag.
- GitHub must provide the asset's `sha256:` digest. Missing or malformed digests
  fail closed; users can still use the manual installation workflow.
- Build the complete workspace with `installer/build_installer.ps1`. Installers
  include `mtt-updater.exe`, which handles installation after MTT exits.

## Behavior and trust

Startup checking defaults to enabled. Startup installation defaults to disabled.
Both settings are stored in the existing preferences database. Only the latest
normal GitHub release is checked; older or equal versions are never installed.

Release information and installers are requested over HTTPS without credentials.
Downloads are bounded by the expected size and verified against GitHub's SHA-256
digest twice: after download and immediately before executing setup. This trusts
the official GitHub repository; a digest is not an independent publisher signature.
Redirects are limited to GitHub and GitHub's asset infrastructure. Notes are
rendered as Markdown with headings, lists, emphasis, and code. HTML is not
executed, image/file/network loaders are disabled, and note links can only open
HTTPS URLs without embedded credentials.

Installation waits for file operations to finish and other MTT processes (including
viewers) to exit. Enabled Organizer rules must be paused so their watcher cannot
enqueue a new move during shutdown. MTT must not be running as administrator.
The current session is flushed before authorizing the updater to proceed.

The copied, unelevated helper waits for MTT to exit, then invokes Inno Setup through
the Windows shell. Only setup is elevated. Automatic process closure and Windows
restart are disabled. The helper waits for setup and reopens MTT using its original
token, including after cancellation or failure. If setup cannot be monitored, MTT
is not reopened automatically: the user must wait for setup to finish and open it
manually. Startup checking is skipped once
on that restart to avoid an automatic installation loop. The existing installer
cache reset/reindex behavior is unchanged.

Before authorization, postponement and handoff failures stop and reap the waiting
helper before removing its download directory. Helper readiness has a bounded
timeout; an authorized helper waits for the parent to actually exit rather than
giving up during slow shutdown. Temporary downloads are removed when abandoned or
after installation. The copied
running helper can remain in its temporary directory because Windows locks its
executable until exit; ordinary temporary-directory cleanup can remove it later.

## Manual Windows validation

Use disposable test installations, not a working file-management session.

1. Install a previous version containing the updater; publish a newer test release
   with matching installer and digest. Confirm notes and manual update controls.
2. Confirm startup check is on and automatic installation is off by default;
   confirm both preferences survive restart.
3. Try updating during copy, move, extraction, compression, image conversion, and
   batch rename. No operation may be cancelled to update.
4. Open another main window and a viewer. Confirm installation waits for both to
   close. Confirm enabled Organizer rules block until paused.
5. Enable minimize-to-tray. Updating must exit, not hide the application.
6. Cancel UAC and cancel setup. Confirm the old app reopens unelevated without an
   automatic retry loop. Confirm successful setup also reopens unelevated.
7. Confirm the search service restarts, the index rebuild notice appears, and user
   preferences remain intact after upgrading.
8. Disconnect the network or alter an installer before handoff. No incomplete or
   altered installer may run. Repeat with missing asset/digest and API rate limits.
9. Try updating an elevated MTT instance. It must request an unelevated restart.
10. Postpone while the helper is preparing, or fail final session persistence.
    Confirm the waiting helper exits, no setup runs, and another attempt succeeds.

The updater cannot bootstrap versions released before this feature: upgrade those
once using the full installer manually.

## Uninstall after a VM downgrade test

Inno Setup accumulates uninstall actions across installations. A test build with
the maintenance flag followed by an older official executable can otherwise
leave an action that starts the older application's UI during uninstall.

The folder-restoration action now invokes a guarded script with the SHA-256 of
the executable packaged by that installer. It runs the maintenance flag only if
the installed executable still matches, keeping it protected against replacement
until the maintenance process exits. On mismatch it skips that call; the native
installer restoration still applies. The existing RunOnceId is preserved so the
guarded action supersedes earlier, unguarded entries.

An already-installed test build with an unguarded uninstall action needs to be
reinstalled using a corrected installer once to replace that logged action.
