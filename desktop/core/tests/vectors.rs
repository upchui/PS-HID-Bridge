//! The Rust implementation against the shared vectors (protocol/vectors/v1.json).

use chacha20poly1305::aead::{AeadInOut, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use pshb_core::crypto::{OpenError, PacketKey};
use pshb_core::protocol::{Direction, InputPayload, PacketType, Pong};
use serde_json::Value;

const VECTORS: &str = include_str!("../../../protocol/vectors/v1.json");

fn hex(v: &Value) -> Vec<u8> {
    let s = v.as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn key(v: &Value) -> [u8; 32] {
    hex(v).try_into().unwrap()
}

fn direction(v: &Value) -> Direction {
    match v.as_u64().unwrap() {
        1 => Direction::AppToBoard,
        2 => Direction::BoardToApp,
        d => panic!("direction {d}"),
    }
}

fn doc() -> Value {
    serde_json::from_str(VECTORS).unwrap()
}

#[test]
fn rfc8439() {
    let d = doc();
    let r = &d["aead_rfc8439"];
    let aead = ChaCha20Poly1305::new(&key(&r["key"]).into());
    let nonce: [u8; 12] = hex(&r["nonce"]).try_into().unwrap();
    let mut buf = hex(&r["plaintext"]);
    let tag = aead
        .encrypt_inout_detached(&Nonce::from(nonce), &hex(&r["aad"]), (&mut buf[..]).into())
        .unwrap();
    assert_eq!(buf, hex(&r["ciphertext"]));
    assert_eq!(tag.to_vec(), hex(&r["tag"]));
}

#[test]
fn packets() {
    let d = doc();
    for v in d["packets"].as_array().unwrap() {
        let name = v["name"].as_str().unwrap();
        let k = PacketKey::new(&key(&v["key"]));
        let dir = direction(&v["direction"]);
        let ptype = PacketType::from_u8(v["type"].as_u64().unwrap() as u8).unwrap();
        let counter = v["counter"].as_u64().unwrap();
        let payload = hex(&v["payload"]);
        let packet = hex(&v["packet"]);
        assert_eq!(k.seal(dir, ptype, counter, &payload), packet, "{name}: seal");
        let (h, got) = k.open(dir, &packet).unwrap();
        assert_eq!(
            (h.ptype, h.counter, &got),
            (ptype as u8, counter, &payload),
            "{name}: open"
        );

        let f = &v["fields"];
        match ptype {
            PacketType::Input | PacketType::Heartbeat => {
                let i = InputPayload::decode(&payload);
                assert_eq!(i.session_id, f["session_id"].as_u64().unwrap(), "{name}");
                assert_eq!(i.flags as u64, f["flags"].as_u64().unwrap());
                assert_eq!(i.mods as u64, f["mods"].as_u64().unwrap());
                let keys: Vec<u8> = f["keys"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|k| k.as_u64().unwrap() as u8)
                    .collect();
                assert_eq!(i.keys.to_vec(), keys);
                assert_eq!(i.buttons as u64, f["buttons"].as_u64().unwrap());
                assert_eq!(i.dx as i64, f["dx"].as_i64().unwrap());
                assert_eq!(i.dy as i64, f["dy"].as_i64().unwrap());
                assert_eq!(i.wheel as i64, f["wheel"].as_i64().unwrap());
                assert_eq!(i.pan as i64, f["pan"].as_i64().unwrap());
                assert_eq!(i.encode().to_vec(), payload);
            }
            PacketType::Pong => {
                let p = Pong::decode(&payload);
                assert_eq!(p.session_id, f["session_id"].as_u64().unwrap());
                assert_eq!(p.rx_malformed as u64, f["rx_malformed"].as_u64().unwrap());
                assert_eq!(p.encode().to_vec(), payload);
            }
            PacketType::Ping => {}
        }
    }
}

#[test]
fn invalid() {
    let d = doc();
    for v in d["invalid"].as_array().unwrap() {
        let name = v["name"].as_str().unwrap();
        let k = PacketKey::new(&key(&v["key"]));
        let res = k.open(direction(&v["direction"]), &hex(&v["packet"]));
        match v["expect"].as_str().unwrap() {
            "auth" => assert_eq!(res, Err(OpenError::Auth), "{name}"),
            "malformed" => assert!(matches!(res, Err(OpenError::Malformed(_))), "{name}: {res:?}"),
            e => panic!("{e}"),
        }
    }
}
