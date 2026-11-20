//! PS HID Bridge core library.
//!
//! Reads mouse and keyboard via evdev, decides with the trigger engine when
//! the bridge is active, encrypts input with ChaCha20-Poly1305 and sends it
//! over UDP to the W5500-EVB-Pico2 board (see `protocol/SPEC.md`).

pub mod access;
pub mod clock;
pub mod config;
pub mod controller;
pub mod counter;
pub mod crypto;
pub mod diagnostics;
pub mod discover;
pub mod engine;
pub mod idle;
pub mod input;
pub mod install;
pub mod keymap;
pub mod lockfile;
pub mod log;
pub mod lowlat;
pub mod mouse;
pub mod oneway;
pub mod protocol;
pub mod rt;
pub mod script;
pub mod service;
pub mod touch;
pub mod transport;
pub mod trigger;
pub mod typing;
pub mod update;
#[cfg(feature = "udisks")]
pub mod usbflash;
pub mod window;

/// Version of the desktop software (shared by the CLI and the GUI):
/// MAJOR.MINOR.PATCH, passed in by the Makefile as PSHB_VERSION
/// (tools/version.sh). A plain `cargo build` marks itself as a dev build.
pub const VERSION: &str = match option_env!("PSHB_VERSION") {
    Some(v) if !v.is_empty() => v,
    _ => concat!(env!("CARGO_PKG_VERSION"), "-dev"),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver_like() {
        assert_eq!(VERSION.split('.').count(), 3);
    }
}
