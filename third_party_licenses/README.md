# Third-Party License Bundle

This directory is distributed with public installers and supplements the
top-level `LICENSE`, `NOTICE`, and `THIRD_PARTY_NOTICES.md` files.

It contains license texts, attribution notes, and binary provenance for bundled
or redistribution-sensitive components. The repository-authored code remains
Apache-2.0 unless a file-specific notice says otherwise.

This bundle is meant to make public installer distribution practical: keep it
with the installer so recipients can see the relevant notices, source-location
notes, and third-party license texts.

## Files

- `GPL-2.0.txt` - GNU GPL version 2 text for mpv components or builds when GPL
  terms apply.
- `LGPL-2.1.txt` - GNU LGPL version 2.1 text for `libmpv2`, ModernH, and mpv
  components or builds when LGPL terms apply.
- `MPV-COPYRIGHT-NOTICE.txt` - mpv upstream license mode summary and public
  release evidence requirement.
- `PDFIUM-LICENSE.txt` - PDFium upstream license notice.
- `PDFIUM-BINARIES-LICENSE.txt` - MIT license for the bblanchon/pdfium-binaries
  packaging project when that distribution is used as the binary source.
- `pdfium-win-x64/` - Exact license, version, build args, and third-party
  license files copied from the original PDFium binary package that matches the
  shipped `pdfium.dll` hash.
- `UNRAR-LICENSE.txt` - Upstream UnRAR license text for the embedded C/C++
  source used by the `unrar` crate.
- `MATERIAL-DESIGN-ICONIC-FONT-NOTICE.txt` - Attribution and license pointer for
  the Material Design Iconic Font asset present in the source tree.
- `SOURCE-AVAILABILITY.md` - Source repository and upstream source-location
  notes for public installer releases, especially GPL/LGPL components.
- `PROVENANCE.md` - Current known binary identities, hashes, and release
  evidence requirements.

When upgrading a bundled DLL or copied third-party file, update both
`THIRD_PARTY_NOTICES.md` and `PROVENANCE.md` before publishing an installer.