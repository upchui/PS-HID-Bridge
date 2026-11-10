//! PS5 HID Bridge core library.
//!
//! Reads mouse and keyboard via evdev, decides with the trigger engine when
//! the bridge is active, encrypts input with ChaCha20-Poly1305 and sends it
//! over UDP to the W5500-EVB-Pico2 board (see `protocol/SPEC.md`).

pub mod config;
pub mod controller;
pub mod counter;
pub mod crypto;
pub mod engine;
pub mod input;
pub mod keymap;
pub mod mouse;
pub mod protocol;
pub mod script;
pub mod transport;
pub mod trigger;

/// Version of the desktop software (shared by the CLI and the GUI).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_semver_like() {
        assert_eq!(VERSION.split('.').count(), 3);
    }
}
