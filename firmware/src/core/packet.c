#include "packet.h"

#include <string.h>

void pshb_header_encode(uint8_t out[PSHB_HDR_LEN], const pshb_header_t *h) {
    out[0] = h->version;
    out[1] = h->type;
    pshb_wr64(&out[2], h->counter);
}

void pshb_header_decode(const uint8_t in[PSHB_HDR_LEN], pshb_header_t *h) {
    h->version = in[0];
    h->type = in[1];
    h->counter = pshb_rd64(&in[2]);
}

void pshb_input_encode(uint8_t out[PSHB_INPUT_PAYLOAD_LEN], const pshb_input_t *in) {
    pshb_wr64(&out[0], in->session_id);
    out[8] = in->flags;
    out[9] = in->mods;
    memcpy(&out[10], in->keys, 6);
    out[16] = in->buttons;
    pshb_wr16(&out[17], (uint16_t)in->dx);
    pshb_wr16(&out[19], (uint16_t)in->dy);
    out[21] = (uint8_t)in->wheel;
    out[22] = (uint8_t)in->pan;
}

void pshb_input_decode(const uint8_t in[PSHB_INPUT_PAYLOAD_LEN], pshb_input_t *out) {
    out->session_id = pshb_rd64(&in[0]);
    out->flags = in[8];
    out->mods = in[9];
    memcpy(out->keys, &in[10], 6);
    out->buttons = in[16];
    out->dx = (int16_t)pshb_rd16(&in[17]);
    out->dy = (int16_t)pshb_rd16(&in[19]);
    out->wheel = (int8_t)in[21];
    out->pan = (int8_t)in[22];
}

void pshb_ping_encode(uint8_t out[PSHB_PING_PAYLOAD_LEN], const pshb_ping_t *p) {
    pshb_wr64(&out[0], p->t_app_us);
    pshb_wr32(&out[8], p->seq);
}

void pshb_ping_decode(const uint8_t in[PSHB_PING_PAYLOAD_LEN], pshb_ping_t *p) {
    p->t_app_us = pshb_rd64(&in[0]);
    p->seq = pshb_rd32(&in[8]);
}

void pshb_pong_encode(uint8_t out[PSHB_PONG_PAYLOAD_LEN], const pshb_pong_t *p) {
    pshb_wr64(&out[0], p->t_app_us);
    pshb_wr32(&out[8], p->seq);
    pshb_wr64(&out[12], p->session_id);
    pshb_wr32(&out[20], p->fw_version);
    pshb_wr32(&out[24], p->uptime_ms);
    out[28] = p->status;
    pshb_wr32(&out[29], p->rx_ok);
    pshb_wr32(&out[33], p->rx_auth_fail);
    pshb_wr32(&out[37], p->rx_replay);
    pshb_wr32(&out[41], p->rx_filtered);
    pshb_wr32(&out[45], p->rx_malformed);
}

void pshb_pong_decode(const uint8_t in[PSHB_PONG_PAYLOAD_LEN], pshb_pong_t *p) {
    p->t_app_us = pshb_rd64(&in[0]);
    p->seq = pshb_rd32(&in[8]);
    p->session_id = pshb_rd64(&in[12]);
    p->fw_version = pshb_rd32(&in[20]);
    p->uptime_ms = pshb_rd32(&in[24]);
    p->status = in[28];
    p->rx_ok = pshb_rd32(&in[29]);
    p->rx_auth_fail = pshb_rd32(&in[33]);
    p->rx_replay = pshb_rd32(&in[37]);
    p->rx_filtered = pshb_rd32(&in[41]);
    p->rx_malformed = pshb_rd32(&in[45]);
}

size_t pshb_payload_len(uint8_t type) {
    switch (type) {
    case PSHB_TYPE_INPUT:
    case PSHB_TYPE_HEARTBEAT: return PSHB_INPUT_PAYLOAD_LEN;
    case PSHB_TYPE_PING: return PSHB_PING_PAYLOAD_LEN;
    case PSHB_TYPE_PONG: return PSHB_PONG_PAYLOAD_LEN;
    default: return 0;
    }
}

void pshb_nonce(uint8_t out[PSHB_NONCE_LEN], uint32_t direction, uint64_t counter) {
    pshb_wr32(&out[0], direction);
    pshb_wr64(&out[4], counter);
}
