# Third-Party Notices

This file highlights bundled, copied, or otherwise redistribution-sensitive
third-party components used by this repository. It is not a full inventory of
every transitive Cargo dependency in Cargo.lock.

Most Rust crates used through Cargo in this project are permissively licensed,
commonly MIT or MIT OR Apache-2.0. The entries below focus on components that
are shipped as files, runtime DLLs, copied scripts, fonts, or statically linked
code with separate terms.

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

## Historical references

The README credits the anthonybaldwin RTX HDR / RTX VSR gist as an early
development reference. The current `mpv_ui/portable_config/scripts/vsr.lua`
implementation has been reworked in-repository and is not treated here as a
copied third-party file.