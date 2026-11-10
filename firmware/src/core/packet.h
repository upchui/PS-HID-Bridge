// Encoding/decoding of the plaintext payloads (independent of encryption).
#ifndef PSHB_PACKET_H
#define PSHB_PACKET_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "protocol.h"

typedef struct {
    uint8_t version;
    uint8_t type;
    uint64_t counter;
} pshb_header_t;

typedef struct {
    uint64_t session_id;
    uint8_t flags;
    uint8_t mods;
    uint8_t keys[6];
    uint8_t buttons;
    int16_t dx, dy;
    int8_t wheel, pan;
} pshb_input_t;

typedef struct {
    uint64_t t_app_us;
    uint32_t seq;
} pshb_ping_t;

typedef struct {
    uint64_t t_app_us;
    uint32_t seq;
    uint64_t session_id;
    uint32_t fw_version;
    uint32_t uptime_ms;
    uint8_t status;
    uint32_t rx_ok, rx_auth_fail, rx_replay, rx_filtered, rx_malformed;
} pshb_pong_t;

void pshb_header_encode(uint8_t out[PSHB_HDR_LEN], const pshb_header_t *h);
void pshb_header_decode(const uint8_t in[PSHB_HDR_LEN], pshb_header_t *h);

void pshb_input_encode(uint8_t out[PSHB_INPUT_PAYLOAD_LEN], const pshb_input_t *in);
void pshb_input_decode(const uint8_t in[PSHB_INPUT_PAYLOAD_LEN], pshb_input_t *out);

void pshb_ping_encode(uint8_t out[PSHB_PING_PAYLOAD_LEN], const pshb_ping_t *p);
void pshb_ping_decode(const uint8_t in[PSHB_PING_PAYLOAD_LEN], pshb_ping_t *p);

void pshb_pong_encode(uint8_t out[PSHB_PONG_PAYLOAD_LEN], const pshb_pong_t *p);
void pshb_pong_decode(const uint8_t in[PSHB_PONG_PAYLOAD_LEN], pshb_pong_t *p);

// Expected payload length for a packet type (0 for unknown types).
size_t pshb_payload_len(uint8_t type);

// Nonce for the given direction and counter.
void pshb_nonce(uint8_t out[PSHB_NONCE_LEN], uint32_t direction, uint64_t counter);

#endif
