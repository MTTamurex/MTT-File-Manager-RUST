# libmpv thumbnail POC

The shared backend is now the application's default video thumbnail extractor.
This standalone executable remains a diagnostic/A-B tool: it has no application
cache or Windows fallback. See [INTEGRATION.md](INTEGRATION.md) for production use.

Standalone Windows development tool. Running this tool does **not** modify the
application's thumbnail pipeline, cache, player, or installer. The separate
opt-in application integration is described in [INTEGRATION.md](INTEGRATION.md).

## What it proves

The existing `libmpv-2.dll` can decode a local video and return a thumbnail in
memory, without Windows Shell, Media Foundation, codec packs, `ffmpeg.exe`, a
visible window, audio, or temporary image files. PNG output is opt-in.

The tool loads the explicitly selected DLL with a restricted Windows dependency
search. Use a trusted DLL with client API major 2 and the same architecture as the
tool. There is no DLL download, PATH search, or additional runtime to install.

## Build and run

From the repository root, using the project's Rust toolchain:

```powershell
cargo build --release --locked --manifest-path tools/mpv-thumbnail-poc/Cargo.toml

$poc = '.\tools\mpv-thumbnail-poc\target\release\mpv-thumbnail-poc.exe'
$dll = '.\target\debug\libmpv-2.dll'

& $poc --dll $dll --inspect
& $poc --dll $dll --at 1 --size 256 'D:\Videos\clip.mp4'
& $poc --dll $dll --at 1 --output 'D:\Videos\thumbnail.png' 'D:\Videos\clip.mp4'
& $poc --dll $dll --reuse --repeat 100 'D:\Videos\clip.mp4' 'D:\Videos\sample.mkv'
```

Point `--dll` at the DLL from the build/install you actually want to evaluate.
`--output` requires one input and one repetition, always writes PNG, and refuses
to overwrite an existing file. Without it, image generation stays in memory.

- Default: deterministic positions between 25% and 75% of duration, with up to
  three candidates when near-black/white frames are found. See [ADAPTIVE.md](ADAPTIVE.md).
- Initial seek uses a nearby keyframe. If that falls outside the middle half,
  it is corrected by exact seek. Quality retries use exact seek to avoid sampling
  the same keyframe again. `--at` remains a single exact request without substitution.
- `--legacy` preserves the original exact 10%-up-to-30-second policy and full-size
  capture for A/B measurement only; it cannot be combined with `--at`.
- Default maximum side: 256 pixels; aspect ratio is preserved.
- Default timeout: 15 seconds per operation (initialization, each batch item,
  final teardown). A batch item's budget includes initialization and teardown.
- `--reuse` keeps a dedicated instance, but stops and releases every input after
  capture. It never uses the application's player instance or user mpv config.
- `--check-release` attempts an exclusive read after every successful capture,
  **while the reused instance is still alive**. Use isolated fixtures: another
  application legitimately holding the file will also make this check fail.

## Output and measurement

Stdout is JSON Lines: runtime inventory when requested, baseline resource sample,
per-input results, and a summary with median and nearest-rank p95. Inputs are
identified by their 1-based argument index, not by their paths or titles.
Stderr contains CLI/startup errors and deadline termination messages.

Timing includes load, seek, screenshot plus RGBA conversion/resizing, and stop.
`total_with_teardown_ms` additionally includes per-file instance initialization
and destruction in fresh mode, exclusive-read checks, and optional PNG writing.
Reused instance startup/final destruction is outside the per-item distribution.
Benchmark without PNG output and compare identical inputs, repetition counts,
and release builds. These are warm-cache measurements, not cold-storage claims.

Resource samples report Windows working set, commit charge, and process handle
count. Allocator retention and decoder caches can raise memory after warmup;
these samples alone cannot prove the absence of native leaks.

Exit codes: `0` all captures succeeded; `1` capture/startup/output failure;
`2` invalid CLI syntax; `124` watchdog deadline.

## Reproducible tests

```powershell
cargo test --locked --manifest-path tools/mpv-thumbnail-poc/Cargo.toml
cargo test -p mtt-mpv-thumbnail
cargo clippy --locked --manifest-path tools/mpv-thumbnail-poc/Cargo.toml --all-targets -- -D warnings

.\tools\mpv-thumbnail-poc\tests\smoke.ps1 `
    -Dll '.\target\debug\libmpv-2.dll' `
    -WorkDir 'D:\Tests\mpv-thumbnail-run' `
    -Repeat 25
