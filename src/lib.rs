pub mod cli;
pub mod commands;
pub mod core;
pub mod mcp;
pub mod ops;
pub mod platform;
pub mod trace;
pub mod watch;

pub use core::error::{Error, Result};
pub use core::paths::Paths;
pub use core::{error, paths, state, types, ui};
