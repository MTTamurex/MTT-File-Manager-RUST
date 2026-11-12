# MTT Copy Engine

Copy-only engine adapted from the Win32 callback and storage profiling approach
in [BetterCopy](https://github.com/articulite/BetterCopy), reviewed at commit
`a3e239bb6bbe60b79df93e6e04ad5e113c40934c`.

The application keeps its existing copy mechanism as the default. Enable the
optional MTT engine in **Settings > General > Copy engine**. Moves, deletion,
organizer jobs, archive extraction and virtual Shell items are not redirected.

## Initial supported scope

- Regular files and directory trees; no existing top-level destination targets.
- Conflicts are reported, never overwritten or merged. There is no automatic
  fallback after an attempted copy.
- Rejects symlinks/reparse points and recursive destinations during preflight.
- WinFsp local mounts that return Windows error 1005 while resolving DOS volume
  names use normalized volume-relative names instead. This compatibility path
  still checks ancestors for reparse points and checks directory identities to
  prevent recursive copying through volume aliases. Other resolution errors are
  not bypassed. This does not imply support for every virtual filesystem feature.
- Per-job progress, cooperative pause/resume and cancellation. Pause applies to
  preflight, queued files and in-flight Win32 progress callbacks. Workers sleep
  on a condition variable, and cancellation wakes paused workers. Pause/resume
  only works within the current application session, not after restarting.
  Completed files remain on cancellation;
  a temporary sibling file is used for each in-flight copy and removed on error.
- Windows `CopyFileExW` preserves file metadata supported by that API. Directory
  timestamps, ACL cloning, link preservation, cloud placeholders, persistent
  resume and elevation are not implemented. Live source trees should not be
  modified during copying; the engine does not provide filesystem snapshots.
- Storage profiling is cached per volume during a job and uses Windows-reported
  disk numbers, including the Windows system volume, rather than comparing drive
  letters. Different partitions on the same reported disk share its budget.
- HDD, remote or unknown storage uses one worker. Known SSD transfers involving
  the same disk or the Windows disk allow two total workers, but only **one large
  file** (>= 1 MiB). Independent non-system SSDs allow four total/two large files.
  The process-wide admission budget also applies across concurrent jobs. Paused
  transfers release admission slots and reacquire them before resuming I/O.
- Unsupported/aggregate/virtual storage may not expose its underlying physical
  topology; unknown identities use a shared conservative budget. These policies
  are safeguards, not evidence that a specific machine's freezing is fixed.

## Diagnosing stalls and comparing cache strategies

Enable the application's diagnostic mode **before** starting a controlled copy.
The opt-in background sampler records `copy_engine` policy/progress events every
two seconds independently of rendering. Records contain only numeric counters
and static labels: workers, active/waiting transfers, API-reported bytes, phase,
pause state and time without byte advancement. They do not include source or
destination paths, filenames, volume identifiers or disk serials. UI frame traces
also include copy activity and the `copy_dashboard` stage. API progress is not a
measurement of physical flush completion, and a no-progress interval alone does
not identify the controller/driver/cache/antivirus responsible for it.

On Windows, files **>= 256 MiB** use `COPY_FILE_NO_BUFFERING`; smaller files keep
buffered copying, following BetterCopy's size threshold. The `started` diagnostic
event labels this policy `unbuffered_from_256_mib`. Worker budgets and thread/I/O
priority are unchanged. Staging, verification and no-replace publication remain
in place; errors do not trigger an automatic buffered retry. This bypasses the
Windows copy cache, not the drive's internal cache, and does not guarantee better
performance or resolve stalls on every controller/workload.
There is an explicitly ignored 32 MiB integrity/smoke comparison:

```text
cargo test -p mtt-copy-engine --test cache_comparison -- --ignored --nocapture
```

This creates disposable files on the temporary volume and compares buffered
CopyFileEx, unbuffered CopyFileEx and the engine. It is **not run automatically**,
and its 32 MiB engine copy remains below the unbuffered threshold. An additional
manual integrity test exercises the production unbuffered path with a 256 MiB
file and a non-sector-aligned tail:

```text
cargo test -p mtt-copy-engine --test unbuffered_integrity -- --ignored --nocapture
```

These tests do not clear OS/device caches and are not comparisons against the app's Shell
or third-party handler. For meaningful performance validation, use the same
non-personal fixture, fresh destinations and alternating test order with the
current app mechanism and optional engine on the intended source/target disks.
Start with a small workload; do not repeat a load that freezes the machine.
Keep the Windows diagnostic log alongside elapsed times and disk response-time
observations. No performance gain is claimed without those measurements.

The application presents a movable, non-modal copy dashboard using its current
theme. Each operation has its own pause/resume/cancel controls, speed graph,
byte/file counters and estimated remaining time (excluding requested pause
intervals). Elapsed time is shown during the operation and retained as total time
after completion, cancellation or failure, excluding user-requested pauses and
including preparation and I/O waits. Completed, cancelled and failed operations remain until dismissed.
The dashboard lists only in-flight source files, including paused transfers:
three names are visible initially and additional names can be expanded. Full
paths appear on hover. Tracking is in memory and limited to active workers,
not the size of the directory tree or queued file list.

Minimize hides the dashboard without pausing or cancelling copies. A status-bar
entry shows operation counts and active progress; click it to restore the panel.
Finished/error results stay accessible there without reopening the panel
automatically. Existing status-bar controls retain their own reserved area.

The crate has no UI, clipboard manipulation, global hooks, move or delete API.
MIT attribution and full license are in `LICENSE` and in the repository's
`third_party_licenses/BetterCopy-MIT.txt`.
