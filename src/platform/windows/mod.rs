pub mod desktop;
pub mod exec;
pub mod keeper;
pub mod lifecycle;
pub mod process;
pub mod testapp;
pub mod viewer;
pub mod win32;

pub use desktop::{Desktop, validate_resolution};
pub use lifecycle::*;

pub fn init() {
    win32::init_dpi();
}
