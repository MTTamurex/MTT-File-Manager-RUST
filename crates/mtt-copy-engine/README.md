# MTT Copy Engine

Copy/move engine adapted from the Win32 callback and storage profiling approach
in [BetterCopy](https://github.com/articulite/BetterCopy), reviewed at commit
`a3e239bb6bbe60b79df93e6e04ad5e113c40934c`.

The application keeps its existing copy mechanism as the default. Enable the
optional MTT engine in **Settings > General > Copy, move and deletion engine**.
The same persisted option enables the application's deletion backend (outside
this crate). Manual filesystem moves (drag/drop and cut/paste) use the same
option. Organizer jobs, archive extraction and virtual Shell items are not redirected.

The shared operation window's minimize button keeps jobs running. Its close
button cancels all active jobs and closes the window immediately, without creating
a minimized/restore entry. Workers finish cancellation in the background without
undoing completed work; their state is released after completion. If no jobs are
active, closing dismisses the retained results.

## Optional move integration (Windows)

- `move_with_conflicts` uses a fresh `Progress`, the existing conflict UI and
  scheduling/admission budgets. Same-volume files are renamed by handle, without
  copying their data. Directory trees are planned and moved file by file, not
  atomically renamed as whole trees; empty source directories are removed last.
- Only Windows error 17 (`ERROR_NOT_SAME_DEVICE`) selects staged buffered copying
  followed by source removal. Other rename failures are reported without an
  automatic copy or Shell retry. Providers must support the required handle APIs.
- A source identity/size/timestamps snapshot is recorded before asking about its
  conflict. An identity-bound source handle denies writes/deletes throughout
  transfer. Source and destination ancestors are pinned against substitution.
  After copying, the staged destination identity is rechecked under a write/delete
  guard before source disposition. No source is deleted by a pathname lookup.
- Publication failure retains the source and existing target. If publication
  succeeds but source removal fails, both copies are kept and a retained-source
  result is shown. Cancellation stops remaining work, not already completed moves;
  a cancellation after publication can leave both copies. This is not a
  transactional snapshot, cryptographic verification, undo log, durable flush
  guarantee, or crash-recovery mechanism.
- Skipped files stay in the source tree. Cleanup uses nonrecursive directory
  disposition: nonempty, changed or inaccessible directories are retained. New
  arrivals are never recursively deleted. Same-folder and overlapping source
  selections are refused before destination writes. Reparse paths remain refused.
- Actual removed-source/destination pairs update views, tags and caches, including
  partial results. Cut/paste completion clears the clipboard only when the whole
  operation succeeds without skipped or retained items. Otherwise the pending
  paste is released while the cut clipboard is retained.
- Progress bytes include logical bytes completed by fast rename; its displayed
  rate is not physical disk throughput. The diagnostic transferred-byte counter
  counts only copies. Folder ACL/timestamp cloning and automatic elevation are
  not added by move support.
- Automated move safety tests use only owned synthetic temporary trees. The
  copy-then-remove branch is forced in tests on one temporary volume; real
  cross-volume/provider behavior still needs manual validation on disposable data.

## Optional deletion integration

- Delete and Shift+Delete use inline MTT confirmation controls, with Cancel
  initially focused. A worker waits for an answer without blocking the UI.
  Closing/cancelling the operation never implies consent, and stale answers
  cannot confirm a later request. Minimized progress is revealed once per new
  confirmation; the user can still minimize or collapse it afterward.
- Confirmation shows the question, selected names and action buttons.
  The list is virtualized, scrollable and limited to about five visible rows
  at the current font scale; full paths appear on hover. Selected directories
  are not expanded into descendants. Source/mode summaries and progress
  statistics appear only after confirmation, not beside the initial question.
- Delete uses `IFileOperation` recycling with a mandatory per-item
  `PreDeleteItem` guard. An item without `TSF_DELETE_RECYCLE_IF_POSSIBLE` is
  refused before deletion. Shell confirmation/error dialogs are suppressed;
  errors stop the operation, while system UAC prompts remain permitted.
- If recycling is unavailable, Shell is stopped before MTT asks separately
  whether to permanently delete the remaining contents of that selected root.
  Consent to recycle never authorizes this fallback. The fallback warning offers
  an unchecked "apply to all" checkbox: only an affirmative answer with that box
  checked permits subsequent unrecyclable items in this operation without more
  prompts. Without it, only the displayed root is authorized. Every item still
  attempts recycling first; recyclable items and already recycled items are not
  permanently deleted by this authorization. It is not saved as a preference or
  shared with other operations, and cancellation still stops remaining work.
