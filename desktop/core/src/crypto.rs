//! ChaCha20-Poly1305 (RFC 8439) packet sealing and opening.

use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce, Tag};

use crate::protocol::{Direction, HEADER_LEN, Header, KEY_LEN, PacketType, TAG_LEN, VERSION, nonce};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// Wrong length, version or type.
    Malformed(&'static str),
    /// Tag verification failed (wrong key, wrong direction or tampered).
    Auth,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(why) => write!(f, "ungültiges Paket: {why}"),
            Self::Auth => write!(f, "Signatur ungültig (falscher Schlüssel?)"),
        }
    }
}

impl std::error::Error for OpenError {}

/// The pre-shared key, ready for use.
pub struct PacketKey {
    aead: ChaCha20Poly1305,
}

impl PacketKey {
    pub fn new(key: &[u8; KEY_LEN]) -> Self {
        Self {
            aead: ChaCha20Poly1305::new(&(*key).into()),
        }
    }

    /// Build a packet: header (associated data) | ciphertext | tag.
    pub fn seal(&self, direction: Direction, ptype: PacketType, counter: u64, payload: &[u8]) -> Vec<u8> {
        let header = Header {
            version: VERSION,
            ptype: ptype as u8,
            counter,
        }
        .encode();
        let mut out = Vec::with_capacity(HEADER_LEN + payload.len() + TAG_LEN);
        out.extend_from_slice(&header);
        out.extend_from_slice(payload);
        let n = Nonce::from(nonce(direction, counter));
        let tag = self
            .aead
            .encrypt_inout_detached(&n, &header, (&mut out[HEADER_LEN..]).into())
            .expect("payload length is far below the ChaCha20 limit");
        out.extend_from_slice(&tag);
        out
    }

    /// Verify and decrypt a packet. Returns the header and the plaintext payload.
    pub fn open(&self, direction: Direction, packet: &[u8]) -> Result<(Header, Vec<u8>), OpenError> {
        let header = check(packet)?;
        let body_end = packet.len() - TAG_LEN;
        let mut payload = packet[HEADER_LEN..body_end].to_vec();
        let tag = Tag::try_from(&packet[body_end..]).map_err(|_| OpenError::Malformed("tag"))?;
        let n = Nonce::from(nonce(direction, header.counter));
        self.aead
            .decrypt_inout_detached(&n, &packet[..HEADER_LEN], (&mut payload[..]).into(), &tag)
            .map_err(|_| OpenError::Auth)?;
        Ok((header, payload))
    }
}

/// Structural check: version, known type and exact length.
pub fn check(packet: &[u8]) -> Result<Header, OpenError> {
    if packet.len() < HEADER_LEN + TAG_LEN {
        return Err(OpenError::Malformed("zu kurz"));
    }
    let header = Header::decode(packet);
    if header.version != VERSION {
        return Err(OpenError::Malformed("Version"));
    }
    let ptype = PacketType::from_u8(header.ptype).ok_or(OpenError::Malformed("Typ"))?;
    if packet.len() != HEADER_LEN + ptype.payload_len() + TAG_LEN {
        return Err(OpenError::Malformed("Länge"));
    }
    Ok(header)
}

/// Parse a pre-shared key given as 64 hex digits (whitespace is ignored).
pub fn parse_key_hex(text: &str) -> Result<[u8; KEY_LEN], String> {
    let digits: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() != 2 * KEY_LEN {
        return Err(format!(
            "Der Schlüssel muss aus {} Hex-Zeichen bestehen (gefunden: {}).",
            2 * KEY_LEN,
            digits.len()
        ));
    }
    let mut key = [0u8; KEY_LEN];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&digits[2 * i..2 * i + 2], 16)
            .map_err(|_| "Der Schlüssel enthält Zeichen, die keine Hex-Ziffern sind.".to_string())?;
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_key() {
        let k = parse_key_hex(&"ab".repeat(32)).unwrap();
        assert_eq!(k, [0xAB; 32]);
        assert!(parse_key_hex("abcd").is_err());
        assert!(parse_key_hex(&"zz".repeat(32)).is_err());
        let spaced = "00112233 44556677\n".repeat(4);
        assert!(parse_key_hex(&spaced).is_ok());
    }

    #[test]
    fn seal_open_roundtrip() {
        let key = PacketKey::new(&[7; 32]);
        let pkt = key.seal(Direction::AppToBoard, PacketType::Ping, 42, &[1; 12]);
        assert_eq!(pkt.len(), 38);
        let (h, payload) = key.open(Direction::AppToBoard, &pkt).unwrap();
        assert_eq!(h.counter, 42);
        assert_eq!(payload, vec![1; 12]);
        assert_eq!(key.open(Direction::BoardToApp, &pkt), Err(OpenError::Auth));
    }
}
