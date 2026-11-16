# Local small-caps rendering patch

Source: `betteroffice-docx-raster` 0.3.0 from crates.io, upstream commit
`34b9e93a657158fcf3d1d1035df3202031625bdd`, `crates/docx-raster`.
The upstream source and README are preserved, with Apache-2.0 LICENSE and
NOTICE from the same BetterOffice release. The Carlito test font retains its
OFL license in `tests/assets`.

Local modifications:

- Render small caps instead of rejecting `TextRunPrimitive` flags and CSS
  `small-caps` shorthands.
- Match authoritative layout: prefer a font's OpenType `smcp` substitution
  per lowercase character; otherwise synthesize language-aware uppercase
  glyphs at `WORD_SMALL_CAPS_ADVANCE_SCALE`.
- Keep font fallback, source-cluster spacing, clipping and whole-run visual
  transforms intact. Only glyph outlines and placement change; source text
  and the stored display list remain untouched.
- Keep authoritative cluster origins and line breaks supplied by layout.
  The raster does not remeasure or reflow the document.
- Accept the small-caps flag on already-positioned glyph runs without
  reapplying casing or scaling to their glyphs.
- Keep the existing rendering budgets, charging uppercase expansions before
  allocating the shaping plan.

The implementation and regression tests live in `src/font/small_caps.rs`
and `src/font/small_caps/tests.rs`. Original upstream font unit tests are
preserved; upstream integration tests are not included in this local copy.
The root `[patch.crates-io]` applies this copy to BetterOffice consumers.
