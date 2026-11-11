// Input path state machine: validates incoming datagrams, applies them to
// the input state and builds replies. Portable (used by firmware and sim);
// time and addresses are passed in by the caller.
//
// Order of checks for v1 packets (protocol/SPEC.md, "Ablauf im Board"):
// structure -> sender filter -> counter > last -> tag -> update last ->
// session (INPUT/HEARTBEAT) -> apply / reply.
#ifndef PSHB_BRIDGE_H
#define PSHB_BRIDGE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "input_state.h"
#include "packet.h"

#define PSHB_FAILSAFE_US_DEFAULT  250000u
#define PSHB_SESSION_HINT_GAP_US  100000u
#define PSHB_TX_COUNTER_MASK      ((1ull << 62) - 1)

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

    // configuration
    uint8_t key[32];
    bool key_set;
    // sender filter (a.b.c.d as 0xaabbccdd): allowed if ((ip ^ net) & mask) == 0;
    // mask 0 = any sender
    uint32_t allowed_net;
    uint32_t allowed_mask;
    bool allow_plaintext;     // accept version 0 packets (debug builds only)
    uint32_t failsafe_us;

    // maintained by the platform
    uint8_t usb_status;       // PSHB_STATUS_USB_* bits

    // protocol state
    uint64_t last_rx_counter; // last counter that passed authentication
    uint64_t tx_counter;      // last counter we sent
    uint64_t last_input_us;   // last valid INPUT/HEARTBEAT
    bool input_seen;
    bool failsafe_active;
    uint64_t last_hint_us;
    bool hint_sent;
    bool input_applied;       // set by every applied INPUT/HEARTBEAT; the caller clears it

    bridge_stats_t stats;
} bridge_t;

// `tx_counter_start` should come from a TRNG (masked to 62 bits inside).
void bridge_init(bridge_t *b, input_state_t *input, uint64_t session_id, uint64_t tx_counter_start,
                 uint64_t now_us);

// Set the pre-shared key; NULL clears it (then every v1 packet fails auth).
void bridge_set_key(bridge_t *b, const uint8_t key[32]);

// Process one received datagram. Returns the length of the reply written to
// `reply` (0 = nothing to send back to the sender).
size_t bridge_handle_packet(bridge_t *b, const uint8_t *pkt, size_t len, uint32_t src_ip,
                            uint64_t now_us, uint8_t *reply, size_t reply_cap);

// Does the sender filter let `src_ip` through?
static inline bool bridge_sender_allowed(const bridge_t *b, uint32_t src_ip) {
    return ((src_ip ^ b->allowed_net) & b->allowed_mask) == 0;
}

// Periodic work (failsafe). Call at least every few milliseconds.
void bridge_tick(bridge_t *b, uint64_t now_us);

// Release everything immediately (e.g. USB suspend) and enter failsafe.
void bridge_release(bridge_t *b);

#endif
