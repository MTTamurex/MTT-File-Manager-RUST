# Third-Party Provenance

This file records the release-sensitive third-party components currently known
to be distributed by the installer or linked into release binaries.

## Runtime DLLs

| Component | Runtime file | Current identity | SHA-256 | License status for public release |
| --- | --- | --- | --- | --- |
| libmpv | `target/release/libmpv-2.dll` | Source family: shinchiro mpv Windows libmpv builds. Exact package: `mpv-dev-x86_64-v3-20260419-git-06f4ce7.7z`, from `https://sourceforge.net/projects/mpv-player-windows/files/libmpv/mpv-dev-x86_64-v3-20260419-git-06f4ce7.7z/download`. Version info: `v0.41.0-514-g06f4ce75a`. File size: `121060864` bytes. Embedded `Comments`: `mpv is distributed under the terms of the GNU General Public License Version 2 or later.` | `8F77950F7D98770B1FFB1D02742C1EE5A17F9C05BCCE0723693188C69CC7C865` | Current binary metadata identifies this DLL as GPLv2+. Treat public installers containing this DLL as GPL-involving distributions unless the DLL is replaced with a confirmed LGPLv2.1+ build and the exact source/build evidence is recorded here. The SourceForge binary archive does not include license files; this repository provides upstream GPL/LGPL texts, notices, and source-location records in `third_party_licenses/`. |
| PDFium | `target/release/pdfium.dll`, `vendor/pdfium.dll` | Original maintainer-retained package directory name: `pdfium-win-x64`; matching license bundle copied to `third_party_licenses/pdfium-win-x64/`. Package `VERSION`: `MAJOR=148`, `MINOR=0`, `BUILD=7749`, `PATCH=0`. Package `args.gn`: `target_cpu = "x64"`, `target_os = "win"`, `pdf_enable_v8 = false`, `pdf_enable_xfa = false`. | `7167AEE6BB3D2724EE62FD83BBEB8883EDC786A6E1999782857D4952536A0ED3` | The original package `bin/pdfium.dll`, `vendor/pdfium.dll`, and `target/release/pdfium.dll` have the same SHA-256. Distribute the copied package `LICENSE` and `licenses/` notices with public installers. |

The current `libmpv-2.dll` status is a distribution obligation, not a
distribution prohibition. If this DLL remains in the official installer, keep
the installer notices, keep MTT File Manager source available, and keep
`SOURCE-AVAILABILITY.md` synchronized with the exact mpv binary/source evidence
available for the release. If exact corresponding source evidence cannot be
kept for a future DLL, replace the DLL with a build whose source and build
provenance can be recorded.

## Rust Components With Embedded Native Code

| Component | Cargo package | Current version | License status for public release |
| --- | --- | --- | --- |
| UnRAR C/C++ source | `unrar`, `unrar_sys` | `0.5.8` | The Rust wrapper is MIT OR Apache-2.0. The embedded UnRAR source uses the upstream UnRAR license and may be used to handle RAR archives, but not to recreate the RAR compression algorithm or develop a RAR-compatible archiver. |

## Release Checklist

- Confirm the exact `libmpv-2.dll` source archive URL, build project,
  commit/tag, and build flags before each future upgrade. A known shinchiro
  source family is useful for provenance, but does not override the DLL's
  embedded GPL metadata.
- Keep `SOURCE-AVAILABILITY.md` synchronized with each public release.
- If using LGPL libmpv, record evidence that mpv was built without GPL-only
  files and that linked libraries, especially FFmpeg, do not make the resulting
  binary GPL.
- Keep the DLL SHA-256 values in this file and in
  `installer/build_installer.ps1` synchronized.
- Keep the matching PDFium license/notices with any PDFium binary upgrade.
- If `pdfium.dll` is upgraded, replace `third_party_licenses/pdfium-win-x64/`
  with the license bundle from the new binary package and update the package
  version/build args in this file.
- Keep the UnRAR license text with any release that includes RAR extraction.