#include <string.h>

// Crypto and packet format against the shared vectors (protocol/vectors/v1.json).
#include "hexutil.h"
#include "mini_test.h"
#include "monocypher.h"
#include "aead.h"
#include "packet.h"
#include "proto.h"
#include "vectors_v1.h"

#define COUNT(a) (sizeof(a) / sizeof((a)[0]))

TEST(rfc8439_aead) {
    uint8_t ct[sizeof(rfc_plaintext)], mac[16], pt[sizeof(rfc_plaintext)];
    crypto_aead_ctx ctx;
    crypto_aead_init_ietf(&ctx, rfc_key, rfc_nonce);
    crypto_aead_write(&ctx, ct, mac, rfc_aad, sizeof(rfc_aad), rfc_plaintext, sizeof(rfc_plaintext));
    CHECK_MEM(ct, rfc_ciphertext, sizeof(rfc_ciphertext));
    CHECK_MEM(mac, rfc_tag, sizeof(rfc_tag));

    crypto_aead_init_ietf(&ctx, rfc_key, rfc_nonce);
    CHECK_EQ_I(crypto_aead_read(&ctx, pt, rfc_tag, rfc_aad, sizeof(rfc_aad), rfc_ciphertext,
                                sizeof(rfc_ciphertext)), 0);
    CHECK_MEM(pt, rfc_plaintext, sizeof(rfc_plaintext));
}

// aead.c: the RFC 8439 example (114 bytes, several blocks, partial ones).
TEST(own_aead_matches_rfc8439) {
    uint8_t ct[sizeof(rfc_plaintext)], tag[16], pt[sizeof(rfc_plaintext)];
    pshb_aead_seal(rfc_key, rfc_nonce, rfc_aad, sizeof(rfc_aad), rfc_plaintext, sizeof(rfc_plaintext), ct, tag);
    CHECK_MEM(ct, rfc_ciphertext, sizeof(rfc_ciphertext));
    CHECK_MEM(tag, rfc_tag, sizeof(rfc_tag));
    CHECK(pshb_aead_open(rfc_key, rfc_nonce, rfc_aad, sizeof(rfc_aad), rfc_ciphertext, sizeof(rfc_ciphertext),
                         rfc_tag, pt));
    CHECK_MEM(pt, rfc_plaintext, sizeof(rfc_plaintext));
    uint8_t bad[16];
    memcpy(bad, rfc_tag, sizeof(bad));
    bad[15] ^= 0x80;
    memset(pt, 0xAA, sizeof(pt));
    CHECK(!pshb_aead_open(rfc_key, rfc_nonce, rfc_aad, sizeof(rfc_aad), rfc_ciphertext, sizeof(rfc_ciphertext),
                          bad, pt));
    CHECK_EQ_U(pt[0], 0xAA);  // nothing decrypted without a valid tag
}

// aead.c against Monocypher for every length up to 4 blocks and several
// lengths of additional data (pseudo-random keys, nonces and texts).
TEST(own_aead_matches_monocypher) {
    uint32_t x = 12345;
    uint8_t key[32], nonce[12], ad[40], pt[256], ct1[256], ct2[256], tag1[16], tag2[16], back[256];
    for (size_t len = 0; len <= sizeof(pt); len++) {
        for (size_t ad_len = 0; ad_len <= sizeof(ad); ad_len += 10) {
            for (size_t i = 0; i < sizeof(key); i++) key[i] = (uint8_t)(x = x * 1103515245u + 12345u) ;
            for (size_t i = 0; i < sizeof(nonce); i++) nonce[i] = (uint8_t)(x = x * 1103515245u + 12345u);
            for (size_t i = 0; i < ad_len; i++) ad[i] = (uint8_t)(x = x * 1103515245u + 12345u);
            for (size_t i = 0; i < len; i++) pt[i] = (uint8_t)(x = x * 1103515245u + 12345u);
            crypto_aead_ctx ctx;
            crypto_aead_init_ietf(&ctx, key, nonce);
            crypto_aead_write(&ctx, ct1, tag1, ad, ad_len, pt, len);
            pshb_aead_seal(key, nonce, ad, ad_len, pt, len, ct2, tag2);
            CHECK_MEM(ct2, ct1, len);
            CHECK_MEM(tag2, tag1, sizeof(tag1));
            CHECK(pshb_aead_open(key, nonce, ad, ad_len, ct2, len, tag2, back));
            CHECK_MEM(back, pt, len);
        }
    }
}

// Blocks prepared from the header give the same result as without; blocks
// for another packet (or another key) are not used.
TEST(prepared_blocks) {
    const vec_packet_t *v = &k_packets[0];
    uint8_t a[PSHB_PONG_PAYLOAD_LEN], b[PSHB_PONG_PAYLOAD_LEN];
    pshb_aead_pre_t pre;
    CHECK(proto_prepare(&pre, v->key, v->direction, v->packet, v->packet_len));
    CHECK(proto_decrypt(v->key, v->direction, v->packet, v->packet_len, a));
    CHECK(proto_decrypt_pre(&pre, v->key, v->direction, v->packet, v->packet_len, b));
    CHECK_MEM(a, b, v->packet_len - PSHB_HDR_LEN - PSHB_TAG_LEN);
    const vec_packet_t *w = &k_packets[1];
    CHECK(proto_decrypt_pre(&pre, w->key, w->direction, w->packet, w->packet_len, b));  // other nonce
    uint8_t other[32];
    memcpy(other, v->key, sizeof(other));
    other[0] ^= 1;
    CHECK(!proto_decrypt_pre(&pre, other, v->direction, v->packet, v->packet_len, b));  // other key
    CHECK(!proto_prepare(&pre, v->key, v->direction, v->packet, v->packet_len + 1));    // wrong length
    CHECK(!pre.valid);
}

