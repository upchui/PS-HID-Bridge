//! Wire format of protocol v1 (see `protocol/SPEC.md`). Pure encoding and
//! decoding; encryption lives in [`crate::crypto`].

pub const VERSION: u8 = 0x01;
pub const HEADER_LEN: usize = 10;
pub const TAG_LEN: usize = 16;
pub const NONCE_LEN: usize = 12;
pub const KEY_LEN: usize = 32;
pub const UDP_PORT_DEFAULT: u16 = 45045;

pub const INPUT_LEN: usize = 23;
pub const PING_LEN: usize = 12;
pub const PONG_LEN: usize = 49;

pub const FLAG_RELEASE_ALL: u8 = 0x01;

pub const STATUS_USB_MOUNTED: u8 = 0x01;
pub const STATUS_USB_SUSPENDED: u8 = 0x02;
pub const STATUS_FAILSAFE: u8 = 0x04;
pub const STATUS_PLAINTEXT: u8 = 0x80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketType {
    Input = 0x01,
    Heartbeat = 0x02,
    Ping = 0x03,
    Pong = 0x04,
}

impl PacketType {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0x01 => Some(Self::Input),
            0x02 => Some(Self::Heartbeat),
            0x03 => Some(Self::Ping),
            0x04 => Some(Self::Pong),
            _ => None,
        }
    }

    pub fn payload_len(self) -> usize {
        match self {
            Self::Input | Self::Heartbeat => INPUT_LEN,
            Self::Ping => PING_LEN,
            Self::Pong => PONG_LEN,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Direction {
    AppToBoard = 1,
    BoardToApp = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u8,
    pub ptype: u8,
    pub counter: u64,
}

impl Header {
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0] = self.version;
        out[1] = self.ptype;
        out[2..10].copy_from_slice(&self.counter.to_le_bytes());
        out
    }

    pub fn decode(b: &[u8]) -> Self {
        Self {
            version: b[0],
            ptype: b[1],
            counter: u64::from_le_bytes(b[2..10].try_into().unwrap()),
        }
    }
}

pub fn nonce(direction: Direction, counter: u64) -> [u8; NONCE_LEN] {
    let mut n = [0u8; NONCE_LEN];
    n[..4].copy_from_slice(&(direction as u32).to_le_bytes());
    n[4..].copy_from_slice(&counter.to_le_bytes());
    n
}

/// INPUT / HEARTBEAT payload.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputPayload {
    pub session_id: u64,
    pub flags: u8,
    pub mods: u8,
    pub keys: [u8; 6],
    pub buttons: u8,
    pub dx: i16,
    pub dy: i16,
    pub wheel: i8,
    pub pan: i8,
}

impl InputPayload {
    pub fn encode(&self) -> [u8; INPUT_LEN] {
        let mut b = [0u8; INPUT_LEN];
        b[0..8].copy_from_slice(&self.session_id.to_le_bytes());
        b[8] = self.flags;
        b[9] = self.mods;
        b[10..16].copy_from_slice(&self.keys);
        b[16] = self.buttons;
        b[17..19].copy_from_slice(&self.dx.to_le_bytes());
        b[19..21].copy_from_slice(&self.dy.to_le_bytes());
        b[21] = self.wheel as u8;
        b[22] = self.pan as u8;
        b
    }

