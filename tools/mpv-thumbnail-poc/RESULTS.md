# POC results

These are the original v1 measurements. The current adaptive v2 selection,
quality checks, cache namespace, and A/B measurements are in [ADAPTIVE.md](ADAPTIVE.md).

## Outcome

**Confirmed locally:** the DLL already staged by the project generates video
thumbnails in memory using `screenshot-raw video`, `vo=null`, and software
decoding. No new decoder runtime, image temporary files, or installer changes
were needed for the standalone experiment. Subsequent opt-in application
integration is documented separately in [INTEGRATION.md](INTEGRATION.md).

Tested runtime:

- mpv: `v0.41.0-514-g06f4ce75a`.
- FFmpeg inside libmpv: `N-124033-gd538a71ad`.
- Client API: 2.5.
- SHA-256: `AE7B7B6B3ECCC7AEEFF3CBEF30FD175F97B27F1B815F89D0BD0CCA926C8C2D99`.
  This matches the libmpv hash pinned by `installer/build_installer.ps1`.

The host is an existing Windows development environment, **not a clean VM**.
The tool itself calls neither Windows media APIs nor an FFmpeg executable.
An external FFmpeg was used only to create synthetic input fixtures beforehand.

## Functional checks

| Check | Result |
| --- | --- |
| H.264 / MP4, HEVC / MKV, VP9 / WebM, AV1 / MKV | Passed |
| Vertical video and 90-degree display rotation | Passed, portrait thumbnail |
| 4K and very short video | Passed |
| PQ/BT.2020-tagged, 10-bit HEVC | Decoded; color fidelity not certified |
| Spaces and non-ASCII input path | Passed |
| Corrupt, truncated, audio-only input | Failed cleanly, as expected |
| Reuse after invalid input | Next valid input captured successfully |
| Seek near end / beyond duration | Captured / rejected, respectively |
| Different timestamps | Different thumbnail PNG hashes |
| Exclusive read after each capture, with instance still alive | Passed |
| Refuse existing PNG output | Passed |

Images from normal, rotated, and synthetic HDR fixtures were inspected. The
rotation was applied; synthetic HDR success does not establish HDR-to-SDR
correctness. All smoke-suite success/failure counts matched expectations.

## Repeated extraction

Release executable, sequential local synthetic 640x360 clips, maximum thumbnail
side 256, no PNG writes. Both runs used the same four inputs and 250 repetitions,
with exclusive-read checks after every capture. Runs were sequential, not
competing against each other. Results are machine-specific warm-cache samples.

| Instance policy | Captures | Failures | Median | p95 |
| --- | ---: | ---: | ---: | ---: |
| Reused dedicated instance | 1,000 | 0 | 29.61 ms | 40.89 ms |
| New instance per input | 1,000 | 0 | 42.51 ms | 56.43 ms |

The reused instance's startup/final destruction is outside its per-item timing;
fresh-instance timings include initialization and destruction. Stop/file release
is included in both. The separate mixed-format smoke batch also exercises 4K,
rotation, HDR tagging, and short/vertical clips.

No steadily accumulating handles were observed: samples in the final 500
iterations were 181–182 for reuse and 180 for fresh instances. Commit charge
fluctuated with native allocator retention; iteration 500 versus 1,000 was
approximately 54.1 versus 53.1 MiB for reuse, and 50.1 versus 50.0 MiB for fresh
instances. These observations are not a substitute for native leak profiling.

## Problems found and corrected during review

1. **Stop/END_FILE race:** asynchronous stop reply arrived before actual file
   teardown. A queue drain was insufficient and caused alternate reused captures
   to mistake the previous file's END_FILE for a new failure. Stop now waits for
   both events before resetting state.
2. **Input retained by reused instance:** stop is now performed after copying the
   frame, and exclusive-read tests verify release before reusing the instance.
3. **Rotation fixture was ineffective:** older metadata assignment did not write
   display rotation with the installed FFmpeg. The test now explicitly sets input
   display rotation and asserts portrait output dimensions.
4. **Output safety:** PNG uses create-new semantics and explicit buffered flush;
   existing output is never silently overwritten.
5. **FFI lifetime/layout:** command-result pixels are copied while event data is
   valid; property nodes are freed separately; API-major and x64 layout checks
   cover the hand-written ABI subset.

## Acceptance still pending

- Repeat clean-Windows coverage for the application integration (standalone
  Sandbox results supplied by the user are summarized below).
- Test concurrent playback in the actual file manager, including the same file.
- Test representative real-world media, storage devices, malformed inputs, and
  HDR tone mapping; synthetic fixtures are not full compatibility coverage.
- Deliberately exercise a blocked native call and verify watchdog behavior.
- Decide production crash/hang isolation and memory budgets before integration.

**Recommendation from the standalone experiment:** the reuse approach is viable
enough for a controlled integration experiment, but not approval to replace the
default production pipeline. The integration is opt-in; see INTEGRATION.md.

## User-reported Windows Sandbox checks

The user supplied successful standalone runs after installing the Visual C++
x64 runtime (not a codec pack). These are reported results, not an independently
reproduced environment audit:

| Input codec | Captures | Failures | Median | p95 |
| --- | ---: | ---: | ---: | ---: |
| H.264, 1920 × 1080 | 10 | 0 | 179.26 ms | 185.60 ms |
| HEVC, 1920 × 816 | 10 | 0 | 361.49 ms | 415.05 ms |
| AV1, 2560 × 1350 | 10 | 0 | 222.57 ms | 340.37 ms |

A separate mixed two-file run completed 200 captures without failure and included
VP9. HEVC/AV1 runs explicitly confirmed source-file release on every capture.
The initial alternating-failure log lacked `stop_ms` and was from an older
executable; rerunning with the corrected binary eliminated those failures.
Different sources, resolutions, and seeks make these unsuitable for comparing
codec performance. No personal paths, filenames, or media titles are retained here.
