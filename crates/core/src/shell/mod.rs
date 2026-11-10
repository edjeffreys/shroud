//! Everything here depends on the pane's shell emitting OSC 133 and OSC 7.

pub mod inject;
pub mod tracker;

pub use inject::integration_env;
pub use tracker::{Finished, Mark, Phase, Tracker};
