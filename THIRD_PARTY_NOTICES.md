# Third-Party Notices

This file highlights bundled, copied, or otherwise redistribution-sensitive
third-party components used by this repository. It is not a full inventory of
every transitive Cargo dependency in Cargo.lock.

Most Rust crates used through Cargo in this project are permissively licensed,
commonly MIT or MIT OR Apache-2.0. The entries below focus on components that
are shipped as files, runtime DLLs, copied scripts, fonts, or statically linked
code with separate terms.

Full license texts and release-sensitive binary provenance are kept in
`third_party_licenses/`. Public installers copy that directory into the
installation folder alongside this notice file.

## Practical Public Distribution Rules

MTT File Manager is intended to be published as a free and open-source project.
The current Windows installer may still include third-party components under
non-Apache terms, especially `libmpv-2.dll` and copied mpv Lua scripts.

For public releases:

- Do not describe the installer as Apache-2.0-only. The repository-authored
  code is Apache-2.0, while the installer is a multi-license bundle.
- Keep `LICENSE`, `NOTICE`, this file, and `third_party_licenses/` with the
  installer.
- Keep the source repository available for the MTT File Manager code shipped in
  each public release.
- Treat the currently shipped `libmpv-2.dll` as GPLv2+ unless it is replaced by
  a confirmed LGPL-compatible build. GPLv2+ is not a distribution ban; it means
  public releases must preserve GPL notices and provide corresponding source
  access or source-location information for the GPL component.
- Do not use UnRAR source code to develop RAR compression or a RAR-compatible
  archiver. The current use is archive handling/extraction.

Source-location notes for public installers are kept in
`third_party_licenses/SOURCE-AVAILABILITY.md`.

## Bundled or copied files

### ModernH

Files:
- `mpv_ui/portable_config/scripts/modernH.lua`
- `mpv_ui/portable_config/script-opts/osc.conf`

Source:
- https://github.com/HarkeshBhatia/ModernH

License:
- LGPL-2.1

Notes:
- These files originate from ModernH and may contain local modifications.
- If you redistribute modified versions, preserve upstream notices and comply
  with applicable LGPL-2.1 obligations for the modified files.
- Full LGPL-2.1 text is included at `third_party_licenses/LGPL-2.1.txt`.

### mpv autoload script

Files:
- `mpv_ui/portable_config/scripts/autoload.lua`

Source:
- https://github.com/mpv-player/mpv/blob/master/TOOLS/lua/autoload.lua

License:
- Upstream mpv project licensing. The mpv repository states GPLv2+ by default,
  or LGPLv2.1+ when built with `-Dgpl=false`.

Notes:
- Treat this file as an upstream mpv component and keep the relevant upstream
  mpv licensing materials with redistributions.
- GPL-2.0 and LGPL-2.1 texts are included in `third_party_licenses/` because
  the applicable terms depend on the exact mpv source/build used.
- mpv license mode notes are included at
  `third_party_licenses/MPV-COPYRIGHT-NOTICE.txt`.
- Source-location notes are included at
  `third_party_licenses/SOURCE-AVAILABILITY.md`.

### mpv OSC backup file

Files:
- `mpv_ui/portable_config/scripts/osc.lua.bak`

Source:
- Backup copy derived from mpv OSC sources.

License:
- Upstream mpv project licensing. The mpv repository states GPLv2+ by default,
  or LGPLv2.1+ when built with `-Dgpl=false`.

Notes:
- This backup file is kept in the repository for reference and is not installed
  by the current installer configuration.
- GPL-2.0 and LGPL-2.1 texts are included in `third_party_licenses/` because
  the applicable terms depend on the exact mpv source/build used.
- mpv license mode notes are included at
  `third_party_licenses/MPV-COPYRIGHT-NOTICE.txt`.
- Source-location notes are included at
  `third_party_licenses/SOURCE-AVAILABILITY.md`.

### Material Design Iconic Font

Files:
- `mpv_ui/portable_config/fonts/Material-Design-Iconic-Font.ttf`

Source:
- https://github.com/zavoloklom/material-design-iconic-font

License:
- The upstream repository `License.md` currently contains the Creative Commons
  Attribution-ShareAlike 4.0 International text.

Notes:
- Upstream project materials have historically referenced multiple license
  descriptors. Review the upstream distribution materials if you plan to
  redistribute this font asset separately or modify it.
- Attribution and the upstream license reference are included at
  `third_party_licenses/MATERIAL-DESIGN-ICONIC-FONT-NOTICE.txt`.

## Runtime components distributed with builds or installer

### libmpv runtime

Files:
- `target/release/libmpv-2.dll`
- Installer/runtime copies of `libmpv-2.dll`

Source:
- https://github.com/mpv-player/mpv
- https://crates.io/crates/libmpv2 (Rust bindings only)

License:
- The `libmpv2` Rust crate is LGPL-2.1.
- The mpv project states GPLv2+ by default, or LGPLv2.1+ when built with
  `-Dgpl=false`.

Notes:
- The applicable redistribution terms depend on the exact `libmpv-2.dll` binary
  you ship.
- Prefer a shared-library LGPL build if you want simpler downstream
  redistribution terms.
