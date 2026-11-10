// Persistent board settings: two flash sectors written alternately, each
// copy carrying a sequence number and a CRC. Loading picks the newest valid
// copy, so an interrupted write never loses the previous settings.
#ifndef PSHB_SETTINGS_H
#define PSHB_SETTINGS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define PSHB_SETTINGS_MAGIC   0x42485350u  // "PSHB"
#define PSHB_SETTINGS_VERSION 1
#define PSHB_SETTINGS_SECTOR  4096u
#define PSHB_SETTINGS_SIZE    256u          // one flash page

enum { PSHB_IP_DHCP = 0, PSHB_IP_STATIC = 1 };

typedef struct {
    uint32_t magic;
    uint16_t version;
    uint16_t size;
    uint32_t seq;

    uint8_t ip_mode;           // PSHB_IP_DHCP / PSHB_IP_STATIC
    uint8_t ip[4], mask[4], gw[4], dns[4];
    uint8_t psk_set;
    uint16_t udp_port;
    uint8_t psk[32];
    uint8_t allowed_ip[4];     // 0.0.0.0 = any sender
    uint8_t pw_set;            // 0 = factory default password, must be changed
    uint8_t pw_salt[16];
    uint8_t pw_hash[32];

    uint8_t reserved[PSHB_SETTINGS_SIZE - 4 - 2 - 2 - 4 - 1 - 16 - 1 - 2 - 32 - 4 - 1 - 16 - 32 - 4];
    uint32_t crc;              // CRC-32 of all bytes before this field
} pshb_settings_t;

_Static_assert(sizeof(pshb_settings_t) == PSHB_SETTINGS_SIZE, "settings must be one flash page");

// Flash access, offsets relative to the settings area (2 sectors).
typedef struct {
    void *ctx;
    void (*read)(void *ctx, uint32_t offset, void *buf, size_t len);
    bool (*erase_sector)(void *ctx, uint32_t offset);
    bool (*program)(void *ctx, uint32_t offset, const void *data, size_t len);
} pshb_flash_t;

void settings_defaults(pshb_settings_t *s);

// Load the newest valid copy. Returns false (and defaults) if there is none.
bool settings_load(const pshb_flash_t *f, pshb_settings_t *out);

// Store `s` (seq is incremented, crc filled in) into the older slot and
// verify it by reading back. The other slot keeps the previous version.
bool settings_save(const pshb_flash_t *f, pshb_settings_t *s);

// Erase both copies (factory reset).
bool settings_erase(const pshb_flash_t *f);

bool settings_valid(const pshb_settings_t *s);

#endif
