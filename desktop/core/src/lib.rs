//! PS5 HID Bridge core library.
//!
//! Reads mouse and keyboard via evdev, encrypts input with ChaCha20-Poly1305
//! and sends it over UDP to the W5500-EVB-Pico2 board (see `protocol/SPEC.md`).

pub mod counter;
pub mod crypto;
pub mod protocol;
pub mod transport;

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
