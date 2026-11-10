#include "proto.h"

#include "monocypher.h"

bool proto_check(const uint8_t *pkt, size_t len, pshb_header_t *h) {
    if (len < PSHB_HDR_LEN + PSHB_TAG_LEN) return false;
    pshb_header_decode(pkt, h);
    if (h->version != PSHB_PROTO_VERSION) return false;
    size_t payload_len = pshb_payload_len(h->type);
    if (payload_len == 0) return false;
    return len == PSHB_HDR_LEN + payload_len + PSHB_TAG_LEN;
}

// A fresh context per packet: crypto_aead_write/read ratchet the key after
// each message, which we do not want (every packet is independent).
bool proto_decrypt(const uint8_t key[PSHB_KEY_LEN], uint32_t direction, const uint8_t *pkt,
                   size_t len, uint8_t *payload_out) {
    size_t payload_len = len - PSHB_HDR_LEN - PSHB_TAG_LEN;
    uint8_t nonce[PSHB_NONCE_LEN];
    pshb_nonce(nonce, direction, pshb_rd64(&pkt[2]));

    crypto_aead_ctx ctx;
    crypto_aead_init_ietf(&ctx, key, nonce);
    int r = crypto_aead_read(&ctx, payload_out, pkt + PSHB_HDR_LEN + payload_len, pkt, PSHB_HDR_LEN,
                             pkt + PSHB_HDR_LEN, payload_len);
    crypto_wipe(&ctx, sizeof(ctx));
    return r == 0;
}

size_t proto_seal(const uint8_t key[PSHB_KEY_LEN], uint32_t direction, uint8_t type,
                  uint64_t counter, const uint8_t *payload, size_t payload_len, uint8_t *out,
                  size_t cap) {
    size_t total = PSHB_HDR_LEN + payload_len + PSHB_TAG_LEN;
    if (cap < total) return 0;
    pshb_header_t h = {.version = PSHB_PROTO_VERSION, .type = type, .counter = counter};
    pshb_header_encode(out, &h);
    uint8_t nonce[PSHB_NONCE_LEN];
    pshb_nonce(nonce, direction, counter);

    crypto_aead_ctx ctx;
    crypto_aead_init_ietf(&ctx, key, nonce);
    crypto_aead_write(&ctx, out + PSHB_HDR_LEN, out + PSHB_HDR_LEN + payload_len, out,
                      PSHB_HDR_LEN, payload, payload_len);
    crypto_wipe(&ctx, sizeof(ctx));
    return total;
}
