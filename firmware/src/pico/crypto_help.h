// Core 1 computes one of the two ChaCha20 blocks of a packet while core 0
// computes the other (pshb_chacha20_block2, core/aead.h).
#ifndef PSHB_CRYPTO_HELP_H
#define PSHB_CRYPTO_HELP_H

#include <stdint.h>

// Core 0, before core 1 is started: claim a doorbell and install the helper.
void crypto_help_init_core0(void);
// Core 1, at its start: take requests (doorbell interrupt).
void crypto_help_init_core1(void);

// Diagnostics: packets whose second block core 1 computed, and the ones
// core 0 computed alone (core 1 did not answer in time or was still busy).
uint32_t crypto_help_helped(void);
uint32_t crypto_help_alone(void);
void crypto_help_reset_counts(void);

#endif
