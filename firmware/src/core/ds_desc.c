#include "ds_desc.h"

#include <stdbool.h>
#include <stddef.h>
#include <string.h>

// Walk the descriptors after the configuration header; NULL at the end or
// at a malformed descriptor.
static const uint8_t *next_desc(const uint8_t *p, const uint8_t *end) {
    if (p + 2 > end || p[0] < 2 || p + p[0] > end) return NULL;
    p += p[0];
    return p + 2 <= end && p[0] >= 2 && p + p[0] <= end ? p : NULL;
}

static const uint8_t *first_desc(const uint8_t *cfg, uint16_t len) {
    if (len < 9 || cfg[1] != DS_DESC_CONFIGURATION || cfg[0] < 9) return NULL;
    const uint8_t *end = cfg + len;
    const uint8_t *p = cfg + cfg[0];
    return p + 2 <= end && p[0] >= 2 && p + p[0] <= end ? p : NULL;
}

uint16_t ds_desc_interface_span(const uint8_t *itf, uint16_t max) {
    if (max < 9 || itf[0] < 9 || itf[1] != DS_DESC_INTERFACE || itf[0] > max) return 0;
    uint16_t off = itf[0];
    while (off < max) {
        const uint8_t *p = itf + off;
        if (max - off < 2 || p[0] < 2 || p[0] > max - off) return 0;  // broken descriptor
        if (p[1] == DS_DESC_IAD || (p[1] == DS_DESC_INTERFACE && p[0] >= 9 && p[2] != itf[2])) return off;
        off = (uint16_t)(off + p[0]);
    }
    return off;
}

const uint8_t *ds_desc_find(const uint8_t *cfg, uint16_t len, uint8_t itf_num, uint8_t type) {
    const uint8_t *end = cfg + len;
    bool in_itf = false;
    for (const uint8_t *p = first_desc(cfg, len); p; p = next_desc(p, end)) {
        if (p[1] == DS_DESC_INTERFACE && p[0] >= 9) {
            in_itf = p[2] == itf_num && p[3] == 0;
            if (in_itf && type == DS_DESC_INTERFACE) return p;
        } else if (in_itf && p[1] == type) {
            return p;
        }
    }
    return NULL;
}

int ds_desc_find_class(const uint8_t *cfg, uint16_t len, uint8_t cls) {
    const uint8_t *end = cfg + len;
    for (const uint8_t *p = first_desc(cfg, len); p; p = next_desc(p, end)) {
        if (p[1] == DS_DESC_INTERFACE && p[0] >= 9 && p[5] == cls) return p[2];
    }
    return -1;
}

uint16_t ds_desc_single_interface(const uint8_t *cfg, uint16_t len, uint8_t itf_num, uint8_t *out, uint16_t cap) {
    const uint8_t *itf = ds_desc_find(cfg, len, itf_num, DS_DESC_INTERFACE);
    if (itf == NULL) return 0;
    uint16_t span = ds_desc_interface_span(itf, (uint16_t)(cfg + len - itf));
    uint16_t hdr = cfg[0];
    if (span == 0 || hdr + span > cap) return 0;
    memcpy(out, cfg, hdr);
    memcpy(out + hdr, itf, span);
    uint16_t total = (uint16_t)(hdr + span);
    out[2] = (uint8_t)(total & 0xFF);
    out[3] = (uint8_t)(total >> 8);
    out[4] = 1;  // bNumInterfaces
    // Renumber every alternate setting of the interface to 0.
    const uint8_t *end = out + total;
    for (uint8_t *p = out + hdr; p + 2 <= end && p[0] >= 2; p += p[0]) {
        if (p[1] == DS_DESC_INTERFACE && p[0] >= 9) p[2] = 0;
    }
    return total;
}