- Shift+Delete requires explicit confirmation before direct filesystem deletion.
  Cancellation stops remaining work and never restores items already removed.
- Direct deletion walks the tree once, depth-first, without a separate total-size
  scan. Directory handles prevent ancestor replacement while traversing children.
  Selected reparse paths are refused before canonicalization; nested reparse
  entries are removed without traversing their targets. Read-only, inaccessible
  or locked entries remain and produce a retained error result; permissions and
  attributes are not silently changed.
- Actual removals are reported in batches of up to 128 entries, including
  children removed before a parent fails or cancellation stops the traversal.
  A surviving selected folder is never reported as deleted.
- Recycling uses Shell callbacks for cancellation and processed-item counts.
  Cancellation is cooperative: pending filesystem calls or system UAC dialogs must
  return before it takes effect. Counts reflect entries reported by Shell, which
  may recycle a whole directory as one item, not every descendant.
- Deletions share the themed copy dashboard, minimization and retained results.
  Each new copy or deletion centers the window once, including when an old window
  position is cached. Dragging and resizing remain available afterward.
  Cancelled results keep their status/message without a progress bar.
  Progress remains indeterminate while running: neither traversal discoveries
  nor Shell work units are advertised as a stable item total or byte throughput.
  Pause is not offered for deletion. Elapsed time freezes on completion.
- Deleting existing Recycle Bin entries and emptying the bin keep the original
  Shell mechanisms, preserving their metadata and confirmation behavior.
- Active filenames are sampled only for display, at most every 300 ms, for both
  copies and deletions. Counters and operations are not throttled.
- Automated deletion tests use owned synthetic temporary trees, simulated Shell
  results and actual progress-sink callbacks. Native unavailable/full/disabled
  Recycle Bin and UAC behavior still require an interactive Windows check.
  No deletion speed advantage is claimed.

## Initial supported scope

- Regular files and directory trees. The application calls `copy_with_conflicts`;
  the non-interactive `copy` API retains its strict refusal of existing targets
  for callers and diagnostics that do not provide a confirmation UI.
- App conflicts offer Replace/Skip/Keep both for files and Merge/Skip/Keep both
  for folders. Merge preserves unrelated contents and resolves conflicting files
  individually. Keep both generates an unused numbered sibling name. Selected
  folders skipped at preflight are not enumerated just to count descendants;
  the dashboard reports skipped conflict items separately from copied files.
- "Apply to all" is unchecked by default, operation-local and scoped to the
  same conflict kind: a folder merge never silently authorizes file replacement.
  `copy_with_conflicts` requires a fresh `Progress` for every call and rejects
  reuse, so previously approved answers cannot leak into a new operation.
  Cancel is initially focused, and the UI waits without blocking rendering or
  occupying disk-transfer permits. Decisions currently happen during preflight.
- Type mismatches (file versus folder), the source item itself and destinations
  already reserved by another queued source permit only Keep both/Skip/Cancel.
  No directory is deleted to make room for a file, and duplicate source names
  cannot race to overwrite one another.
- Replacement copies into an owned temporary sibling and checks the source and
  staged main-stream size before publication. The original destination remains
  in place until same-volume rename with replacement succeeds. A recorded file
  identity/size/timestamps is rechecked; Windows denies writes to that target
  while copying, then releases the handle immediately before rename. This is not
  a transactional snapshot or crash-recovery/backup service. Read-only or locked
  targets can fail without being silently unlocked or deleted first.
- Newly created or detectably changed targets after preflight fail safely rather
  than inheriting an old decision. There is no automatic retry through Shell.
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

On Windows, **all file sizes use buffered `CopyFileExW` copying**. There is no
256 MiB cache-policy threshold and no application-owned fixed-size copy buffer;
Windows manages the transfer buffers and system cache. The `started` diagnostic
event labels this policy `buffered`. Worker budgets and thread/I/O priority are
unchanged. Staging, verification and no-replace publication remain in place;
errors do not trigger an automatic retry. System caching does not eliminate the
drive's internal SLC-cache limits or guarantee better performance on every workload.
There is an explicitly ignored 32 MiB integrity/smoke comparison:

