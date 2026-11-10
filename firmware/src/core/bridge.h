// Input path state machine: validates incoming datagrams, applies them to
// the input state and builds replies. Portable (used by firmware and sim);
// time and addresses are passed in by the caller.
#ifndef PSHB_BRIDGE_H
#define PSHB_BRIDGE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "input_state.h"
#include "packet.h"

typedef struct {
    uint32_t rx_ok;
    uint32_t rx_auth_fail;
    uint32_t rx_replay;
    uint32_t rx_filtered;
    uint32_t rx_malformed;
} bridge_stats_t;

typedef struct {
    input_state_t *input;
    uint64_t session_id;
    uint32_t fw_version;
    uint64_t boot_us;
    uint32_t allowed_ip;      // sender filter, a.b.c.d as 0xaabbccdd; 0 = any
    bool allow_plaintext;     // accept version 0 packets (debug builds only)
    uint8_t usb_status;       // PSHB_STATUS_USB_* bits, maintained by the platform
    uint64_t tx_counter;      // counter for packets we send
    bridge_stats_t stats;
} bridge_t;

void bridge_init(bridge_t *b, input_state_t *input, uint64_t session_id, uint64_t now_us);

// Process one received datagram. Returns the length of the reply written to
// `reply` (0 = nothing to send back to the sender).
size_t bridge_handle_packet(bridge_t *b, const uint8_t *pkt, size_t len, uint32_t src_ip,
                            uint64_t now_us, uint8_t *reply, size_t reply_cap);

#endif
