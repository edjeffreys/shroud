//! Must stay free of I/O and `unsafe`, so everything here is testable as plain functions.
#![forbid(unsafe_code)]

pub mod config;
pub mod input;
pub mod layout;
pub mod overlay;
pub mod pace;
pub mod protocol;
pub mod render;
pub mod shell;
pub mod statusbar;
pub mod switcher;
pub mod theme;
pub mod visual;
pub mod watchdog;
