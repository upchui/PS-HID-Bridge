#include "bridge.h"

#include <string.h>

#include "pshb_config.h"

void bridge_init(bridge_t *b, input_state_t *input, uint64_t session_id, uint64_t now_us) {
    memset(b, 0, sizeof(*b));
    b->input = input;
    b->session_id = session_id;
    b->fw_version = PSHB_FW_VERSION;
    b->boot_us = now_us;
}

static void apply_input(bridge_t *b, const pshb_input_t *in) {
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
    if (b->allow_plaintext) s |= PSHB_STATUS_PLAINTEXT;
    return s;
}

static size_t build_pong(bridge_t *b, uint8_t version, const pshb_ping_t *ping, uint64_t now_us,
                         uint8_t *reply, size_t cap) {
    size_t len = PSHB_HDR_LEN + PSHB_PONG_PAYLOAD_LEN;
    if (cap < len) return 0;
    pshb_header_t h = {.version = version, .type = PSHB_TYPE_PONG, .counter = ++b->tx_counter};
    pshb_pong_t p = {
        .t_app_us = ping->t_app_us,
        .seq = ping->seq,
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
    pshb_header_encode(reply, &h);
    pshb_pong_encode(reply + PSHB_HDR_LEN, &p);
    return len;
}

size_t bridge_handle_packet(bridge_t *b, const uint8_t *pkt, size_t len, uint32_t src_ip,
                            uint64_t now_us, uint8_t *reply, size_t reply_cap) {
    if (len < PSHB_HDR_LEN) {
        b->stats.rx_malformed++;
        return 0;
    }
    pshb_header_t h;
    pshb_header_decode(pkt, &h);
    size_t payload_len = pshb_payload_len(h.type);
    if (payload_len == 0 || h.type == PSHB_TYPE_PONG) {
        b->stats.rx_malformed++;
        return 0;
    }
    if (b->allowed_ip != 0 && src_ip != b->allowed_ip) {
        b->stats.rx_filtered++;
        return 0;
    }

    if (h.version != PSHB_PROTO_VERSION_PLAIN || !b->allow_plaintext) {
        // Encrypted protocol: added in step 4.
        b->stats.rx_malformed++;
        return 0;
    }
    if (len != PSHB_HDR_LEN + payload_len) {
        b->stats.rx_malformed++;
        return 0;
    }
    const uint8_t *payload = pkt + PSHB_HDR_LEN;
    b->stats.rx_ok++;

    if (h.type == PSHB_TYPE_PING) {
        pshb_ping_t ping;
        pshb_ping_decode(payload, &ping);
        return build_pong(b, PSHB_PROTO_VERSION_PLAIN, &ping, now_us, reply, reply_cap);
    }

    pshb_input_t in;
    pshb_input_decode(payload, &in);
    apply_input(b, &in);
    return 0;
}
