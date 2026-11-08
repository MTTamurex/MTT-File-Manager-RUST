# Production video thumbnail integration

## Default build

The isolated libmpv backend is enabled in normal builds. The former experimental
feature is no longer required or defined:

```powershell
cargo build
.\target\debug\mtt-file-manager.exe
```

For a release test:

```powershell
cargo build --release
```

Use the application's normal runtime files alongside the matching executable,
including its existing `libmpv-2.dll`, `pdfium.dll`, and normal assets. This is
**not** the standalone POC executable. No new decoder DLL/executable is added to
the installer; the shared backend is statically compiled Rust code. Clean Windows
may need the Microsoft Visual C++ x64 runtime, as established during VM testing.

A startup log beginning `[MPV-THUMB] isolated video backend enabled` identifies
the primary extractor. The standalone POC remains a diagnostic tool, not the
application executable or a required runtime dependency.

## Behavior

- Videos use libmpv first, with Windows Shell then Media Foundation on failure.
  Image/document extraction is not replaced.
- The normal thumbnail/folder-preview database and temporary fallback filename
  are preserved. No new cache namespace, hash/version change, or extractor-driven
  invalidation is introduced. Valid cached images resolve before source I/O or
  either extractor. POC stores (`mpv-poc-v1`/`v2`/`v3`) are not read, migrated, or
  deleted. Existing Windows-generated frames are not replaced with covers merely
  because the extractor changed.
- Existing workers, source-read safety checks, result generations/epochs,
  resizing, GPU upload, and cache writing remain in use.
- One persistent child process handles video extractions serially, independently
  of the visible player. It reuses its own libmpv instance, stops each file after
  copying pixels, and runs below normal process priority with software decoding.
- The adaptive v2 policy disperses candidates in the middle half of the video,
  rejects nearly uniform black/white frames, and limits selection to three
  candidates. Native scaling reduces the frame before copying pixels. See
  [ADAPTIVE.md](ADAPTIVE.md) for the algorithm, edge cases, and measurements.
- MKVs prefer embedded JPEG attachments named exactly `cover.jpg` or `cover.jpeg`.
  A bounded probe uses the existing DLL's internal FFmpeg demuxer; unusable or
  absent covers return to normal adaptive frame selection. The player and other
  containers are unaffected. No external cover files are loaded.
- Only the current executable is launched, using the internal
  `--internal-mpv-thumbnail-helper` mode. It dispatches before preferences,
  diagnostics, migrations, and UI initialization. Do not launch this mode manually:
  its stdin/stdout are a private binary protocol, not a user-facing CLI.
- Requests carry a sequence number, target size, remaining time budget, and bounded UTF-8 absolute path.
  Responses must match that sequence and contain valid RGBA dimensions/length.
  Path frames are bounded at 128 KiB, and RGBA payloads at 512 × 512 × 4 bytes.
  No PNG or intermediate media files are involved.
- A parent-owned, non-inherited Job Object uses `KILL_ON_JOB_CLOSE`. Assignment
  must succeed before the first request/session initialization. It also lets the
  parent kill the helper tree on cancellation, timeout, or shutdown. This is
  **crash/hang isolation, not a security sandbox** for a compromised decoder.
- The parent supervises reads AND writes outside the blocking pipe-I/O thread.
  The 15-second request budget includes waiting for the single extractor slot.
  Navigation/deletion cancellation is checked approximately every 20 ms while
  waiting. A failed/stuck child is killed and reaped; that request is not retried
  internally. The next request can start a fresh child.
- IPC version 2 forwards the remaining parent deadline budget, accounting for
  queue wait and helper initialization; quality retries do not each get 15 seconds.
- Decode, crash/transport, active-request timeout, and runtime-unavailable errors
  allow a Windows fallback after rechecking source safety/cancellation. Only
  waiting for the shared slot to time out does not trigger Windows or kill the
  healthy helper. Unsafe source states remain deferred. If Windows also fails,
  supervisor failures retain transient backoff, while decode failures retain the
  per-path policy. Cancellation never marks the file broken or permits a late
  result to be persisted/uploaded.
