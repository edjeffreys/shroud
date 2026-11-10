//! Must not encode for panes; that needs libghostty-vt and lives in shroud-vt.

pub mod bindings;
pub mod chord;
pub mod commands;
pub mod decode;
pub mod event;
pub mod keybinds;
pub mod outer_colors;
pub mod shortcut;

pub use bindings::Bindings;
pub use chord::Chord;
pub use decode::Decoder;
pub use event::{Event, KeyAction, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent, key};
pub use keybinds::{Action, Keybinds, Outcome, Selecting};
pub use outer_colors::{ColorSlot, OuterColors, Rgb};
pub use shortcut::Shortcut;
