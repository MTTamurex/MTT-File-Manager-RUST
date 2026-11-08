# Embedded covers and adaptive thumbnail selection

The application uses this backend by default in an isolated helper, with Windows
Shell/Media Foundation as fallback on extraction failure. The standalone diagnostic
tool uses libmpv only. There are no added decoder DLLs or installed tools. FFmpeg
below is a fixture generator only. See [INTEGRATION.md](INTEGRATION.md) for routing.

## Selection and quality

Automatic MKV requests first probe embedded attachments through libmpv's internal
FFmpeg (`lavf`) demuxer. Only an internal attached-picture track whose `filename`
metadata is exactly `cover.jpg` or `cover.jpeg` is accepted. Other names, paths,
PNG attachments, and external images are not considered covers. The expected
layout is one cover per MKV. Explicit `--at` and `--legacy` bypass cover selection.

Discovery disables track auto-selection and reads metadata without initializing
mpv's main-video decoder (FFmpeg may still probe stream data). The cover is then
loaded in a fresh file/video-output lifecycle, selected before
decoding, scaled with the same native filter, and captured without a seek or
black/white rejection. Even a deliberately black/white cover is preserved. This
avoids returning a stale video frame if a cover decoder fails during a paused
track switch. No attachment bytes are written to disk by the extractor.

The optional cover probe (discovery plus cover load/capture) has a two-second
soft cap and at most half the remaining work budget. Absent, unsupported, or
unreadable covers restore the original/default demuxer and use the frame policy
below. The extra MKV open/probe has a cost, including on files without covers;
other containers do not run this probe. Native hangs still require supervisor
termination. The visible player is unchanged.

1. Hash the path with BLAKE3, using the same input bytes as the existing cache key.
   No video content is hashed or scanned. The seed is stable for the same path;
   renaming/moving the file can change the choice.
2. Map the seed to the middle half of the duration (25%–75%). Two alternative
   positions are dispersed by one third of that interval. Different episodes
   normally get different positions even when their durations are identical.
3. Seek to a nearby keyframe. If its reported timestamp falls outside the middle
   half, correct with exact seek, rather than accidentally capturing the opening.
   Clips of one second or less use exact seek immediately.
4. Capture through a software `lavfi` scale filter, limited to the requested box,
   with bilinear scaling, square output pixels (`reset_sar=1`), and a single filter
   graph thread. The application requests at most 512 pixels on the largest side.
   Source decoding may still occur at full resolution, but RGBA capture/copy is
   reduced. Rotation and anamorphic aspect are tested.
5. Sample at most 64 × 64 pixel locations from the reduced image. A candidate is
   unsuitable only if at least 98.5% of samples have luminance <=16 or >=240 AND
   luminance standard deviation is <=12. This deliberately preserves ordinary
   contrast, dark scenes with detail, and flat-colored animation backgrounds.
6. Accept the first suitable candidate. Only unsuitable frames trigger another
   position, using exact seek so a long GOP cannot return the same keyframe again.
   At most three candidates are decoded; an initial keyframe correction is an
   additional seek for the same candidate, not an extra quality candidate.
7. If all candidates are unsuitable, return the one with the highest contrast/
   non-extreme-sample score. Ties retain the first. Optional retries are skipped
   near the soft deadline; if a later probe fails, an available candidate is kept
   when the session can still stop/release the source within its budget.

With no usable duration, probes are 0, 0.5, and 1 second, only retrying an unsuitable
initial frame. EOF/unseekable media is tolerated when a frame is already available;
there is no whole-stream scan. Very short clips keep their percentage targets
within the available duration and may legitimately return the same frame.

This is not content understanding, opening recognition, or a guarantee of unique
images across a folder. Entirely blank videos must still produce a blank thumbnail.
HDR tone-mapping accuracy remains a separate issue.

## Budget and persistence

The parent retains its 15-second hard request deadline, including queue wait and
process startup. IPC v2 forwards the remaining milliseconds to the child, which
also accounts for initialization and safety probes. The backend reserves 200 ms
for source release, and does not start another optional probe with less than
200 ms left before its soft deadline. A native hang is handled by killing/reaping
the helper before allowing the application's Windows fallback. Cancellation,
unsafe files, and shared-slot queue timeouts do not trigger Windows extraction.

