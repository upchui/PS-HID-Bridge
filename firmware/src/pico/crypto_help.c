// Two ChaCha20 blocks, one per core. Both blocks of a packet depend only on
// key and nonce and take ~10 µs each on this CPU; computed side by side they
// cost core 0 one block instead of two (on the input path, while the rest
// of the datagram arrives by DMA, see w5500_port.c).
//
// Core 1 computes its block in a doorbell interrupt, so its own loop (web
// interface, DHCP) pauses for ~10 µs per packet. If it does not answer in
// time -- interrupts off, e.g. while it erases flash, or still busy with
// the previous request -- core 0 computes the block itself: nothing waits
// for core 1 longer than HELP_WAIT_US.
#include "crypto_help.h"

#include <string.h>

#include "hardware/irq.h"
#include "hardware/sync.h"
#include "pico/multicore.h"
#include "pico/time.h"

#include "aead.h"

// After its own block, core 0 waits at most this long for core 1's (which
// started at the same time and needs the same ~10 µs).
#define HELP_WAIT_US 6

static int s_bell = -1;
static volatile bool s_ready;          // core 1 takes requests
static volatile uint32_t s_req, s_done; // last request, last one answered
static uint32_t s_in[16], s_out[16];
static uint32_t s_helped, s_alone;

static void bell_irq(void) {
    multicore_doorbell_clear_current_core((uint)s_bell);
    uint32_t seq = s_req;
    __dmb();
    if (seq == s_done) return;
    uint32_t in[16], out[16];
    memcpy(in, s_in, sizeof(in));
    pshb_chacha20_block(out, in);
    memcpy(s_out, out, sizeof(out));
    pshb_wipe(in, sizeof(in));
    pshb_wipe(out, sizeof(out));
    __dmb();  // the result before the answer
    s_done = seq;
}

static void block2_two_cores(uint32_t out0[16], const uint32_t in0[16], uint32_t out1[16],
                             const uint32_t in1[16]) {
    if (!s_ready || s_done != s_req) {
        // Core 1 not there yet, or still on the previous request: never
        // write a request it might be reading.
        s_alone++;
        pshb_chacha20_block(out0, in0);
        pshb_chacha20_block(out1, in1);
        return;
    }
    memcpy(s_in, in1, sizeof(s_in));
    uint32_t seq = s_req + 1;
    __dmb();  // the request before its number
    s_req = seq;
    multicore_doorbell_set_other_core((uint)s_bell);
    pshb_chacha20_block(out0, in0);
    uint32_t start = time_us_32();
    while (s_done != seq) {
        if (time_us_32() - start > HELP_WAIT_US) {
            s_alone++;
            pshb_chacha20_block(out1, in1);
            return;
        }
    }
    __dmb();
    memcpy(out1, s_out, sizeof(s_out));
    pshb_wipe(s_out, sizeof(s_out));
    s_helped++;
}

void crypto_help_init_core0(void) {
    s_bell = multicore_doorbell_claim_unused(1u << 1, false);  // a doorbell of core 1
    if (s_bell >= 0) pshb_chacha20_block2 = block2_two_cores;
}

void crypto_help_init_core1(void) {
    if (s_bell < 0) return;
    uint irq = multicore_doorbell_irq_num((uint)s_bell);
    irq_set_exclusive_handler(irq, bell_irq);
    irq_set_enabled(irq, true);
    s_ready = true;
}

uint32_t crypto_help_helped(void) {
    return s_helped;
}

uint32_t crypto_help_alone(void) {
    return s_alone;
}

void crypto_help_reset_counts(void) {
    s_helped = 0;
    s_alone = 0;
}
