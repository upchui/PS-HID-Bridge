#include "bridge.h"

#include <string.h>

#include "monocypher.h"
#include "proto.h"
#include "pshb_config.h"
#include "pshb_version.h"

void bridge_init(bridge_t *b, input_state_t *input, uint64_t session_id, uint64_t tx_counter_start,
                 uint64_t now_us) {
    memset(b, 0, sizeof(*b));
    b->input = input;
    b->session_id = session_id;
    b->fw_version = PSHB_FW_VERSION;
    b->boot_us = now_us;
    b->failsafe_us = PSHB_FAILSAFE_US_DEFAULT;
    b->tx_counter = tx_counter_start & PSHB_TX_COUNTER_MASK;
    b->failsafe_active = true;  // nothing pressed until the first valid input
}

void bridge_set_key(bridge_t *b, const uint8_t key[32]) {
    if (key) {
        memcpy(b->key, key, sizeof(b->key));
        b->key_set = true;
    } else {
        crypto_wipe(b->key, sizeof(b->key));
        b->key_set = false;
    }
}

void bridge_release(bridge_t *b) {
    input_state_release_all(b->input);
    b->failsafe_active = true;
}

void bridge_tick(bridge_t *b, uint64_t now_us) {
    if (!b->failsafe_active && now_us - b->last_input_us > b->failsafe_us) {
        bridge_release(b);
    }
}

static void apply_input(bridge_t *b, const pshb_input_t *in, uint64_t now_us) {
    b->last_input_us = now_us;
    b->input_seen = true;
    b->input_applied = true;
    b->failsafe_active = false;
    if (in->flags & PSHB_FLAG_RELEASE_ALL) {
        input_state_release_all(b->input);
        return;
    }
    input_state_set_keyboard(b->input, in->mods, in->keys);
    input_state_set_buttons(b->input, in->buttons);
    input_state_add_motion(b->input, in->dx, in->dy, in->wheel, in->pan);
}

static uint8_t status_bits(const bridge_t *b) {
    uint8_t s = b->usb_status;
    if (b->failsafe_active) s |= PSHB_STATUS_FAILSAFE;
    if (b->allow_plaintext) s |= PSHB_STATUS_PLAINTEXT;
    return s;
}

static size_t build_pong(bridge_t *b, bool encrypted, uint64_t t_app_us, uint32_t seq,
                         uint64_t now_us, uint8_t *reply, size_t cap) {
    pshb_pong_t p = {
        .t_app_us = t_app_us,
        .seq = seq,
        .session_id = b->session_id,
        .fw_version = b->fw_version,
        .uptime_ms = (uint32_t)((now_us - b->boot_us) / 1000),
        .status = status_bits(b),
        .rx_ok = b->stats.rx_ok,
        .rx_auth_fail = b->stats.rx_auth_fail,
        .rx_replay = b->stats.rx_replay,
        .rx_filtered = b->stats.rx_filtered,
        .rx_malformed = b->stats.rx_malformed,
    };
    uint8_t payload[PSHB_PONG_PAYLOAD_LEN];
    pshb_pong_encode(payload, &p);
    b->tx_counter = (b->tx_counter + 1) & PSHB_TX_COUNTER_MASK;

    if (encrypted) {
        return proto_seal(b->key, PSHB_DIR_BOARD_TO_APP, PSHB_TYPE_PONG, b->tx_counter, payload,
                          sizeof(payload), reply, cap);
    }
    size_t len = PSHB_HDR_LEN + sizeof(payload);
    if (cap < len) return 0;
    pshb_header_t h = {.version = PSHB_PROTO_VERSION_PLAIN, .type = PSHB_TYPE_PONG,
                       .counter = b->tx_counter};
    pshb_header_encode(reply, &h);
    memcpy(reply + PSHB_HDR_LEN, payload, sizeof(payload));
    return len;
}

// Unencrypted debug format: no counter, session or tag checks.
static size_t handle_plain(bridge_t *b, const uint8_t *pkt, size_t len, uint64_t now_us,
                           uint8_t *reply, size_t cap) {
    pshb_header_t h;
    pshb_header_decode(pkt, &h);
    size_t payload_len = pshb_payload_len(h.type);
    if (payload_len == 0 || h.type == PSHB_TYPE_PONG || len != PSHB_HDR_LEN + payload_len) {
        b->stats.rx_malformed++;
        return 0;
    }
    b->stats.rx_ok++;
    const uint8_t *payload = pkt + PSHB_HDR_LEN;
    if (h.type == PSHB_TYPE_PING) {
        pshb_ping_t ping;
        pshb_ping_decode(payload, &ping);
        return build_pong(b, false, ping.t_app_us, ping.seq, now_us, reply, cap);
    }
    pshb_input_t in;
    pshb_input_decode(payload, &in);
    apply_input(b, &in, now_us);
    return 0;
}

size_t bridge_handle_packet(bridge_t *b, const uint8_t *pkt, size_t len, uint32_t src_ip,
                            uint64_t now_us, uint8_t *reply, size_t reply_cap) {
    if (len >= PSHB_HDR_LEN && pkt[0] == PSHB_PROTO_VERSION_PLAIN && b->allow_plaintext) {
        if (!bridge_sender_allowed(b, src_ip)) {
            b->stats.rx_filtered++;
            return 0;
        }
        return handle_plain(b, pkt, len, now_us, reply, reply_cap);
    }

    // 1. structure
    pshb_header_t h;
    if (!proto_check(pkt, len, &h) || h.type == PSHB_TYPE_PONG) {
        b->stats.rx_malformed++;
        return 0;
    }
    // 2. sender filter
    if (!bridge_sender_allowed(b, src_ip)) {
        b->stats.rx_filtered++;
        return 0;
    }
    // 3. replay (cheap, before any crypto)
    if (h.counter <= b->last_rx_counter) {
        b->stats.rx_replay++;
        return 0;
    }
    // 4. authenticity
    uint8_t payload[PSHB_INPUT_PAYLOAD_LEN > PSHB_PING_PAYLOAD_LEN ? PSHB_INPUT_PAYLOAD_LEN
                                                                   : PSHB_PING_PAYLOAD_LEN];
    if (!b->key_set || !proto_decrypt(b->key, PSHB_DIR_APP_TO_BOARD, pkt, len, payload)) {
        b->stats.rx_auth_fail++;
        return 0;
    }
    // 5. only authentic packets may advance the counter
    b->last_rx_counter = h.counter;

    if (h.type == PSHB_TYPE_PING) {
        b->stats.rx_ok++;
        pshb_ping_t ping;
        pshb_ping_decode(payload, &ping);
        return build_pong(b, true, ping.t_app_us, ping.seq, now_us, reply, reply_cap);
    }

    // 6. INPUT / HEARTBEAT must belong to the current session
    pshb_input_t in;
    pshb_input_decode(payload, &in);
    crypto_wipe(payload, sizeof(payload));
    if (in.session_id != b->session_id) {
        b->stats.rx_replay++;
        // Tell a legitimate app about the new session right away (rate limited).
        if (!b->hint_sent || now_us - b->last_hint_us >= PSHB_SESSION_HINT_GAP_US) {
            b->hint_sent = true;
            b->last_hint_us = now_us;
            return build_pong(b, true, 0, 0, now_us, reply, reply_cap);
        }
        return 0;
    }
    b->stats.rx_ok++;
    apply_input(b, &in, now_us);
    return 0;
}
