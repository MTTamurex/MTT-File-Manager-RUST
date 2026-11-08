# Experimental application integration

## Enable explicitly

The default build is unchanged. Enable the experiment only when compiling:

```powershell
cargo build --features mpv-thumbnails-poc
.\target\debug\mtt-file-manager.exe
```

For a release test:

```powershell
cargo build --release --features mpv-thumbnails-poc
```

Use the application's normal runtime files alongside the matching executable,
including its existing `libmpv-2.dll`, `pdfium.dll`, and normal assets. This is
**not** the standalone POC executable. No new decoder DLL/executable is added to
the installer; the shared backend is statically compiled Rust code. Clean Windows
may need the Microsoft Visual C++ x64 runtime, as established during VM testing.

A startup log beginning `[MPV-THUMB] experimental isolated video backend enabled`
identifies the experimental build. It is a compile-time feature, not a preference
or environment switch. Rebuild without the feature to return to normal behavior.

## Behavior

- Videos use libmpv exclusively, with no Windows Shell/Media Foundation fallback.
  Image/document extraction is not replaced.
- All persistent thumbnail/folder-preview cache accesses in this build use a
  nested `mpv-poc-v3` directory. The temporary fallback database also has a
  different filename. Normal cache contents are neither read nor cleared.
  The prior `mpv-poc-v1` and `mpv-poc-v2` caches are also preserved; thumbnails are
  regenerated rather than reusing v2 video frames over embedded covers. App preferences, sessions, and
  other state are **not** isolated.
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
- Crash/transport/timeout/unavailable failures use the existing worker transient
  backoff; decode failures use the existing per-path failure policy. Unsafe source
  states remain deferred. Cancelled requests do not mark a file as broken and
  late results are discarded before they are persisted/uploaded.
- New supervisor logs contain categories/timings/dimensions, not source paths or
  media titles. Existing application diagnostics retain their existing behavior;
  sanitize reports before sharing them.

## Automated tests

Protocol bounds, crash recovery, blocked peer reads/writes, cancellation,
shutdown, process reuse, and actual process reaping (using a process handle):

```powershell
cargo test --features mpv-thumbnails-poc --lib infrastructure::mpv_thumbnails -- --test-threads=1
cargo test --features mpv-thumbnails-poc --lib infrastructure::disk_cache::namespace
cargo test -p mtt-mpv-thumbnail
```

Fault injection uses contained PowerShell mock peers in tests. No crash/hang
switches are added to the production helper.

The following tests exercise the **real application executable** and its internal
helper, without opening the GUI. They are opt-in because they require the existing
runtime and a locally available fixture:

```powershell
$env:MTT_MPV_TEST_VIDEO = 'D:\Tests\clip.mp4'
cargo test --features mpv-thumbnails-poc --test mpv_thumbnail_process -- --ignored --test-threads=1
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
cargo test --features mpv-thumbnails-poc --test mpv_thumbnail_cover -- --ignored --test-threads=1
```

## Manual GUI acceptance checklist

Use generic dedicated test media and a fresh cache namespace/input directory;
already cached thumbnails do not exercise the helper again. Do not delete the
normal cache to force testing.

1. Open a folder of mixed-codec videos in the experimental application. Confirm
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
8. Run the normal build: confirm its pre-existing cache still works independently.

Still pending before default enablement: manual GUI/Sandbox acceptance,
representative malformed/large media, HDR-to-SDR color handling, CPU/latency
tuning, and a native-decoder/process memory budget. The IPC byte limit is **not**
a process memory limit. Every executable still has its existing startup DLL
dependencies; errors before helper dispatch may surface as transport failures.

## Validation performed during implementation

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
