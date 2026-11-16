// ChaCha20-Poly1305 (RFC 8439, IETF variant: 96-bit nonce, 32-bit block
// counter) for the protocol's packets: the same result as Monocypher's
// crypto_aead_init_ietf + crypto_aead_write/read, but faster for small
// packets, which are what the input path decrypts 1000 times per second:
//  * Poly1305 with 26-bit limbs (25 32x32->64 multiplications per block),
//  * the ChaCha20 block function replaceable by a faster one for the CPU
//    (pico/chacha20_m33.S), checked against the C version before use.
#ifndef PSHB_AEAD_H
#define PSHB_AEAD_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// One ChaCha20 block: `out` = 20 rounds of `in`, plus `in` (RFC 8439 2.3).
// `out` and `in` must not overlap.
typedef void (*pshb_chacha20_block_fn)(uint32_t out[16], const uint32_t in[16]);
void pshb_chacha20_block_c(uint32_t out[16], const uint32_t in[16]);

// The block function in use (pshb_chacha20_block_c unless the platform
// installs a faster one at start, before any other thread uses it).
extern pshb_chacha20_block_fn pshb_chacha20_block;

// Two independent blocks at once (pshb_aead_prepare: blocks 0 and 1 of a
// packet). By default one after the other; a platform with a second core
// can compute one on each (pico/crypto_help.c).
typedef void (*pshb_chacha20_block2_fn)(uint32_t out0[16], const uint32_t in0[16], uint32_t out1[16],
                                        const uint32_t in1[16]);
extern pshb_chacha20_block2_fn pshb_chacha20_block2;

// Overwrite a secret with zeros (word-wise, not optimised away).
void pshb_wipe(void *p, size_t n);

// Encrypt `len` bytes of `pt` into `ct` and authenticate them with `ad`.
void pshb_aead_seal(const uint8_t key[32], const uint8_t nonce[12], const uint8_t *ad, size_t ad_len,
                    const uint8_t *pt, size_t len, uint8_t *ct, uint8_t tag[16]);

// Check `tag` (in constant time) and only then decrypt `ct` into `pt`.
// Returns false if the tag does not match (`pt` untouched).
bool pshb_aead_open(const uint8_t key[32], const uint8_t nonce[12], const uint8_t *ad, size_t ad_len,
                    const uint8_t *ct, size_t len, const uint8_t tag[16], uint8_t *pt);

// The two ChaCha20 blocks of a packet of at most 64 bytes depend only on key
// and nonce, not on the data: they can be computed while the rest of the
// packet is still being read (pico/w5500_port.c reads it by DMA).
typedef struct {
    bool valid;
    uint8_t key[32];
    uint8_t nonce[12];
    uint32_t block0[16];  // Poly1305 key (first half)
    uint32_t block1[16];  // keystream for bytes 0..63
} pshb_aead_pre_t;

void pshb_aead_prepare(pshb_aead_pre_t *pre, const uint8_t key[32], const uint8_t nonce[12]);

// pshb_aead_open with the blocks from `pre` if it was prepared for this key
// and nonce and the text fits one block; otherwise computed as usual.
bool pshb_aead_open_pre(const pshb_aead_pre_t *pre, const uint8_t key[32], const uint8_t nonce[12],
                        const uint8_t *ad, size_t ad_len, const uint8_t *ct, size_t len, const uint8_t tag[16],
                        uint8_t *pt);

#endif
