// State shared between core 0 (input path) and core 1 (network management).
// Every field has exactly one writer.
#ifndef PSHB_NET_SHARED_H
#define PSHB_NET_SHARED_H

#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>

#include "bridge.h"
#include "settings.h"

typedef enum {
    NET_STATE_INIT = 0,
    NET_STATE_CHIP_ERROR,  // W5500 did not answer
    NET_STATE_NO_LINK,
    NET_STATE_DHCP,        // link up, waiting for a lease
    NET_STATE_READY,       // IP configured, input socket open
} net_state_t;

typedef struct {
    // written by core 1
    _Atomic uint32_t state;       // net_state_t
    _Atomic uint32_t ip;          // a.b.c.d as 0xaabbccdd, 0 = none
    _Atomic uint32_t udp_port;
    _Atomic bool udp_enable;      // socket 1 is open and may be used by core 0
    // written by core 0
    _Atomic bool udp_active;      // core 0 is currently using socket 1
} net_shared_t;

extern net_shared_t g_net;

// Loaded by core 0 at boot, owned by core 1 afterwards (web interface).
extern pshb_settings_t g_settings;

// Input path state of core 0; core 1 only reads statistics/status from it
// (aligned 32-bit loads are atomic).
extern bridge_t g_bridge;

// Core 1 -> core 0: new key / sender filter (picked up by core 0's loop).
void input_cfg_post(const pshb_settings_t *s);
// Core 0: apply a pending update to the bridge, if any.
void input_cfg_poll(bridge_t *b);

static inline uint32_t ip_to_u32(const uint8_t ip[4]) {
    return ((uint32_t)ip[0] << 24) | ((uint32_t)ip[1] << 16) | ((uint32_t)ip[2] << 8) | ip[3];
}

#endif