// The board's path (blocks prepared while the packet arrives) refuses a
// changed packet like the plain one: every byte of ciphertext and tag, and
// the header (associated data, prepared for the original header).
TEST(prepared_blocks_refuse_tampering) {
    const vec_packet_t *v = &k_packets[0];
    uint8_t out[PSHB_PONG_PAYLOAD_LEN];
    for (size_t i = 0; i < v->packet_len; i++) {
        uint8_t bad[PSHB_MAX_PACKET_LEN];
        memcpy(bad, v->packet, v->packet_len);
        bad[i] ^= 0x40;
        pshb_aead_pre_t pre;
        CHECK(proto_prepare(&pre, v->key, v->direction, v->packet, v->packet_len));
        if (proto_decrypt_pre(&pre, v->key, v->direction, bad, v->packet_len, out)) {
            printf("  byte %zu changed, still accepted\n", i);
            CHECK(false);
        }
    }
}

TEST(seal_matches_vectors) {
    for (size_t i = 0; i < COUNT(k_packets); i++) {
        const vec_packet_t *v = &k_packets[i];
        uint8_t out[PSHB_MAX_PACKET_LEN];
        size_t n = proto_seal(v->key, v->direction, v->type, v->counter, v->payload, v->payload_len,
                              out, sizeof(out));
        CHECK_EQ_U(n, v->packet_len);
        CHECK_MEM(out, v->packet, v->packet_len);
    }
}

TEST(open_matches_vectors) {
    for (size_t i = 0; i < COUNT(k_packets); i++) {
        const vec_packet_t *v = &k_packets[i];
        pshb_header_t h;
        uint8_t payload[PSHB_PONG_PAYLOAD_LEN];
        CHECK(proto_check(v->packet, v->packet_len, &h));
        CHECK_EQ_U(h.type, v->type);
        CHECK_EQ_U(h.counter, v->counter);
        CHECK(proto_decrypt(v->key, v->direction, v->packet, v->packet_len, payload));
        CHECK_MEM(payload, v->payload, v->payload_len);
    }
}

TEST(input_fields_decode) {
    for (size_t i = 0; i < COUNT(k_packets); i++) {
        const vec_packet_t *v = &k_packets[i];
        if (!v->has_input) continue;
        pshb_input_t in;
        pshb_input_decode(v->payload, &in);
        CHECK_EQ_U(in.session_id, v->session_id);
        CHECK_EQ_U(in.flags, v->flags);
        CHECK_EQ_U(in.mods, v->mods);
        CHECK_MEM(in.keys, v->keys, 6);
        CHECK_EQ_U(in.buttons, v->buttons);
        CHECK_EQ_I(in.dx, v->dx);
        CHECK_EQ_I(in.dy, v->dy);
        CHECK_EQ_I(in.wheel, v->wheel);
        CHECK_EQ_I(in.pan, v->pan);
        uint8_t re[PSHB_INPUT_PAYLOAD_LEN];
        pshb_input_encode(re, &in);
        CHECK_MEM(re, v->payload, sizeof(re));
    }
}

TEST(invalid_vectors_rejected) {
    for (size_t i = 0; i < COUNT(k_invalid); i++) {
        const vec_invalid_t *v = &k_invalid[i];
        pshb_header_t h;
        bool structure_ok = proto_check(v->packet, v->packet_len, &h);
        if (!v->expect_auth) {
            if (structure_ok) fprintf(stderr, "  %s: structure unexpectedly ok\n", v->name);
            CHECK(!structure_ok);
            continue;
        }
        CHECK(structure_ok);
        uint8_t payload[PSHB_PONG_PAYLOAD_LEN];
        memset(payload, 0xAA, sizeof(payload));
        bool ok = proto_decrypt(v->key, v->direction, v->packet, v->packet_len, payload);
        if (ok) fprintf(stderr, "  %s: decrypt unexpectedly ok\n", v->name);
        CHECK(!ok);
        CHECK_EQ_U(payload[0], 0xAA);  // untouched on failure
    }
}

TEST(seal_rejects_small_buffer) {
    const vec_packet_t *v = &k_packets[0];
    uint8_t out[PSHB_HDR_LEN + PSHB_INPUT_PAYLOAD_LEN + PSHB_TAG_LEN - 1];
    CHECK_EQ_U(proto_seal(v->key, v->direction, v->type, v->counter, v->payload, v->payload_len, out,
                          sizeof(out)), 0);
}

TEST(hex_roundtrip) {
    uint8_t key[4];
    CHECK(hex_decode("00fFa5Cc", key, 4));
    const uint8_t want[] = {0x00, 0xFF, 0xA5, 0xCC};
    CHECK_MEM(key, want, 4);
    char s[9];
    hex_encode(key, 4, s);
    CHECK(strcmp(s, "00ffa5cc") == 0);
    CHECK(!hex_decode("00ff", key, 4));       // too short
    CHECK(!hex_decode("00ffa5cc00", key, 4)); // too long
    CHECK(!hex_decode("00ffa5cg", key, 4));   // bad digit
}

int main(void) {
    RUN(rfc8439_aead);
    RUN(own_aead_matches_rfc8439);
    RUN(own_aead_matches_monocypher);
    RUN(prepared_blocks);
    RUN(prepared_blocks_refuse_tampering);
    RUN(seal_matches_vectors);
    RUN(open_matches_vectors);
    RUN(input_fields_decode);
    RUN(invalid_vectors_rejected);
    RUN(seal_rejects_small_buffer);
    RUN(hex_roundtrip);
    return TEST_SUMMARY();
}
