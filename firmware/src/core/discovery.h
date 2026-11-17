// Discovery: the answer to an app looking for boards in the network
// (protocol/SPEC.md "Discovery"). Portable: the board and the simulator use it.
#ifndef PSHB_DISCOVERY_H
#define PSHB_DISCOVERY_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "protocol.h"

typedef struct {
    uint8_t mac[6];
    uint16_t udp_port;
    uint32_t fw_version;
    uint8_t flags;  // PSHB_DISCOVERY_KEY_SET, ...
    char name[PSHB_DISCOVERY_NAME_LEN];  // NUL-terminated, the name shown (board_name.h)
} pshb_discovery_info_t;

// Answers per second at most: a flood of requests costs core 1 little and
// the board sends no flood back.
#define PSHB_DISCOVERY_MAX_PER_S 16

typedef struct {
    uint64_t window_start_us;
    uint32_t sent;
} pshb_discovery_limit_t;

// The answer to `req` in `out` (PSHB_DISCOVERY_LEN bytes); 0 if `req` is no
// valid request (wrong size, magic, version or type) or `cap` is too small.
size_t pshb_discovery_answer(const uint8_t *req, size_t len, const pshb_discovery_info_t *info, uint8_t *out,
                             size_t cap);

// True if one more answer may go out now (at most PSHB_DISCOVERY_MAX_PER_S
// per second); counts it.
bool pshb_discovery_allow(pshb_discovery_limit_t *l, uint64_t now_us);

#endif