- The currently staged `libmpv-2.dll` is the shinchiro SourceForge package
  `mpv-dev-x86_64-20260419-git-06f4ce7.7z`, has version metadata
  `v0.41.0-514-g06f4ce75a`, SHA-256
  `AE7B7B6B3ECCC7AEEFF3CBEF30FD175F97B27F1B815F89D0BD0CCA926C8C2D99`, and an
  embedded comment stating that mpv is distributed under the GNU GPL version 2
  or later.
- Before publishing a public installer, record the exact DLL source/build
  evidence in `third_party_licenses/PROVENANCE.md`. Treat the DLL as GPLv2+
  unless that evidence proves LGPLv2.1+ eligibility for the shipped binary and
  its linked dependencies.
- mpv license mode notes are included at
  `third_party_licenses/MPV-COPYRIGHT-NOTICE.txt`.
- Source-location notes are included at
  `third_party_licenses/SOURCE-AVAILABILITY.md`.

### PDFium runtime

Files:
- `target/release/pdfium.dll`
- Installer/runtime copies of `pdfium.dll`

Source:
- https://pdfium.googlesource.com/pdfium/
- https://github.com/bblanchon/pdfium-binaries

License:
- `pdfium-render` (Rust bindings) is MIT OR Apache-2.0.
- PDFium itself remains under its upstream license and third-party notices.
- The `pdfium-binaries` packaging repository is MIT, but that does not replace
  PDFium's own licensing obligations.

Notes:
- Keep the license information and notices that correspond to the specific
  PDFium build you redistribute.
- The current `pdfium.dll` matches the original `pdfium-win-x64` package, whose
  `VERSION` records `148.0.7749.0` and whose `args.gn` records a Windows x64
  build with V8/XFA disabled.
- The matching package license bundle is copied to
  `third_party_licenses/pdfium-win-x64/` and is included by public installers.

## DOCX viewer dependencies

### BetterOffice DOCX engine

Components:
- `betteroffice-docx-edit`, `betteroffice-docx-layout`,
  `betteroffice-docx-parse`, `betteroffice-docx-raster`, and
  `betteroffice-ooxml-text`, all pinned to version `0.3.0`.

Source:
- https://github.com/openooxml/betteroffice
- https://crates.io/crates/betteroffice-docx

License:
- Apache-2.0. The complete license text is included in the top-level
  `LICENSE` file.

Attribution:
- The upstream notice is reproduced in
  `third_party_licenses/BETTEROFFICE-NOTICE.txt` and included in public
  installers.
- The application uses the native parsing, layout, and raster crates for a
  read-only viewer. It does not redistribute BetterOffice's editor UI or font
  assets.

### Yrs

Component:
- Yrs `0.27.3`, required transitively by BetterOffice and vendored under
  `vendor/yrs`. `vendor/yrs/src/block.rs` has one equivalent syntax change so
  the dependency builds with stable Rust; the patch is documented in
  `vendor/yrs/PATCHES.md`.

Source:
- https://github.com/y-crdt/y-crdt
- https://crates.io/crates/yrs/0.27.3

License:
- MIT. The full text is included in `third_party_licenses/YRS-LICENSE.txt`
  and `vendor/yrs/LICENSE`.

Attribution:
- Copyright (c) 2020 Bartosz Sypytkowski and Kevin Jahns.

### Raster and ZIP support

Components:
- `dashmap` `6.2.1`, MIT; its license text is in
  `third_party_licenses/DASHMAP-LICENSE.txt`.
- `quick-xml` `0.41.0`, MIT; its license text is in
  `third_party_licenses/QUICK-XML-LICENSE.txt`.
- `zip` `8.6.0`, MIT; its license text is in
  `third_party_licenses/ZIP-8-LICENSE.txt`.
- `tiny-skia` and `tiny-skia-path` `0.12.0`, used by BetterOffice's native
  raster backend; both are BSD-3-Clause.
- `libz-rs-sys` and `zlib-rs` `0.5.5`, used by the BetterOffice OPC ZIP reader;
  both use the Zlib license.
- `ryu-js` `1.0.3`, used by BetterOffice serialization. Its Apache-2.0 OR
  BSL-1.0 terms are used under the Apache-2.0 option.

License texts:
- `third_party_licenses/DASHMAP-LICENSE.txt`
- `third_party_licenses/QUICK-XML-LICENSE.txt`
- `third_party_licenses/ZIP-8-LICENSE.txt`
- `third_party_licenses/TINY-SKIA-LICENSE.txt`
- `third_party_licenses/ZLIB-RS-LICENSE.txt`
- The Apache-2.0 text is in the top-level `LICENSE`.

## Statically linked or embedded code

### unrar

Component:
- Archive extraction support compiled via the `unrar` crate.

Source:
- https://github.com/muja/unrar.rs
- https://crates.io/crates/unrar

License:
- The Rust wrapper is MIT OR Apache-2.0.
- The embedded UnRAR sources use the upstream UnRAR license.

Notes:
- Distributing binaries with UnRAR support may trigger additional upstream
  notice obligations.
- The upstream UnRAR license text is included at
  `third_party_licenses/UNRAR-LICENSE.txt`.

## Historical references

The README credits the anthonybaldwin RTX HDR / RTX VSR gist as an early
development reference. The current `mpv_ui/portable_config/scripts/vsr.lua`
implementation has been reworked in-repository and is not treated here as a
copied third-party file.
