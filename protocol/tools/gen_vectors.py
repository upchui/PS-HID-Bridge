#!/usr/bin/env python3
"""Generate protocol/vectors/v1.json (deterministic).

Usage: gen_vectors.py [OUTPUT]      (default: protocol/vectors/v1.json)
       gen_vectors.py --check       (fail if the committed file is out of date)
"""

from __future__ import annotations

import hashlib
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pshb_proto as p  # noqa: E402

DEFAULT_OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "vectors", "v1.json")

KEY = hashlib.sha256(b"ps-hid-bridge test key 1").digest()
OTHER_KEY = hashlib.sha256(b"ps-hid-bridge test key 2").digest()
SESSION = 0x1122334455667788
APP_COUNTER = 1_762_790_400_000_000  # 2025-11-10 16:00 UTC in microseconds
BOARD_COUNTER = 0x2468ACE013579BDF & ((1 << 62) - 1)


def rfc8439() -> dict:
    # RFC 8439, section 2.8.2
    key = bytes(range(0x80, 0xA0))
    nonce = bytes.fromhex("070000004041424344454647")
    aad = bytes.fromhex("50515253c0c1c2c3c4c5c6c7")
    pt = (b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip "
          b"for the future, sunscreen would be it.")
    out = p._aead(key).encrypt(nonce, pt, aad)
    ct, tag = out[:-16], out[-16:]
    assert tag.hex() == "1ae10b594f09e26a7e902ecbd0600691", "cryptography does not match RFC 8439"
    return {"key": key.hex(), "nonce": nonce.hex(), "aad": aad.hex(), "plaintext": pt.hex(),
            "ciphertext": ct.hex(), "tag": tag.hex()}


def packet(name, direction, ptype, counter, payload: bytes, fields: dict) -> dict:
    return {
        "name": name,
        "direction": direction,
        "type": ptype,
        "counter": counter,
        "key": KEY.hex(),
        "payload": payload.hex(),
        "packet": p.seal(KEY, direction, ptype, counter, payload).hex(),
        "fields": fields,
    }


def input_fields(i: p.Input) -> dict:
    return {"session_id": i.session_id, "flags": i.flags, "mods": i.mods, "keys": list(i.keys),
            "buttons": i.buttons, "dx": i.dx, "dy": i.dy, "wheel": i.wheel, "pan": i.pan}


def packets() -> list[dict]:
    out = []
    typing = p.Input(SESSION, 0, 0x02, [0x04, 0x1D, 0, 0, 0, 0], 0x01, 300, -42, 1, -1)
    out.append(packet("input_keys_and_motion", p.DIR_APP_TO_BOARD, p.TYPE_INPUT, APP_COUNTER,
                      typing.encode(), input_fields(typing)))
    extremes = p.Input(SESSION, 0, 0xFF, [0x01] * 6, 0x1F, 32767, -32768, -128, 127)
    out.append(packet("input_extremes_rollover", p.DIR_APP_TO_BOARD, p.TYPE_INPUT, APP_COUNTER + 1,
                      extremes.encode(), input_fields(extremes)))
    release = p.Input(SESSION, p.FLAG_RELEASE_ALL)
    out.append(packet("input_release_all", p.DIR_APP_TO_BOARD, p.TYPE_INPUT, APP_COUNTER + 2,
                      release.encode(), input_fields(release)))
    hb = p.Input(SESSION, 0, 0x01, [0x16, 0, 0, 0, 0, 0], 0x02, 0, 0, 0, 0)
    out.append(packet("heartbeat", p.DIR_APP_TO_BOARD, p.TYPE_HEARTBEAT, APP_COUNTER + 3,
                      hb.encode(), input_fields(hb)))
    ping = p.Ping(123_456_789_012, 7)
    out.append(packet("ping", p.DIR_APP_TO_BOARD, p.TYPE_PING, APP_COUNTER + 4, ping.encode(),
                      {"t_app_us": ping.t_app_us, "seq": ping.seq}))
    pong = p.Pong(123_456_789_012, 7, SESSION, 0x000100, 61_000, p.STATUS_USB_MOUNTED,
                  1000, 2, 3, 4, 5)
    out.append(packet("pong", p.DIR_BOARD_TO_APP, p.TYPE_PONG, BOARD_COUNTER, pong.encode(),
                      dict(vars(pong))))
    return out


def invalid(valid: list[dict]) -> list[dict]:
    base = bytes.fromhex(valid[0]["packet"])
    pong = bytes.fromhex(valid[-1]["packet"])

    def flip(data: bytes, index: int) -> bytes:
        b = bytearray(data)
        b[index] ^= 0x01
        return bytes(b)

    cases = [
        ("bad_tag", p.DIR_APP_TO_BOARD, KEY, flip(base, len(base) - 1), "auth"),
        ("bad_ciphertext", p.DIR_APP_TO_BOARD, KEY, flip(base, 12), "auth"),
        ("modified_counter", p.DIR_APP_TO_BOARD, KEY, flip(base, 2), "auth"),
        ("modified_type", p.DIR_APP_TO_BOARD, KEY, bytes([base[0], p.TYPE_HEARTBEAT]) + base[2:], "auth"),
        ("wrong_direction", p.DIR_APP_TO_BOARD, KEY,
         p.seal(KEY, p.DIR_BOARD_TO_APP, p.TYPE_INPUT, APP_COUNTER, bytes.fromhex(valid[0]["payload"])), "auth"),
        ("pong_as_app_direction", p.DIR_APP_TO_BOARD, KEY, pong, "auth"),
        ("wrong_key", p.DIR_APP_TO_BOARD, OTHER_KEY, base, "auth"),
        ("truncated", p.DIR_APP_TO_BOARD, KEY, base[:-1], "malformed"),
        ("bad_version", p.DIR_APP_TO_BOARD, KEY, bytes([0x02]) + base[1:], "malformed"),
        ("unknown_type", p.DIR_APP_TO_BOARD, KEY, bytes([base[0], 0x09]) + base[2:], "malformed"),
        ("header_only", p.DIR_APP_TO_BOARD, KEY, base[:10], "malformed"),
    ]
    out = []
    for name, direction, key, pkt, expect in cases:
        # Sanity check against the reference implementation.
        try:
            p.open_packet(key, direction, pkt)
            raise AssertionError(f"{name} unexpectedly valid")
        except p.AuthError:
            assert expect == "auth", name
        except ValueError:
            assert expect == "malformed", name
        out.append({"name": name, "direction": direction, "key": key.hex(), "packet": pkt.hex(),
                    "expect": expect})
    return out


def generate() -> str:
    valid = packets()
    for v in valid:
        ptype, counter, payload = p.open_packet(bytes.fromhex(v["key"]), v["direction"],
                                                bytes.fromhex(v["packet"]))
        assert (ptype, counter, payload.hex()) == (v["type"], v["counter"], v["payload"])
    doc = {
        "description": "PS HID Bridge protocol v1 test vectors, generated by protocol/tools/gen_vectors.py",
        "aead_rfc8439": rfc8439(),
        "packets": valid,
        "invalid": invalid(valid),
    }
    return json.dumps(doc, indent=2) + "\n"


def main() -> int:
    text = generate()
    if len(sys.argv) > 1 and sys.argv[1] == "--check":
        with open(DEFAULT_OUT) as f:
            if f.read() != text:
                print("protocol/vectors/v1.json is out of date: run protocol/tools/gen_vectors.py")
                return 1
        print("protocol/vectors/v1.json is up to date")
        return 0
    out = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_OUT
    with open(out, "w") as f:
        f.write(text)
    print(f"wrote {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
