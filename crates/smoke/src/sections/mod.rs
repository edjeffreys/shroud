//! Each section starts from nothing, so they can run in any order or at once.

mod api;
mod lifecycle;
mod plugins;
mod sessions;
mod shortcuts;
mod terminal;

use crate::harness::{Env, Report};

pub type Section = fn(&Env, &mut Report);

pub const ALL: &[(&str, Section)] = &[
    ("terminal", terminal::run),
    ("sessions", sessions::run),
    ("api", api::run),
    ("plugins", plugins::run),
    ("plugin-sources", plugins::run_sources),
    ("lifecycle", lifecycle::run),
    ("shortcuts", shortcuts::run),
];
