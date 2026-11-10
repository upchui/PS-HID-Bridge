# PS5 HID Bridge – Protocol v1

This document describes the UDP packets between the desktop app and the board. There are three implementations:

- firmware in C (`firmware/src/core/`)
- desktop core in Rust (`desktop/core/src/protocol.rs`)
- Python reference (`protocol/tools/pshb_proto.py`)

All three are checked against the shared test vectors in `protocol/vectors/`.

## Overview

- Transport: UDP; the board listens on port **45045** (configurable).
- Every packet is encrypted and authenticated with **ChaCha20-Poly1305** (RFC 8439) and a **pre-shared key** (256 bit).
- All numbers are **little endian**.
- The board silently drops packets with a bad tag, an old counter, a wrong session or a foreign sender IP, and only counts them.

## Packet layout

| Offset | Size | Field | Content |
|---|---|---|---|
| 0 | 1 | version | `0x01` |
| 1 | 1 | type | see below |
| 2 | 8 | counter | u64, sender's sequence number |
| 10 | n | payload | encrypted, length depends on the type |
| 10+n | 16 | tag | Poly1305 |

The first 10 bytes (version, type, counter) are not encrypted but enter the tag as **associated data**, so they cannot be changed without the check failing.

| Type | Name | Direction | Payload | Packet length |
|---|---|---|---|---|
| `0x01` | INPUT | app → board | 23 B | 49 B |
| `0x02` | HEARTBEAT | app → board | 23 B | 49 B |
| `0x03` | PING | app → board | 12 B | 38 B |
| `0x04` | PONG | board → app | 49 B | 75 B |

### Nonce

The 12-byte nonce is built from the direction and the counter:

| Offset | Size | Content |
|---|---|---|
| 0 | 4 | direction, u32: `1` = app → board, `2` = board → app |
| 4 | 8 | counter from the header, u64 |

Both directions use the same key. Because the direction is part of the nonce, app and board nonces can never collide.

**A nonce must never be used twice with the same key.** Therefore:

- **App:** counter = `max(last + 1, current Unix time in µs, stored floor)`. The floor is regularly set to `counter + 60 s` and saved, so the counter stays unique across app restarts and clocks that are set back.
- **Board:** the counter starts at a random value from the TRNG on every boot (limited to 62 bits) and counts up.

## Payloads

### INPUT and HEARTBEAT (23 bytes)

| Offset | Size | Field | Content |
|---|---|---|---|
| 0 | 8 | session_id | u64, the board's session (from PONG) |
| 8 | 1 | flags | bit 0 = RELEASE_ALL: release everything, ignore the other fields |
| 9 | 1 | mods | modifier bits as in the HID keyboard report (bit 0 = left Ctrl … bit 7 = right GUI) |
| 10 | 6 | keys | up to 6 pressed keys, HID usage page 0x07, unused = 0; with more than 6 keys all six = `0x01` (ErrorRollOver) |
| 16 | 1 | buttons | mouse buttons: bit 0 left, 1 right, 2 middle, 3 back, 4 forward |
| 17 | 2 | dx | i16, relative mouse motion |
| 19 | 2 | dy | i16 |
| 21 | 1 | wheel | i8, vertical wheel |
| 22 | 1 | pan | i8, horizontal wheel |

- Keyboard and mouse buttons are **absolute state**: every packet contains all keys currently pressed, so a lost packet is corrected by the next one. Every distinct state is sent (and reported over USB) in order, so even a key tap shorter than a millisecond arrives.
- Motion and wheels are **relative**: the board adds them up and passes them on through the USB reports without losing anything.
- HEARTBEAT has the same layout. The app sends it while the bridge is active and nothing else was sent for 50 ms. It carries the full key state and `dx = dy = wheel = pan = 0`.

### PING (12 bytes)

| Offset | Size | Field | Content |
|---|---|---|---|
| 0 | 8 | t_app_us | u64, app's send time (any monotonic clock) |
| 8 | 4 | seq | u32, sequence number |

PING does not need a session id; the app learns the session from the answer.

### PONG (49 bytes)

| Offset | Size | Field | Content |
|---|---|---|---|
| 0 | 8 | t_app_us | echo from PING (0 for a session hint, see below) |
| 8 | 4 | seq | echo from PING |
| 12 | 8 | session_id | the board's current session |
| 20 | 4 | fw_version | `0x00MMmmpp` (major, minor, patch) |
| 24 | 4 | uptime_ms | time since boot |
| 28 | 1 | status | bit 0 USB connected, bit 1 USB suspended, bit 2 failsafe active, bit 7 unencrypted debug firmware |
| 29 | 4 | rx_ok | valid packets |
| 33 | 4 | rx_auth_fail | bad tag or no key set |
| 37 | 4 | rx_replay | counter not greater than the last one, or wrong session |
| 41 | 4 | rx_filtered | foreign sender IP |
| 45 | 4 | rx_malformed | wrong length, version or unknown type |

## Processing on the board

For every received packet the board checks, in this order:

1. Length at least header + tag, version `0x01`, type INPUT, HEARTBEAT or PING, length matches the type. Otherwise `rx_malformed`.
2. Sender IP allowed (if a filter is configured). Otherwise `rx_filtered`.
3. `counter > last valid counter`. Otherwise `rx_replay`. This check is cheap and therefore comes before the cryptography.
4. Verify the tag and decrypt. Otherwise `rx_auth_fail`.
5. Only now: `last valid counter = counter`. A forged packet can therefore not push the counter up.
6. INPUT/HEARTBEAT: `session_id` must match the current session. Otherwise `rx_replay`; at most every 100 ms the board answers with a PONG with `t_app_us = seq = 0` as a **session hint**, so the app learns the new session immediately.
7. PING: PONG to the sender (IP and port of the received packet).

The board creates a new **session id** from the TRNG on every boot. After a restart every previously recorded input packet is worthless, even though the board's own counter starts over.

## Failsafe

- If no valid INPUT or HEARTBEAT arrives for **250 ms**, the board releases all keys and mouse buttons (status bit 2).
- The same happens when the PS5 disconnects the USB port or puts it into rest mode.
- The next valid packet restores the state.

## Behaviour of the app

1. Send PING until a PONG arrives; take the `session_id` from it.
2. Send an INPUT on every change, at most one per millisecond. Motion within that millisecond is summed up; whatever does not fit into i16 goes into the next packet.
3. While the bridge is active: HEARTBEAT if nothing was sent for 50 ms.
4. PING every second (more often for the latency view). If the `session_id` in a PONG changes, use the new one from now on and send the full state again.
5. When deactivating: INPUT with `RELEASE_ALL`.

## Unencrypted debug mode (version 0)

Only the debug image `ps5-hid-bridge-plain.uf2` and the simulator with `--plaintext` accept packets with version `0x00`. They have the same header and payloads, but no encryption and no tag. Counter, session and sender filter are not checked in this mode. Answers are also sent as version 0. Status bit 7 shows this mode.

## Test vectors

`protocol/vectors/v1.json` is generated by `protocol/tools/gen_vectors.py` with fixed keys and counters:

- `aead_rfc8439`: the test vector from RFC 8439, section 2.8.2. It proves that the libraries implement the standard construction.
- `packets`: valid packets of all types and directions, with the decoded fields.
- `invalid`: packets that must fail to open: wrong tag, wrong key, wrong direction, modified header, too short.

`make test` checks that the committed file matches the generator and that C, Rust and Python process all vectors identically.
