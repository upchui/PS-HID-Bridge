#!/usr/bin/env python3
"""Convert protocol/vectors/v1.json into a C header for the firmware host tests.

Usage: gen_c_vectors.py INPUT.json OUTPUT.h
"""

from __future__ import annotations

import json
import sys


def arr(name: str, hexstr: str) -> str:
    data = bytes.fromhex(hexstr)
    body = ", ".join(f"0x{b:02x}" for b in data) or "0"
    return f"static const uint8_t {name}[] = {{{body}}};\n"


def main() -> int:
    src, dst = sys.argv[1], sys.argv[2]
    with open(src) as f:
        doc = json.load(f)

    out = ["// Generated from protocol/vectors/v1.json by gen_c_vectors.py -- do not edit.\n",
           "#pragma once\n#include <stddef.h>\n#include <stdint.h>\n\n"]

    r = doc["aead_rfc8439"]
    for k in ("key", "nonce", "aad", "plaintext", "ciphertext", "tag"):
        out.append(arr(f"rfc_{k}", r[k]))

    out.append("""
typedef struct {
    const char *name;
    uint32_t direction;
    uint8_t type;
    uint64_t counter;
    const uint8_t *key;
    const uint8_t *payload;
    size_t payload_len;
    const uint8_t *packet;
    size_t packet_len;
    int has_input;
    uint64_t session_id;
    uint8_t flags, mods, keys[6], buttons;
    int16_t dx, dy;
    int8_t wheel, pan;
} vec_packet_t;

typedef struct {
    const char *name;
    uint32_t direction;
    const uint8_t *key;
    const uint8_t *packet;
    size_t packet_len;
    int expect_auth;  /* 1 = structure ok, tag fails; 0 = malformed */
} vec_invalid_t;

""")
    rows = []
    for i, v in enumerate(doc["packets"]):
        out.append(arr(f"p{i}_key", v["key"]))
        out.append(arr(f"p{i}_payload", v["payload"]))
        out.append(arr(f"p{i}_packet", v["packet"]))
        f = v["fields"]
        if v["type"] in (1, 2):
            keys = ", ".join(str(k) for k in f["keys"])
            inp = (f"1, {f['session_id']}ull, {f['flags']}, {f['mods']}, {{{keys}}}, {f['buttons']}, "
                   f"{f['dx']}, {f['dy']}, {f['wheel']}, {f['pan']}")
        else:
            inp = "0, 0, 0, 0, {0}, 0, 0, 0, 0, 0"
        rows.append(f'    {{"{v["name"]}", {v["direction"]}, {v["type"]}, {v["counter"]}ull, p{i}_key, '
                    f'p{i}_payload, sizeof(p{i}_payload), p{i}_packet, sizeof(p{i}_packet), {inp}}},\n')
    out.append("\nstatic const vec_packet_t k_packets[] = {\n" + "".join(rows) + "};\n\n")

    rows = []
    for i, v in enumerate(doc["invalid"]):
        out.append(arr(f"x{i}_key", v["key"]))
        out.append(arr(f"x{i}_packet", v["packet"]))
        rows.append(f'    {{"{v["name"]}", {v["direction"]}, x{i}_key, x{i}_packet, sizeof(x{i}_packet), '
                    f'{1 if v["expect"] == "auth" else 0}}},\n')
    out.append("\nstatic const vec_invalid_t k_invalid[] = {\n" + "".join(rows) + "};\n")

    with open(dst, "w") as f:
        f.write("".join(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
