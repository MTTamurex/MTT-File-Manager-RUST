# Vendored Yrs patch

This directory contains Yrs 0.27.3, from https://crates.io/crates/yrs/0.27.3
and https://github.com/y-crdt/y-crdt.

The sole source change is in `src/block.rs`: the `TypePtr::ID` match guard is
expressed as a nested match with the same branches. This avoids the experimental
`if_let_guard` feature while keeping the project on stable Rust.

The optional Cargo `authors` metadata is omitted; required copyright attribution
remains in the MIT license file.

Yrs is licensed under MIT. The complete license is retained in `LICENSE` and
`third_party_licenses/YRS-LICENSE.txt`.
