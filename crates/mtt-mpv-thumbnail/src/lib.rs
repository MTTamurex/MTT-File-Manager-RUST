//! Dedicated, software-only libmpv thumbnail sessions. Run in an isolated process:
//! native calls may block or crash; this crate is not a crash/security sandbox.
mod capture;
mod cover;
#[cfg(test)]
mod cover_tests;
mod extract;
mod ffi;
#[cfg(test)]
mod native_tests;
mod node;
mod pixels;
mod quality;
mod selection;

pub use capture::Session;
pub use extract::Capture;
pub use ffi::Api;
