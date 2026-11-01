# Local security compatibility patch

Source: `betteroffice-opc` 0.3.0 from crates.io, upstream commit
`34b9e93a657158fcf3d1d1035df3202031625bdd`, `crates/ooxml-opc`.

The upstream Apache-2.0 license and NOTICE are preserved alongside this file.
The source modules and README are copied from the published crate; archive
fixtures and registry metadata are not needed by this local dependency.
Upstream fixture-based unit tests require the `tests/fixtures` directory from
the published crate; validation can run in an isolated copy with those fixtures.

Local modifications:

- Require `zip` 9.0.5 or a compatible newer release instead of vulnerable 8.x.
- Adapt filename access to the fallible API introduced by zip 9.
- Explicitly validate header consistency before reading package parts or copying
  compressed source members. Plain `by_index` does not enable this check.
- Add local regression coverage in `src/security_tests.rs`.

The root `[patch.crates-io]` applies this copy to all BetterOffice consumers.
Remove the patch when a compatible upstream release requires a patched zip.