```

`WorkDir` must not exist; the script never clears or overwrites an existing
directory. It creates synthetic media, JSONL reports, and explicit inspection
PNGs there. **FFmpeg is required only to generate these test fixtures**, not to
run the tool. Use a recent FFmpeg with x264, x265, libvpx-vp9, libaom-av1, and
`-display_rotation` support. Its path can be supplied through `-Ffmpeg`.

The script checks four codecs, MP4/MKV/WebM, vertical video, actual display
rotation metadata, 4K, a short clip, synthetic PQ/BT.2020 tagging, Unicode/spaces,
corrupt/truncated/audio-only inputs, recovery after failure, seeks, output
overwrite protection, and file release. HDR decoding success is **not** a
color-accuracy or tone-mapping test.

For the clean-Windows test, create fixtures elsewhere and copy only the tool,
the existing libmpv runtime (and its existing dependencies, if any), and fixtures
to a clean VM. Do not install FFmpeg or codec extensions there. Record the OS,
DLL hash, `--inspect` inventory, reports, and visual inspection results without
publishing personal paths or media.

## Implementation boundaries

- `crates/mtt-mpv-thumbnail/src/ffi.rs`: dynamically loaded subset of the public libmpv `client.h` ABI.
- `crates/mtt-mpv-thumbnail/src/node.rs`: typed borrowed views and inventory serialization.
- `crates/mtt-mpv-thumbnail/src/capture.rs`: dedicated instance, async commands/events, seek/capture/stop.
- `crates/mtt-mpv-thumbnail/src/extract.rs`: policy orchestration and bounded retries.
- `crates/mtt-mpv-thumbnail/src/selection.rs`: stable dispersed candidate positions.
- `crates/mtt-mpv-thumbnail/src/quality.rs`: conservative sampled blank-frame detection.
- `crates/mtt-mpv-thumbnail/src/pixels.rs`: checked stride/channel conversion and thumbnail resizing.
- `metrics.rs`: process samples, percentiles, standalone deadline watchdog.
- `main.rs`: CLI and batch runner.

The shared backend is a Rust crate, compiled into the executable, **not** another
DLL to distribute. Its tests now run with `cargo test -p mtt-mpv-thumbnail`.

The existing Rust `libmpv2` wrapper's `command()` discards command results.
This POC therefore calls `mpv_command_async` and copies `screenshot-raw video`
pixels from `MPV_EVENT_COMMAND_REPLY` before the next event wait invalidates
them. Event-owned nodes must **not** be freed by the caller; property-owned
nodes are freed with `mpv_free_node_contents`.

Software decoding (`hwdec=no`) and `vo=null` avoid GPU/window dependencies. The
tested build supports software screenshot fallback even with null video output.
For a file that emitted `START_FILE`, stop waits for both the command reply and
`END_FILE`: the reply alone does not guarantee the file has finished closing.
Loads cancelled before `START_FILE` do not require a nonexistent `END_FILE`.

Automatic MKV capture now prefers the embedded attachment named exactly
`cover.jpg` or `cover.jpeg`, using the existing DLL's internal FFmpeg demuxer.
Missing/unreadable covers use the adaptive frame policy; `--at` and `--legacy`
still capture video frames. See [ADAPTIVE.md](ADAPTIVE.md) for probe budgets,
fallback behavior, and production cache compatibility.

## Not production-ready

- Clean Windows and simultaneous playback in the actual application still need
  validation; this tool does not integrate with or launch the application.
- Blank-frame rejection is a heuristic, not semantic scene selection or a
  cross-file similarity detector. Similar thumbnails can still occur.
- HDR is not guaranteed to be correctly tone-mapped to SDR.
- A native libmpv/FFmpeg scale filter reduces the captured frame before RGBA
  copying. This does not eliminate full-resolution video decoding. `--legacy`
  retains full-size screenshots; the 256 MiB checked-copy limit is **not** a
  decoder/process memory limit.
- Commands are async, but native initialization, property queries, decoding, or
  destruction could still hang/crash. The standalone watchdog exits this entire
  **test process**; it must not be transplanted into the main application.
  Production needs an explicit isolation/recovery decision.
- CPU decoding, one task at a time, local regular files only; no production
  scheduling, cache integration, GPU acceleration, images, or document support.

See [RESULTS.md](RESULTS.md) for observed results and remaining acceptance work.