Production retains the normal `thumbnails.db` location, cache keys, and temporary
fallback filename. Valid existing thumbnails are reused without source extraction;
no regeneration is triggered just by changing the backend. Experimental POC stores
are not read, imported, or deleted. Consequently an existing cached video frame
is not automatically replaced by newly supported embedded cover art.

## Reproduction and A/B testing

Build the standalone tool in release mode as described in README.md. Compare the
same local fixture and size, without PNG writing:

```powershell
& $poc --dll $dll --reuse --repeat 30 --size 512 --legacy 'D:\Tests\clip.mp4'
& $poc --dll $dll --reuse --repeat 30 --size 512 'D:\Tests\clip.mp4'
```

`--at` is still an exact diagnostic request: it never substitutes another frame,
even if the image is black/white. Reports include strategy, native scale status,
candidate timestamps/quality, actual raw screenshot dimensions, selected attempt,
retry count/failure, and quality fallback. Paths and seeds are not reported.

Deterministic policy/quality tests need no DLL. Opt-in native tests additionally
require the existing DLL, a recent FFmpeg with x264, and an existing scratch folder:

```powershell
$env:MTT_MPV_TEST_DLL = (Resolve-Path '.\target\debug\libmpv-2.dll').Path
$env:MTT_MPV_TEST_DIR = 'D:\Tests'
cargo test -p mtt-mpv-thumbnail -- --include-ignored --test-threads=1
```

They create only synthetic media in owned temporary subdirectories. They cover
black/white replacement, explicit-position preservation, the three-candidate cap,
all-blank/very-short media, stable dispersion, long GOP opening avoidance,
anamorphic scaling, file release, unknown duration, and legacy/adaptive reuse.
Cover tests add both JPEG attachment names, cover preference, portrait aspect,
blank-cover preservation, absent/wrong-name/corrupt covers, exclusive file access,
explicit/legacy behavior, and mixed-request session reuse.
An expired-probe test also checks teardown before FILE_LOADED and subsequent reuse.
`MTT_MPV_TEST_FFMPEG` can specify the fixture generator executable explicitly.

For the interleaved release benchmark only:

```powershell
cargo test -p mtt-mpv-thumbnail --release interleaved_policy_benchmark -- --ignored --nocapture --test-threads=1
```

## Local measurement

A release interleaved run used one synthetic H.264 3840 × 2160, 12 fps, 8-second
clip with two-second GOPs, a reused instance, 512-pixel thumbnails, 12 samples per
policy, and rotating policy order each round. It used the existing pinned DLL.
This is a machine-specific warm-cache sample, not a general performance guarantee.

| Policy | Median total | p95 total | Mean capture/conversion |
| --- | ---: | ---: | ---: |
| Legacy exact position, full-size screenshot | 361.00 ms | 444.92 ms | 83.94 ms |
| Same exact position, native scaling | 396.44 ms | 622.72 ms | 5.87 ms |
| Adaptive position, full-size screenshot | 305.61 ms | 367.65 ms | 77.63 ms |
| Adaptive position, native scaling (v2) | 273.89 ms | 442.42 ms | 5.46 ms |

The combined policy improved median time by about 24% in this run. Scaling alone
reduced capture/copy cost but did NOT improve total time for the exact-seek case;
filter initialization/seek work and decoder cost still matter. Raw RGBA screenshot
data shrank from 3840 × 2160 to 512 × 288 (about 31.6 MiB to 0.56 MiB).
Long-GOP short clips may need exact correction, and two/three-candidate blank-frame
recovery costs more than a single capture. Representative real-world measurements
and visual review are still necessary before default enablement.

Validation of the adaptive-v2 revision, before embedded-cover support: 20 backend functional/unit tests passed (including
five opt-in native-media tests), plus the release benchmark above. The application's
90-test thumbnail selection and all 12 real-helper integration executions across
H.264/HEVC/VP9/AV1 passed. The normal build check and standalone smoke suite also
passed. Clippy passed for the backend/tool without allowances, and for the full
application with the previously documented unrelated constant-assertion allowance.

Validation of the embedded-cover revision: 26 backend tests, 90 application
thumbnail tests, the synthetic real-helper cover test (both JPEG names), and
12 real-helper integration executions across H.264/HEVC/VP9/AV1 passed. The normal
build check, two normal-build cache-namespace tests, standalone release smoke
suite, formatting, and Clippy checks also passed. Full-application Clippy retains
the same unrelated constant-assertion allowance; backend/tool checks need none.
