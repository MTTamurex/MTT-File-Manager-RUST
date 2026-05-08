# Source Availability for Public Installers

This file records where recipients of public MTT File Manager installers can
find source materials for the repository-authored code and the main bundled
GPL/LGPL components.

## MTT File Manager

Repository-authored source code:
- https://github.com/MTTamurex/MTT-File-Manager-RUST

License:
- Apache-2.0 for repository-authored code unless a file-specific notice says
  otherwise.

## mpv and libmpv

Current installer component:
- `libmpv-2.dll`
- SHA-256: `8F77950F7D98770B1FFB1D02742C1EE5A17F9C05BCCE0723693188C69CC7C865`
- Version metadata: `v0.41.0-514-g06f4ce75a`
- Embedded comment: `mpv is distributed under the terms of the GNU General
  Public License Version 2 or later.`
- Exact binary package: `mpv-dev-x86_64-v3-20260419-git-06f4ce7.7z`
- Exact binary package URL:
  https://sourceforge.net/projects/mpv-player-windows/files/libmpv/mpv-dev-x86_64-v3-20260419-git-06f4ce7.7z/download

Known source/build locations:
- mpv upstream source: https://github.com/mpv-player/mpv
- Reported Windows binary distribution family:
  https://sourceforge.net/projects/mpv-player-windows/files/libmpv/
- Reported shinchiro Windows build scripts:
  https://github.com/shinchiro/mpv-winbuild-cmake

Public release handling:
- Treat the current `libmpv-2.dll` as GPLv2+ unless replaced with a confirmed
  LGPL-compatible build.
- Keep the GPL text and mpv notices with installers that include this DLL.
- The current SourceForge binary archive does not include license files, so the
  installer carries the upstream GPL/LGPL texts and mpv notices from this
  directory.
- Keep the exact DLL hash, source/build evidence, and this file synchronized
  with each public release.
- If the exact source/build package for a future DLL cannot be identified,
  replace the DLL with one whose corresponding source and build provenance can
  be recorded before publishing that release.

## Copied mpv Lua Files

Bundled/copied source files:
- `mpv_ui/portable_config/scripts/autoload.lua`
- `mpv_ui/portable_config/scripts/osc.lua.bak` is kept only as a repository
  reference and is not installed by the current installer.

Source:
- https://github.com/mpv-player/mpv
- https://github.com/mpv-player/mpv/blob/master/TOOLS/lua/autoload.lua

## ModernH

Bundled/copied source files:
- `mpv_ui/portable_config/scripts/modernH.lua`
- `mpv_ui/portable_config/script-opts/osc.conf`

Source:
- https://github.com/HarkeshBhatia/ModernH

License:
- LGPL-2.1

## Notes

This file is a source-location aid, not a replacement for the license texts in
this directory. Keep `THIRD_PARTY_NOTICES.md`, `PROVENANCE.md`, and this file
updated whenever a bundled DLL, copied Lua script, or other redistribution-
sensitive component changes.