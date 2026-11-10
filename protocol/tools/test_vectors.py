#!/usr/bin/env python3
"""Check the Python reference implementation against protocol/vectors/v1.json."""

from __future__ import annotations

import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import pshb_proto as p  # noqa: E402


def main() -> int:
    with open(os.path.join(HERE, "..", "vectors", "v1.json")) as f:
        doc = json.load(f)

    r = doc["aead_rfc8439"]
    ct = p._aead(bytes.fromhex(r["key"])).encrypt(bytes.fromhex(r["nonce"]), bytes.fromhex(r["plaintext"]),
                                                   bytes.fromhex(r["aad"]))
    assert ct.hex() == r["ciphertext"] + r["tag"], "RFC 8439 vector"

    for v in doc["packets"]:
        key = bytes.fromhex(v["key"])
        payload = bytes.fromhex(v["payload"])
        assert p.seal(key, v["direction"], v["type"], v["counter"], payload).hex() == v["packet"], v["name"]
        ptype, counter, got = p.open_packet(key, v["direction"], bytes.fromhex(v["packet"]))
        assert (ptype, counter, got) == (v["type"], v["counter"], payload), v["name"]
        f = v["fields"]
        if ptype in (p.TYPE_INPUT, p.TYPE_HEARTBEAT):
            i = p.Input.decode(payload)
            assert vars(i) == f, v["name"]
            assert i.encode() == payload
        elif ptype == p.TYPE_PONG:
            assert vars(p.Pong.decode(payload)) == f, v["name"]

    for v in doc["invalid"]:
        try:
            p.open_packet(bytes.fromhex(v["key"]), v["direction"], bytes.fromhex(v["packet"]))
        except p.AuthError:
            assert v["expect"] == "auth", v["name"]
        except ValueError:
            assert v["expect"] == "malformed", v["name"]
        else:
            raise AssertionError(f"{v['name']} accepted")

    print(f"python: {len(doc['packets'])} packets, {len(doc['invalid'])} invalid vectors ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
