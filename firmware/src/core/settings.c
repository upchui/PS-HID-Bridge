#include "settings.h"

#include <string.h>

#include "crc32.h"
#include "protocol.h"

static uint32_t settings_crc(const pshb_settings_t *s) {
    return crc32(s, offsetof(pshb_settings_t, crc));
}

void settings_defaults(pshb_settings_t *s) {
    memset(s, 0, sizeof(*s));
    s->magic = PSHB_SETTINGS_MAGIC;
    s->version = PSHB_SETTINGS_VERSION;
    s->size = sizeof(*s);
    s->ip_mode = PSHB_IP_DHCP;
    s->udp_port = PSHB_UDP_PORT_DEFAULT;
}

bool settings_valid(const pshb_settings_t *s) {
    return s->magic == PSHB_SETTINGS_MAGIC && s->version == PSHB_SETTINGS_VERSION &&
           s->size == sizeof(*s) && s->crc == settings_crc(s) && s->udp_port != 0;
}

uint32_t settings_prefix_mask(unsigned prefix) {
    if (prefix == 0) return 0;
    if (prefix >= 32) return 0xFFFFFFFFu;
    return 0xFFFFFFFFu << (32 - prefix);
}

void settings_sender_filter(const pshb_settings_t *s, uint32_t *net, uint32_t *mask) {
    uint32_t ip = ((uint32_t)s->allowed_ip[0] << 24) | ((uint32_t)s->allowed_ip[1] << 16) |
                  ((uint32_t)s->allowed_ip[2] << 8) | s->allowed_ip[3];
    if (ip == 0 || s->allowed_host_bits >= 32) {
        *net = 0;
        *mask = 0;
        return;
    }
    *mask = settings_prefix_mask(32u - s->allowed_host_bits);
    *net = ip & *mask;
}

// Newer in sequence-number order (wrap-safe).
static bool newer(uint32_t a, uint32_t b) {
    return (int32_t)(a - b) > 0;
}

// Index of the newest valid slot, or -1.
static int newest(const pshb_flash_t *f, pshb_settings_t copies[2]) {
    bool ok[2];
    for (int i = 0; i < 2; i++) {
        f->read(f->ctx, (uint32_t)i * PSHB_SETTINGS_SECTOR, &copies[i], sizeof(copies[i]));
        ok[i] = settings_valid(&copies[i]);
    }
    if (ok[0] && ok[1]) return newer(copies[1].seq, copies[0].seq) ? 1 : 0;
    if (ok[0]) return 0;
    if (ok[1]) return 1;
    return -1;
}

bool settings_load(const pshb_flash_t *f, pshb_settings_t *out) {
    pshb_settings_t copies[2];
    int i = newest(f, copies);
    if (i < 0) {
        settings_defaults(out);
        return false;
    }
    *out = copies[i];
    return true;
}

bool settings_save(const pshb_flash_t *f, pshb_settings_t *s) {
    pshb_settings_t copies[2];
    int cur = newest(f, copies);
    int target = cur == 0 ? 1 : 0;
    s->magic = PSHB_SETTINGS_MAGIC;
    s->version = PSHB_SETTINGS_VERSION;
    s->size = sizeof(*s);
    s->seq = cur < 0 ? 1 : copies[cur].seq + 1;
    s->crc = settings_crc(s);

    uint32_t off = (uint32_t)target * PSHB_SETTINGS_SECTOR;
    if (!f->erase_sector(f->ctx, off)) return false;
    if (!f->program(f->ctx, off, s, sizeof(*s))) return false;

    pshb_settings_t check;
    f->read(f->ctx, off, &check, sizeof(check));
    return memcmp(&check, s, sizeof(check)) == 0;
}

bool settings_erase(const pshb_flash_t *f) {
    return f->erase_sector(f->ctx, 0) && f->erase_sector(f->ctx, PSHB_SETTINGS_SECTOR);
}
