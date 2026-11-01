pub mod backend;
pub mod desktop;
pub mod lifecycle;
pub mod process;

pub use backend::launch_application;
pub use desktop::{Desktop, validate_resolution};
pub use lifecycle::*;
pub use process::X11_INSTALL;

pub fn init() {}
