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

// Overwrite a secret with zeros (word-wise, not optimised away).
void pshb_wipe(void *p, size_t n);

// Encrypt `len` bytes of `pt` into `ct` and authenticate them with `ad`.
void pshb_aead_seal(const uint8_t key[32], const uint8_t nonce[12], const uint8_t *ad, size_t ad_len,
                    const uint8_t *pt, size_t len, uint8_t *ct, uint8_t tag[16]);

// Check `tag` (in constant time) and only then decrypt `ct` into `pt`.
// Returns false if the tag does not match (`pt` untouched).
bool pshb_aead_open(const uint8_t key[32], const uint8_t nonce[12], const uint8_t *ad, size_t ad_len,
                    const uint8_t *ct, size_t len, const uint8_t tag[16], uint8_t *pt);

#endif
