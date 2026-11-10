//! Must not do I/O; callers own the sockets and buffers.

pub mod api;
pub mod message;
pub mod socket_path;
pub mod tags;

pub use message::{Attach, ClientMsg, Control, DecodeError, ExitReason, ServerMsg, Size, VERSION};