    pub fn decode(b: &[u8]) -> Self {
        Self {
            session_id: u64::from_le_bytes(b[0..8].try_into().unwrap()),
            flags: b[8],
            mods: b[9],
            keys: b[10..16].try_into().unwrap(),
            buttons: b[16],
            dx: i16::from_le_bytes([b[17], b[18]]),
            dy: i16::from_le_bytes([b[19], b[20]]),
            wheel: b[21] as i8,
            pan: b[22] as i8,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ping {
    pub t_app_us: u64,
    pub seq: u32,
}

impl Ping {
    pub fn encode(&self) -> [u8; PING_LEN] {
        let mut b = [0u8; PING_LEN];
        b[0..8].copy_from_slice(&self.t_app_us.to_le_bytes());
        b[8..12].copy_from_slice(&self.seq.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Self {
        Self {
            t_app_us: u64::from_le_bytes(b[0..8].try_into().unwrap()),
            seq: u32::from_le_bytes(b[8..12].try_into().unwrap()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pong {
    pub t_app_us: u64,
    pub seq: u32,
    pub session_id: u64,
    pub fw_version: u32,
    pub uptime_ms: u32,
    pub status: u8,
    pub rx_ok: u32,
    pub rx_auth_fail: u32,
    pub rx_replay: u32,
    pub rx_filtered: u32,
    pub rx_malformed: u32,
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}

impl Pong {
    pub fn encode(&self) -> [u8; PONG_LEN] {
        let mut b = [0u8; PONG_LEN];
        b[0..8].copy_from_slice(&self.t_app_us.to_le_bytes());
        b[8..12].copy_from_slice(&self.seq.to_le_bytes());
        b[12..20].copy_from_slice(&self.session_id.to_le_bytes());
        b[20..24].copy_from_slice(&self.fw_version.to_le_bytes());
        b[24..28].copy_from_slice(&self.uptime_ms.to_le_bytes());
        b[28] = self.status;
        for (i, v) in [
            self.rx_ok,
            self.rx_auth_fail,
            self.rx_replay,
            self.rx_filtered,
            self.rx_malformed,
        ]
        .iter()
        .enumerate()
        {
            b[29 + 4 * i..33 + 4 * i].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    pub fn decode(b: &[u8]) -> Self {
        Self {
            t_app_us: u64::from_le_bytes(b[0..8].try_into().unwrap()),
            seq: u32_at(b, 8),
            session_id: u64::from_le_bytes(b[12..20].try_into().unwrap()),
            fw_version: u32_at(b, 20),
            uptime_ms: u32_at(b, 24),
            status: b[28],
            rx_ok: u32_at(b, 29),
            rx_auth_fail: u32_at(b, 33),
            rx_replay: u32_at(b, 37),
            rx_filtered: u32_at(b, 41),
            rx_malformed: u32_at(b, 45),
        }
    }

    /// "major.minor.patch" from 0xMMmmpppp (patch: commits since the last MAJOR.MINOR).
    pub fn fw_version_string(&self) -> String {
        let v = self.fw_version;
        format!("{}.{}.{}", v >> 24, (v >> 16) & 0xFF, v & 0xFFFF)
    }

    /// A PONG with seq = t_app_us = 0 is the board's answer to an input
    /// packet with a stale session id, not to a ping.
    pub fn is_session_hint(&self) -> bool {
        self.seq == 0 && self.t_app_us == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_roundtrip_and_layout() {
        let p = InputPayload {
            session_id: 0x1122334455667788,
            flags: 0,
            mods: 0x02,
            keys: [0x04, 0x1D, 0, 0, 0, 0],
            buttons: 0x01,
            dx: -2,
            dy: 300,
            wheel: -1,
            pan: 1,
        };
        let b = p.encode();
        assert_eq!(&b[0..8], &[0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11]);
        assert_eq!(&b[17..19], &[0xFE, 0xFF]);
        assert_eq!(InputPayload::decode(&b), p);
    }

    #[test]
    fn nonce_layout() {
        let n = nonce(Direction::BoardToApp, 0x0102030405060708);
        assert_eq!(n, [2, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn pong_roundtrip() {
        let p = Pong {
            t_app_us: 1,
            seq: 2,
            session_id: 3,
            fw_version: 0x0109_0123,
            uptime_ms: 5,
            status: STATUS_USB_MOUNTED,
            rx_ok: 7,
            rx_auth_fail: 8,
            rx_replay: 9,
            rx_filtered: 10,
            rx_malformed: 11,
        };
        assert_eq!(Pong::decode(&p.encode()), p);
        assert_eq!(p.fw_version_string(), "1.9.291");
    }
}