```text
cargo test -p mtt-copy-engine --test cache_comparison -- --ignored --nocapture
```

This creates disposable files on the temporary volume and compares buffered
CopyFileEx, unbuffered CopyFileEx and the engine. It is **not run automatically**,
and its engine copy uses the production buffered policy. An additional
manual integrity test exercises the production buffered path with a 256 MiB
file and a non-sector-aligned tail:

```text
cargo test -p mtt-copy-engine --test buffered_integrity -- --ignored --nocapture
```

These tests do not clear OS/device caches and are not comparisons against the app's Shell
or third-party handler. For meaningful performance validation, use the same
non-personal fixture, fresh destinations and alternating test order with the
current app mechanism and optional engine on the intended source/target disks.
Start with a small workload; do not repeat a load that freezes the machine.
Keep the Windows diagnostic log alongside elapsed times and disk response-time
observations. No performance gain is claimed without those measurements.

### Comparing the full engine on a selected file (Windows)

The following explicitly ignored test compares buffered and unbuffered copying
using the **same engine pipeline**, including preflight, worker admission,
callbacks, staging, size checks and no-replace publication. Only the Win32 copy
cache flag changes. The unbuffered mode exists only in the crate's unit-test
build; normal application copying remains buffered for every file size.

Choose an existing, non-personal test file on the mounted source volume and an
existing destination folder. This performs **two full copies**, temporarily
requiring roughly twice the file size in free destination space. Each sample
uses a fresh owned temporary folder; those folders and their copies are removed
afterward. The source is opened for reading, never deleted or overwritten.
Run no other copies concurrently. Example with generic fixture paths:

```powershell
$env:MTT_COPY_CACHE_SOURCE = 'X:\Test\source.bin'
$env:MTT_COPY_CACHE_DESTINATION = 'D:\Test'
$env:MTT_COPY_CACHE_ORDER = 'buffered-first'
cargo test -p mtt-copy-engine --lib compare_engine_cache_modes_on_selected_file -- --ignored --nocapture --test-threads=1
```

Repeat under comparable idle conditions with `MTT_COPY_CACHE_ORDER` set to
`unbuffered-first` to check order effects. Cache state is **not** reset. The test
prints only mode, elapsed milliseconds, byte count and numeric/static outcomes,
not paths or filenames. Byte-for-byte verification of the main data stream
happens after both measured copies so its reads do not prime the source between
samples. Alternate streams and ACLs are not compared by this benchmark; API
progress counters may include alternate-stream bytes beyond the main file size.
Owned temporary folders are closed explicitly so cleanup errors fail the test
instead of silently leaving copies behind. Timings include
the whole engine call but do not measure physical flush completion. The test
does not invoke Explorer/Shell and cannot establish by itself why they differ.
Do not change the application's cache policy based on a single warm-cache run.

The application presents a movable, non-modal copy dashboard using its current
theme. Its title shows byte-weighted progress, copied/total files and summed
speed for active operations, excluding retained terminal results. Once all
operations finish, the title summarizes the retained results with zero speed.
Preparing operations show unknown percentage rather than a misleading estimate.
Each operation keeps its own pause/resume/cancel controls, speed graph, byte
counters and estimated remaining time (excluding requested pause intervals).
Elapsed time is shown during the operation and retained as total time
after completion, cancellation or failure, excluding user-requested pauses and
including preparation and I/O waits. Completed, cancelled and failed operations remain until dismissed.
The dashboard lists only in-flight source files, including paused transfers:
up to three single-line name slots retain their dimensions even between files,
bounded by the operation's worker budget. The expansion row is reserved only
when additional workers are possible. Additional names can be expanded in a fixed-height scroll
area. Full paths appear on hover. The window remains manually resizable and
keeps a stable identity as its title changes. Tracking is in memory and limited to active workers,
not the size of the directory tree or queued file list.

Minimize hides the dashboard without pausing or cancelling copies. A status-bar
entry shows operation counts and active progress; click it to restore the panel.
Finished/error results stay accessible there without reopening the panel
automatically. Existing status-bar controls retain their own reserved area.

The crate has no UI, clipboard manipulation, global hooks, move or delete API.
MIT attribution and full license are in `LICENSE` and in the repository's
`third_party_licenses/BetterCopy-MIT.txt`.
