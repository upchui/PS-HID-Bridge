#include "aead.h"

#include <string.h>

#include "monocypher.h"  // crypto_verify16

pshb_chacha20_block_fn pshb_chacha20_block = pshb_chacha20_block_c;

// Clear secrets (key material, keystream) before their stack space is
// reused. Monocypher's crypto_wipe writes byte by byte through a volatile
// pointer, about 35 ns per byte on the RP2350, which on the ~300 bytes
// per packet took longer than both ChaCha20 blocks together. memset writes
// words; the empty asm with a memory clobber keeps the compiler from
// dropping it as a dead store.
void pshb_wipe(void *p, size_t n) {
    memset(p, 0, n);
    __asm__ __volatile__("" : : "r"(p) : "memory");
}

static inline uint32_t rd32(const uint8_t *p) {
    return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24;
}

static inline void wr32(uint8_t *p, uint32_t v) {
    p[0] = (uint8_t)v;
    p[1] = (uint8_t)(v >> 8);
    p[2] = (uint8_t)(v >> 16);
    p[3] = (uint8_t)(v >> 24);
}

// ---------------------------------------------------------------- ChaCha20

#define ROTL(v, n) (((v) << (n)) | ((v) >> (32 - (n))))
#define QR(a, b, c, d)                                    \
    do {                                                  \
        a += b; d ^= a; d = ROTL(d, 16);                  \
        c += d; b ^= c; b = ROTL(b, 12);                  \
        a += b; d ^= a; d = ROTL(d, 8);                   \
        c += d; b ^= c; b = ROTL(b, 7);                   \
    } while (0)

void pshb_chacha20_block_c(uint32_t out[16], const uint32_t in[16]) {
    uint32_t x[16];
    memcpy(x, in, sizeof(x));
    for (int i = 0; i < 10; i++) {
        QR(x[0], x[4], x[8], x[12]);
        QR(x[1], x[5], x[9], x[13]);
        QR(x[2], x[6], x[10], x[14]);
        QR(x[3], x[7], x[11], x[15]);
        QR(x[0], x[5], x[10], x[15]);
        QR(x[1], x[6], x[11], x[12]);
        QR(x[2], x[7], x[8], x[13]);
        QR(x[3], x[4], x[9], x[14]);
    }
    for (int i = 0; i < 16; i++) out[i] = x[i] + in[i];
    pshb_wipe(x, sizeof(x));
}

// Block counter 0 (the first block gives the Poly1305 key).
static void chacha_init(uint32_t st[16], const uint8_t key[32], const uint8_t nonce[12]) {
    st[0] = 0x61707865u;  // "expand 32-byte k"
    st[1] = 0x3320646eu;
    st[2] = 0x79622d32u;
    st[3] = 0x6b206574u;
    for (int i = 0; i < 8; i++) st[4 + i] = rd32(key + 4 * i);
    st[12] = 0;
    for (int i = 0; i < 3; i++) st[13 + i] = rd32(nonce + 4 * i);
}

// out = in XOR keystream, starting at block counter 1.
static void chacha_xor(uint32_t st[16], uint8_t *out, const uint8_t *in, size_t len) {
    uint32_t ks[16];
    st[12] = 1;
    while (len) {
        pshb_chacha20_block(ks, st);
        st[12]++;
        size_t n = len < 64 ? len : 64;
        for (size_t i = 0; i < n; i++) out[i] = in[i] ^ (uint8_t)(ks[i >> 2] >> (8 * (i & 3)));
        out += n;
        in += n;
        len -= n;
    }
    pshb_wipe(ks, sizeof(ks));
}

// ---------------------------------------------------------------- Poly1305
// 26-bit limbs (the "donna" 32-bit construction): every product fits in
// 64 bits and needs one 32x32->64 multiplication.

typedef struct {
    uint32_t r[5], s[4], h[5], pad[4];
} poly_t;

static void poly_init(poly_t *p, const uint8_t key[32]) {
    p->r[0] = (rd32(key + 0)) & 0x3ffffff;
    p->r[1] = (rd32(key + 3) >> 2) & 0x3ffff03;
    p->r[2] = (rd32(key + 6) >> 4) & 0x3ffc0ff;
    p->r[3] = (rd32(key + 9) >> 6) & 0x3f03fff;
    p->r[4] = (rd32(key + 12) >> 8) & 0x00fffff;
    for (int i = 0; i < 4; i++) p->s[i] = p->r[i + 1] * 5;
    memset(p->h, 0, sizeof(p->h));
    for (int i = 0; i < 4; i++) p->pad[i] = rd32(key + 16 + 4 * i);
}

