// Wire protocol constants (see protocol/SPEC.md) and little-endian helpers.
#ifndef PSHB_PROTOCOL_H
#define PSHB_PROTOCOL_H

#include <stdint.h>

#define PSHB_UDP_PORT_DEFAULT 45045

// Header (also the AEAD associated data): version u8 | type u8 | counter u64 LE
#define PSHB_PROTO_VERSION       0x01
#define PSHB_PROTO_VERSION_PLAIN 0x00  // debug builds only: no encryption
#define PSHB_HDR_LEN             10
#define PSHB_TAG_LEN             16
#define PSHB_NONCE_LEN           12
#define PSHB_KEY_LEN             32

enum {
    PSHB_TYPE_INPUT = 0x01,
    PSHB_TYPE_HEARTBEAT = 0x02,
    PSHB_TYPE_PING = 0x03,
    PSHB_TYPE_PONG = 0x04,
};

// Nonce = direction u32 LE || counter u64 LE
enum {
    PSHB_DIR_APP_TO_BOARD = 1,
    PSHB_DIR_BOARD_TO_APP = 2,
};

// INPUT / HEARTBEAT payload (23 bytes)
//   0 session_id u64 | 8 flags u8 | 9 mods u8 | 10 keys[6] | 16 buttons u8
//  17 dx i16 | 19 dy i16 | 21 wheel i8 | 22 pan i8
#define PSHB_INPUT_PAYLOAD_LEN 23
#define PSHB_FLAG_RELEASE_ALL  0x01

// PING payload (12 bytes): t_app_us u64 | seq u32
#define PSHB_PING_PAYLOAD_LEN 12

// PONG payload (49 bytes)
//   0 t_app_us u64 | 8 seq u32 | 12 session_id u64 | 20 fw_version u32
//  24 uptime_ms u32 | 28 status u8 | 29 rx_ok u32 | 33 rx_auth_fail u32
//  37 rx_replay u32 | 41 rx_filtered u32 | 45 rx_malformed u32
#define PSHB_PONG_PAYLOAD_LEN 49
enum {
    PSHB_STATUS_USB_MOUNTED = 0x01,
    PSHB_STATUS_USB_SUSPENDED = 0x02,
    PSHB_STATUS_FAILSAFE = 0x04,  // no valid input recently: everything released
    PSHB_STATUS_NO_PARTITION_TABLE = 0x08,  // updates overwrite the firmware, no fallback slot
    PSHB_STATUS_PLAINTEXT = 0x80,  // debug build without encryption
};

#define PSHB_MAX_PACKET_LEN (PSHB_HDR_LEN + PSHB_PONG_PAYLOAD_LEN + PSHB_TAG_LEN)

static inline uint16_t pshb_rd16(const uint8_t *p) {
    return (uint16_t)(p[0] | (p[1] << 8));
}
static inline uint32_t pshb_rd32(const uint8_t *p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}
static inline uint64_t pshb_rd64(const uint8_t *p) {
    return (uint64_t)pshb_rd32(p) | ((uint64_t)pshb_rd32(p + 4) << 32);
}
static inline void pshb_wr16(uint8_t *p, uint16_t v) {
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
}
static inline void pshb_wr32(uint8_t *p, uint32_t v) {
    for (int i = 0; i < 4; i++) p[i] = (uint8_t)(v >> (8 * i));
}
static inline void pshb_wr64(uint8_t *p, uint64_t v) {
    for (int i = 0; i < 8; i++) p[i] = (uint8_t)(v >> (8 * i));
}

#endif