- Windows providers retain their existing synchronous/blocking behavior; the
  15-second libmpv deadline does not hard-bound the Windows fallback. No automatic
  same-request libmpv restart/retry or recursive fallback loop is added.
  Shell/MF do not uniformly accept verbatim (`\\?\`) paths; successful fallback
  still depends on provider support for the supplied Win32 path and format.
- New supervisor logs contain categories/timings/dimensions, not source paths or
  media titles. Existing application diagnostics retain their existing behavior;
  sanitize reports before sharing them.

## Automated tests

Protocol bounds, crash recovery, blocked peer reads/writes, cancellation,
shutdown, process reuse, and actual process reaping (using a process handle):

```powershell
cargo test --lib infrastructure::mpv_thumbnails -- --test-threads=1
cargo test --lib infrastructure::disk_cache::compatibility_tests
cargo test --lib workers::thumbnail::extraction -- --test-threads=1
cargo test --lib cache_compatibility_tests
cargo test -p mtt-mpv-thumbnail
```

Fault injection uses contained PowerShell mock peers in tests. No crash/hang
switches are added to the production helper.

The following tests exercise the **real application executable** and its internal
helper, without opening the GUI. They are opt-in because they require the existing
runtime and a locally available fixture:

```powershell
$env:MTT_MPV_TEST_VIDEO = 'D:\Tests\clip.mp4'
cargo test --test mpv_thumbnail_process -- --ignored --test-threads=1
```

Repeat with H.264, HEVC, VP9, and AV1. For GUI-subsystem release verification,
add `--release` to that `cargo test` command. Checks include repeated capture,
exclusive file access while the helper is alive, invalid-input recovery, and
two independent helpers reading the same file concurrently. Concurrent helpers
are not a substitute for testing actual player playback below.

The synthetic real-helper cover test needs FFmpeg (fixture generation only) and
an existing scratch directory in `MTT_MPV_TEST_DIR`. It verifies both accepted
attachment names, red cover pixels over a blue video, repeated process/session
reuse, portrait sizing, and exclusive source access:

```powershell
cargo test --test mpv_thumbnail_cover -- --ignored --test-threads=1
```

The native routing test additionally verifies that a successful real helper skips
Windows and that injected decode/unavailable/transport/timeout failures reach real
Windows providers. It requires a synthetic H.264 MP4 supported by the local Windows
installation, the bundled runtime, and the built application (not the test runner):

```powershell
cargo build
$env:MTT_MPV_TEST_APP = (Resolve-Path '.\target\debug\mtt-file-manager.exe').Path
$env:MTT_MPV_TEST_VIDEO = 'D:\Tests\clip.mp4'
cargo test --lib real_helper_success_skips_windows_and_real_windows_recovers -- --ignored --test-threads=1
```

## Manual GUI acceptance checklist

Use generic dedicated test media and a fresh input directory;
already cached thumbnails do not exercise the helper again. Do not delete the
normal cache to force testing.

1. Open a folder of mixed-codec videos in the application. Confirm
   thumbnails appear and `[MPV-THUMB] extracted` messages identify source decoding.
2. Scroll quickly, change folders, switch visible panels, and return. Check that
   obsolete work does not display results in the wrong location or leave loading
   indicators permanently stuck.
3. Play a video in the integrated player while generating uncached thumbnails.
   Repeat with the same file, then with another file. Check audio, video, seek,
   playback controls, CPU use, and responsiveness.
4. After generation finishes, rename/move a dedicated test file; verify the
   thumbnail worker has not retained a handle. Avoid doing this to valuable media.
5. Include a synthetic corrupt video alongside valid ones. Confirm the next valid
   request works and errors do not trigger infinite retries.
6. Close the application during uncached extraction. In Task Manager confirm no
   process with the internal helper argument remains. The automated suite also
   verifies that closing the Job Object terminates a deliberately stuck peer.
7. Restart: confirm cached thumbnails are reused without another source decode.
8. Upgrade with an existing normal cache: confirm entries remain usable without
   extraction. Experimental entries should not appear in the production cache.

Remaining manual release acceptance/known limits: final GUI/Sandbox checks,
representative malformed/large media, HDR-to-SDR color handling, CPU/latency
tuning, and a native-decoder/process memory budget. The IPC byte limit is **not**
a process memory limit. Every executable still has its existing startup DLL
dependencies; errors before helper dispatch may surface as transport failures.

## Historical POC validation (before promotion)

- Normal `cargo check` passed; the two cache namespace/migration-path tests also
  passed with the experimental feature disabled.
- The feature-enabled thumbnail test selection passed all 90 tests, including
  supervisor fault injection and cache namespace checks.
- The three real-helper integration tests passed separately for H.264, HEVC,
  VP9, and AV1 (12 test executions). These used local synthetic fixtures and the
  existing DLL, in the debug profile, without launching the GUI.
- All seven shared-backend tests and three standalone CLI/metrics tests passed.
  The standalone smoke script also passed after the backend was shared.
- Formatting and patch whitespace checks passed.
- All-target Clippy passed with `-D warnings -A clippy::assertions_on_constants`.
  Without that single allowance, an existing constant assertion in
  `src/ui/disclosure.rs` fails the check; that unrelated file was not changed.

Actual UI playback/navigation, the integrated build in Sandbox, GUI-subsystem
release pipe verification, and HDR color accuracy remain manual acceptance work.

## Production promotion validation

- The complete library run passed 2,534 tests (11 ignored at that point).
- After review corrections, the focused thumbnail run passed 99 tests, plus
  two normal-cache compatibility tests. The added native routing test was run
  explicitly: real libmpv success skipped Windows, and injected decode/unavailable/
  transport/timeout failures recovered through real Windows providers using a
  synthetic H.264 MP4 with a regular Win32 path.
- All 26 backend tests (including synthetic media/cover tests), the real-helper
  JPEG cover test, and all 12 real-helper executions across H.264/HEVC/VP9/AV1
  passed without an experimental application feature.
- Default and no-online-updates all-target checks passed. Clippy uses the same
  existing unrelated `clippy::assertions_on_constants` allowance for the application;
  the backend needs no allowance.
- The release cover-test build did not finish within the 10-minute command limit.
  This is an incomplete release validation, not a reported compiler/test failure.
  Release helper/pipe and final GUI/Sandbox acceptance should be completed before
  publishing an installer. No installer or release was published by this promotion.