// One full 16-byte block (the AEAD pads everything to full blocks).
static void poly_block(poly_t *p, const uint8_t m[16]) {
    const uint32_t r0 = p->r[0], r1 = p->r[1], r2 = p->r[2], r3 = p->r[3], r4 = p->r[4];
    const uint32_t s1 = p->s[0], s2 = p->s[1], s3 = p->s[2], s4 = p->s[3];
    uint32_t h0 = p->h[0], h1 = p->h[1], h2 = p->h[2], h3 = p->h[3], h4 = p->h[4];

    h0 += (rd32(m + 0)) & 0x3ffffff;
    h1 += (rd32(m + 3) >> 2) & 0x3ffffff;
    h2 += (rd32(m + 6) >> 4) & 0x3ffffff;
    h3 += (rd32(m + 9) >> 6) & 0x3ffffff;
    h4 += (rd32(m + 12) >> 8) | (1u << 24);

    uint64_t d0 = (uint64_t)h0 * r0 + (uint64_t)h1 * s4 + (uint64_t)h2 * s3 + (uint64_t)h3 * s2 + (uint64_t)h4 * s1;
    uint64_t d1 = (uint64_t)h0 * r1 + (uint64_t)h1 * r0 + (uint64_t)h2 * s4 + (uint64_t)h3 * s3 + (uint64_t)h4 * s2;
    uint64_t d2 = (uint64_t)h0 * r2 + (uint64_t)h1 * r1 + (uint64_t)h2 * r0 + (uint64_t)h3 * s4 + (uint64_t)h4 * s3;
    uint64_t d3 = (uint64_t)h0 * r3 + (uint64_t)h1 * r2 + (uint64_t)h2 * r1 + (uint64_t)h3 * r0 + (uint64_t)h4 * s4;
    uint64_t d4 = (uint64_t)h0 * r4 + (uint64_t)h1 * r3 + (uint64_t)h2 * r2 + (uint64_t)h3 * r1 + (uint64_t)h4 * r0;

    uint32_t c;
    c = (uint32_t)(d0 >> 26); h0 = (uint32_t)d0 & 0x3ffffff;
    d1 += c; c = (uint32_t)(d1 >> 26); h1 = (uint32_t)d1 & 0x3ffffff;
    d2 += c; c = (uint32_t)(d2 >> 26); h2 = (uint32_t)d2 & 0x3ffffff;
    d3 += c; c = (uint32_t)(d3 >> 26); h3 = (uint32_t)d3 & 0x3ffffff;
    d4 += c; c = (uint32_t)(d4 >> 26); h4 = (uint32_t)d4 & 0x3ffffff;
    h0 += c * 5; c = h0 >> 26; h0 &= 0x3ffffff;
    h1 += c;

    p->h[0] = h0; p->h[1] = h1; p->h[2] = h2; p->h[3] = h3; p->h[4] = h4;
}

// `m` padded with zeros to a multiple of 16 bytes (RFC 8439 2.8).
static void poly_padded(poly_t *p, const uint8_t *m, size_t len) {
    while (len >= 16) {
        poly_block(p, m);
        m += 16;
        len -= 16;
    }
    if (len) {
        uint8_t last[16] = {0};
        memcpy(last, m, len);
        poly_block(p, last);
        pshb_wipe(last, sizeof(last));
    }
}

static void poly_finish(poly_t *p, uint8_t mac[16]) {
    uint32_t h0 = p->h[0], h1 = p->h[1], h2 = p->h[2], h3 = p->h[3], h4 = p->h[4], c;
    // Full carry.
    c = h1 >> 26; h1 &= 0x3ffffff;
    h2 += c; c = h2 >> 26; h2 &= 0x3ffffff;
    h3 += c; c = h3 >> 26; h3 &= 0x3ffffff;
    h4 += c; c = h4 >> 26; h4 &= 0x3ffffff;
    h0 += c * 5; c = h0 >> 26; h0 &= 0x3ffffff;
    h1 += c;
    // g = h - (2^130 - 5); take g if it is not negative (constant time).
    uint32_t g0 = h0 + 5; c = g0 >> 26; g0 &= 0x3ffffff;
    uint32_t g1 = h1 + c; c = g1 >> 26; g1 &= 0x3ffffff;
    uint32_t g2 = h2 + c; c = g2 >> 26; g2 &= 0x3ffffff;
    uint32_t g3 = h3 + c; c = g3 >> 26; g3 &= 0x3ffffff;
    uint32_t g4 = h4 + c - (1u << 26);
    uint32_t mask = (g4 >> 31) - 1;  // all ones if g >= 0
    h0 = (h0 & ~mask) | (g0 & mask);
    h1 = (h1 & ~mask) | (g1 & mask);
    h2 = (h2 & ~mask) | (g2 & mask);
    h3 = (h3 & ~mask) | (g3 & mask);
    h4 = (h4 & ~mask) | (g4 & mask);
    // h mod 2^128, plus the pad.
    uint32_t w0 = h0 | (h1 << 26);
    uint32_t w1 = (h1 >> 6) | (h2 << 20);
    uint32_t w2 = (h2 >> 12) | (h3 << 14);
    uint32_t w3 = (h3 >> 18) | (h4 << 8);
    uint64_t f = (uint64_t)w0 + p->pad[0];
    wr32(mac + 0, (uint32_t)f);
    f = (uint64_t)w1 + p->pad[1] + (f >> 32);
    wr32(mac + 4, (uint32_t)f);
    f = (uint64_t)w2 + p->pad[2] + (f >> 32);
    wr32(mac + 8, (uint32_t)f);
    f = (uint64_t)w3 + p->pad[3] + (f >> 32);
    wr32(mac + 12, (uint32_t)f);
    pshb_wipe(p, sizeof(*p));
}

