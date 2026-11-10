//! Turning pane grids into bytes for the outer terminal. Never sees a pane, PTY or socket.

pub mod borders;
pub mod cell;
pub mod color;
pub mod compose;
pub mod diff;
pub mod frame;
pub mod grid;
pub mod highlight;
pub mod images;
pub mod link;
mod num;
pub mod passthrough;
pub mod sgr;
pub mod text;
#[cfg(test)]
mod visible;

pub use cell::{Cell, Style, Underline, Width};
pub use color::Color;
pub use compose::{PaneView, compose};
pub use diff::diff;
pub use frame::{Cursor, CursorShape, Frame};
pub use grid::Grid;
pub use highlight::{Highlight, Shape};
pub use link::{LinkId, Links, NO_LINK};
