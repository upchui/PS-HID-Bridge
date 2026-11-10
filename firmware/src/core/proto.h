// Encrypted packet format v1: ChaCha20-Poly1305 (RFC 8439) via Monocypher.
//   header (10 B, associated data) | ciphertext | tag (16 B)
//   nonce = direction u32 LE || counter u64 LE
#ifndef PSHB_PROTO_H
#define PSHB_PROTO_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "packet.h"

// Structural check of a v1 packet: version, known type, exact length.
// Fills *h. Does not touch the key.
bool proto_check(const uint8_t *pkt, size_t len, pshb_header_t *h);

// Verify the tag and decrypt the payload (len - header - tag bytes) into
// `payload_out`. The packet must have passed proto_check(). On failure
// `payload_out` is left untouched.
bool proto_decrypt(const uint8_t key[PSHB_KEY_LEN], uint32_t direction, const uint8_t *pkt,
                   size_t len, uint8_t *payload_out);

// Build a v1 packet. Returns its length, or 0 if `cap` is too small.
size_t proto_seal(const uint8_t key[PSHB_KEY_LEN], uint32_t direction, uint8_t type,
                  uint64_t counter, const uint8_t *payload, size_t payload_len, uint8_t *out,
                  size_t cap);

#endif
