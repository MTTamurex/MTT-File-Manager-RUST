//! Miller's Columns view: hierarchical multi-pane navigation (Finder-style).
//!
//! Rendering is split into:
//! - `layout`: ancestor-chain computation and horizontal strip geometry.
//! - `column`: compact rows and their interactions for ancestor columns.
//! - `renderer`: virtualized column rendering and scrolling.
//! - `grouping`: group geometry for rendering and rectangle selection.
//!
//! The focused (rightmost) column is rendered by the normal details list view
//! (see the `miller_bridge` app operation), which gives it the full interaction
//! stack (rename, multi-select, drag, rectangle selection, keyboard, context
//! menu). Orchestration lives in the bridge.

pub mod column;
mod grouping;
pub mod layout;
mod renderer;

pub use column::{MillerColumnAction, MillerColumnContext, MillerColumnOutput};
pub use layout::{ancestor_chain, ANCESTOR_COL_WIDTH, COL_ROW_HEIGHT, FOCUSED_COL_WIDTH};

#[cfg(test)]
mod renderer_tests;
pub use renderer::render_miller_column;
