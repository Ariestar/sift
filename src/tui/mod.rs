//! Shared terminal UI primitives for the workspace browser.
//!
//! Product orchestration lives in `commands/browse`. Pane **data** capability
//! (sliding window / ensure) lives in [`crate::pane`]. This area only renders
//! workspace models and chrome (`pane` module here = borders / titles / lists).

pub mod content;
pub mod pane;
pub mod panic;
pub mod terminal;
pub mod theme;
pub mod workspace;

#[cfg(all(test, windows))]
mod conpty_tests;
