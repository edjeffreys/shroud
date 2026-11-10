//! Must not know what a pane is; leaves are PaneIds.

mod arrange;
pub mod float;
pub mod rearrange;
pub mod rect;
pub mod shape;
pub mod tree;

pub use float::FloatRect;
pub use rearrange::Slot;
pub use rect::Rect;
pub use shape::Shape;
pub use tree::{Axis, Direction, MAX_PANES, PaneId, Placement, Tree, neighbour};