// Tag over ad | pad | ct | pad | le64(ad_len) | le64(len), keyed with the
// first 32 bytes of block 0.
static void aead_tag(uint8_t tag[16], const uint32_t block0[16], const uint8_t *ad, size_t ad_len,
                     const uint8_t *ct, size_t len) {
    uint8_t key[32];
    for (int i = 0; i < 8; i++) wr32(key + 4 * i, block0[i]);
    poly_t p;
    poly_init(&p, key);
    pshb_wipe(key, sizeof(key));
    poly_padded(&p, ad, ad_len);
    poly_padded(&p, ct, len);
    uint8_t sizes[16];
    wr32(sizes + 0, (uint32_t)ad_len);
    wr32(sizes + 4, (uint32_t)((uint64_t)ad_len >> 32));
    wr32(sizes + 8, (uint32_t)len);
    wr32(sizes + 12, (uint32_t)((uint64_t)len >> 32));
    poly_block(&p, sizes);
    poly_finish(&p, tag);
}

void pshb_aead_seal(const uint8_t key[32], const uint8_t nonce[12], const uint8_t *ad, size_t ad_len,
                    const uint8_t *pt, size_t len, uint8_t *ct, uint8_t tag[16]) {
    uint32_t st[16], block0[16];
    chacha_init(st, key, nonce);
    pshb_chacha20_block(block0, st);
    chacha_xor(st, ct, pt, len);
    aead_tag(tag, block0, ad, ad_len, ct, len);
    pshb_wipe(block0, sizeof(block0));
    pshb_wipe(st, sizeof(st));
}

void pshb_aead_prepare(pshb_aead_pre_t *pre, const uint8_t key[32], const uint8_t nonce[12]) {
    uint32_t st[16];
    chacha_init(st, key, nonce);
    pshb_chacha20_block(pre->block0, st);
    st[12] = 1;
    pshb_chacha20_block(pre->block1, st);
    pshb_wipe(st, sizeof(st));
    memcpy(pre->key, key, sizeof(pre->key));
    memcpy(pre->nonce, nonce, sizeof(pre->nonce));
    pre->valid = true;
}

bool pshb_aead_open_pre(const pshb_aead_pre_t *pre, const uint8_t key[32], const uint8_t nonce[12],
                        const uint8_t *ad, size_t ad_len, const uint8_t *ct, size_t len, const uint8_t tag[16],
                        uint8_t *pt) {
    if (!pre || !pre->valid || len > 64 || memcmp(pre->nonce, nonce, sizeof(pre->nonce)) != 0 ||
        memcmp(pre->key, key, sizeof(pre->key)) != 0) {
        return pshb_aead_open(key, nonce, ad, ad_len, ct, len, tag, pt);
    }
    uint8_t real[16];
    aead_tag(real, pre->block0, ad, ad_len, ct, len);
    bool ok = crypto_verify16(real, tag) == 0;
    if (ok) {
        for (size_t i = 0; i < len; i++) pt[i] = ct[i] ^ (uint8_t)(pre->block1[i >> 2] >> (8 * (i & 3)));
    }
    pshb_wipe(real, sizeof(real));
    return ok;
}

bool pshb_aead_open(const uint8_t key[32], const uint8_t nonce[12], const uint8_t *ad, size_t ad_len,
                    const uint8_t *ct, size_t len, const uint8_t tag[16], uint8_t *pt) {
    uint32_t st[16], block0[16];
    uint8_t real[16];
    chacha_init(st, key, nonce);
    pshb_chacha20_block(block0, st);
    aead_tag(real, block0, ad, ad_len, ct, len);
    pshb_wipe(block0, sizeof(block0));
    bool ok = crypto_verify16(real, tag) == 0;
    if (ok) chacha_xor(st, pt, ct, len);
    pshb_wipe(real, sizeof(real));
    pshb_wipe(st, sizeof(st));
    return ok;
}
