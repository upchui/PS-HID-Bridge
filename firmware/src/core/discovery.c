#include "discovery.h"

#include <string.h>

#define MAGIC_LEN 8

size_t pshb_discovery_answer(const uint8_t *req, size_t len, const pshb_discovery_info_t *info, uint8_t *out,
                             size_t cap) {
    if (len != PSHB_DISCOVERY_LEN || cap < PSHB_DISCOVERY_LEN) return 0;
    if (memcmp(req, PSHB_DISCOVERY_MAGIC, MAGIC_LEN) != 0) return 0;
    if (req[8] != PSHB_DISCOVERY_VERSION || req[9] != PSHB_DISCOVERY_REQUEST) return 0;

    memset(out, 0, PSHB_DISCOVERY_LEN);
    memcpy(out, PSHB_DISCOVERY_MAGIC, MAGIC_LEN);
    out[8] = PSHB_DISCOVERY_VERSION;
    out[9] = PSHB_DISCOVERY_ANSWER;
    out[10] = info->flags;
    memcpy(out + 12, info->mac, 6);
    pshb_wr16(out + 18, info->udp_port);
    pshb_wr32(out + 20, info->fw_version);
    // Always NUL-terminated within the field.
    for (size_t i = 0; i < PSHB_DISCOVERY_NAME_LEN - 1 && info->name[i]; i++) out[24 + i] = (uint8_t)info->name[i];
    return PSHB_DISCOVERY_LEN;
}

bool pshb_discovery_allow(pshb_discovery_limit_t *l, uint64_t now_us) {
    if (now_us - l->window_start_us >= 1000000u) {
        l->window_start_us = now_us;
        l->sent = 0;
    }
    if (l->sent >= PSHB_DISCOVERY_MAX_PER_S) return false;
    l->sent++;
    return true;
}
