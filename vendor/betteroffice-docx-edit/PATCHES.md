# Local display-frame observation patch

Source: `betteroffice-docx-edit` 0.3.0 from crates.io, upstream commit
`34b9e93a657158fcf3d1d1035df3202031625bdd`, `crates/docx-edit`.
The source and README are preserved, with the Apache-2.0 LICENSE and NOTICE
from the same BetterOffice release.

Local modification:

- Make the existing `EngineSession::build_display_list_frame_observed` method
  public. No changes to layout, display-list generation, retained state,
  frame encoding or the normal unobserved entry point.
- Document its existing three callbacks on a full build: after display-input
  preparation, after primitive generation and after retaining the list. The
  remaining call time is frame finalization and binary encoding.

The DOCX viewer uses this observer only when diagnostic mode is enabled.
Its diagnostic records contain fixed phase names and durations, not document
data. Parent `build_display_list` timing includes the detailed child stages;
do not add parent and child timings together.

Coverage in `src/docx_viewer/renderer/tests/display_profile.rs` compares
observed and unobserved frame bytes, retained lists and engine counters using
synthetic DOCX packages. Existing viewer regressions exercise headers,
footers, page fields, sections, notes and small caps.

Upstream fixture-based crate tests are preserved in the source but require
assets outside this local copy, including sibling crate fonts and demo files.
Run the application tests above for this patch; this copy does not include
upstream fixtures or integration-test targets.
