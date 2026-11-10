"""Reference implementation of the PS5 HID Bridge wire protocol.

See protocol/SPEC.md. Used by tools/udp_test.py, the simulator integration
tests and the test vector generator. Encryption needs the `cryptography`
package (python3-cryptography); the plaintext debug format works without it.
"""

from __future__ import annotations

import struct
from dataclasses import dataclass, field

VERSION = 0x01
VERSION_PLAIN = 0x00

TYPE_INPUT = 0x01
TYPE_HEARTBEAT = 0x02
TYPE_PING = 0x03
TYPE_PONG = 0x04

DIR_APP_TO_BOARD = 1
DIR_BOARD_TO_APP = 2

FLAG_RELEASE_ALL = 0x01

STATUS_USB_MOUNTED = 0x01
STATUS_USB_SUSPENDED = 0x02
STATUS_FAILSAFE = 0x04
STATUS_PLAINTEXT = 0x80

UDP_PORT_DEFAULT = 45045
KEY_LEN = 32
TAG_LEN = 16

HEADER = struct.Struct("<BBQ")            # version, type, counter
INPUT = struct.Struct("<QBB6sBhhbb")      # 23 bytes
PING = struct.Struct("<QI")               # 12 bytes
PONG = struct.Struct("<QIQIIBIIIII")      # 49 bytes
NONCE = struct.Struct("<IQ")              # direction, counter

PAYLOAD_LEN = {
    TYPE_INPUT: INPUT.size,
    TYPE_HEARTBEAT: INPUT.size,
    TYPE_PING: PING.size,
    TYPE_PONG: PONG.size,
}


@dataclass
class Input:
    session_id: int = 0
    flags: int = 0
    mods: int = 0
    keys: list[int] = field(default_factory=lambda: [0] * 6)
    buttons: int = 0
    dx: int = 0
    dy: int = 0
    wheel: int = 0
    pan: int = 0

    def encode(self) -> bytes:
        keys = bytes((list(self.keys) + [0] * 6)[:6])
        return INPUT.pack(self.session_id, self.flags, self.mods, keys, self.buttons,
                          self.dx, self.dy, self.wheel, self.pan)

    @classmethod
    def decode(cls, data: bytes) -> "Input":
        sid, flags, mods, keys, buttons, dx, dy, wheel, pan = INPUT.unpack(data)
        return cls(sid, flags, mods, list(keys), buttons, dx, dy, wheel, pan)


@dataclass
class Ping:
    t_app_us: int
    seq: int

    def encode(self) -> bytes:
        return PING.pack(self.t_app_us, self.seq)


@dataclass
class Pong:
    t_app_us: int
    seq: int
    session_id: int
    fw_version: int
    uptime_ms: int
    status: int
    rx_ok: int
    rx_auth_fail: int
    rx_replay: int
    rx_filtered: int
    rx_malformed: int

    @classmethod
    def decode(cls, data: bytes) -> "Pong":
        return cls(*PONG.unpack(data))

    def encode(self) -> bytes:
        return PONG.pack(self.t_app_us, self.seq, self.session_id, self.fw_version,
                         self.uptime_ms, self.status, self.rx_ok, self.rx_auth_fail,
                         self.rx_replay, self.rx_filtered, self.rx_malformed)

    @property
    def fw_version_str(self) -> str:
        v = self.fw_version
        return f"{(v >> 16) & 0xFF}.{(v >> 8) & 0xFF}.{v & 0xFF}"


def header(version: int, ptype: int, counter: int) -> bytes:
    return HEADER.pack(version, ptype, counter)


def nonce(direction: int, counter: int) -> bytes:
    return NONCE.pack(direction, counter)


# ---------------------------------------------------------------- plaintext

def plain_packet(ptype: int, counter: int, payload: bytes) -> bytes:
    """Unencrypted debug packet (version 0), accepted only by debug builds."""
    return header(VERSION_PLAIN, ptype, counter) + payload


def parse_plain(packet: bytes) -> tuple[int, int, bytes]:
    """Returns (type, counter, payload) of a version-0 packet."""
    if len(packet) < HEADER.size:
        raise ValueError("packet too short")
    version, ptype, counter = HEADER.unpack_from(packet)
    if version != VERSION_PLAIN:
        raise ValueError(f"not a plaintext packet (version {version})")
    payload = packet[HEADER.size:]
    if len(payload) != PAYLOAD_LEN.get(ptype, -1):
        raise ValueError("bad payload length")
    return ptype, counter, payload


# ---------------------------------------------------------------- encrypted (v1)

class AuthError(ValueError):
    """Tag verification failed (wrong key, direction or tampered packet)."""


def _aead(key: bytes):
    from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
    if len(key) != KEY_LEN:
        raise ValueError("key must be 32 bytes")
    return ChaCha20Poly1305(key)


def seal(key: bytes, direction: int, ptype: int, counter: int, payload: bytes) -> bytes:
    """Build a v1 packet: header (AAD) | ciphertext | tag."""
    hdr = header(VERSION, ptype, counter)
    return hdr + _aead(key).encrypt(nonce(direction, counter), payload, hdr)


def check(packet: bytes) -> tuple[int, int]:
    """Structural check of a v1 packet. Returns (type, counter) or raises ValueError."""
    if len(packet) < HEADER.size + TAG_LEN:
        raise ValueError("packet too short")
    version, ptype, counter = HEADER.unpack_from(packet)
    if version != VERSION:
        raise ValueError(f"unsupported version {version}")
    if ptype not in PAYLOAD_LEN:
        raise ValueError(f"unknown type {ptype}")
    if len(packet) != HEADER.size + PAYLOAD_LEN[ptype] + TAG_LEN:
        raise ValueError("bad length")
    return ptype, counter


def open_packet(key: bytes, direction: int, packet: bytes) -> tuple[int, int, bytes]:
    """Verify and decrypt a v1 packet. Returns (type, counter, payload)."""
    from cryptography.exceptions import InvalidTag
    ptype, counter = check(packet)
    hdr = packet[:HEADER.size]
    try:
        payload = _aead(key).decrypt(nonce(direction, counter), packet[HEADER.size:], hdr)
    except InvalidTag as e:
        raise AuthError("authentication failed") from e
    return ptype, counter, payload
