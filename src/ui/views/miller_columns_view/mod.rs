//! Miller's Columns view: hierarchical multi-pane navigation (Finder-style).
//!
//! Rendering is split into:
//! - `layout`: ancestor-chain computation and horizontal strip geometry.
//! - `column`: the compact icon + name renderer for ancestor columns.
//!
//! The focused (rightmost) column is rendered by the normal details list view
//! (see the `miller_bridge` app operation), which gives it the full interaction
//! stack (rename, multi-select, drag, rectangle selection, keyboard, context
//! menu). Orchestration lives in the bridge.

pub mod column;
pub mod layout;

pub use column::{render_miller_column, MillerColumnAction, MillerColumnContext};
pub use layout::{ancestor_chain, ANCESTOR_COL_WIDTH, COL_ROW_HEIGHT, FOCUSED_COL_WIDTH};
