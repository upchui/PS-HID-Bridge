// Latency diagnostics of the input path (web interface, /api/diag,
// docs/latency.md). Written by core 0 only; core 1 reads them for the web
// interface (aligned 32-bit loads, a value that is being updated is at worst
// one sample off) and asks core 0 to start over through `reset_request`.
#ifndef PSHB_DIAG_H
#define PSHB_DIAG_H

#include <stdbool.h>
#include <stdint.h>

#include "latency.h"

typedef struct {
    latency_hist_t usb;     // INTn -> report fetched by the host
    latency_hist_t rx;      // packet noticed -> applied (SPI + decryption)
    latency_hist_t ready;   // INTn -> input applied: "packet -> report ready"
    latency_hist_t ready1;  // the same, first datagram of a round only (not queued behind others)
    latency_hist_t detect;  // INTn -> noticed by the main loop
    latency_hist_t bus;     // core 0 waits for the SPI bus (per receive round)
    latency_hist_t spi;     // reading one datagram over SPI
    latency_hist_t handle;  // checks + decryption + apply of one datagram
    latency_hist_t gap;     // time between two mouse reports fetched by the host
    latency_hist_t armed;   // mouse report armed -> fetched
    uint32_t rounds;        // receive rounds
    uint32_t datagrams;     // datagrams read
    uint32_t mouse_inputs;  // packets with mouse input
    uint32_t mouse_waits;   // ... of them arrived while a report was already armed
    uint32_t mouse_fetches; // mouse reports fetched by the host
    uint32_t mouse_misses;  // polls missed: motion waited, the report was armed a frame late
    uint32_t mouse_host_skips; // reports armed in time but fetched a frame later (host skipped)
    uint32_t mouse_lead_us; // reports are armed this long before the fetch (0 = right away)
    uint32_t mouse_floor_us; // ... and at least this long (main loop's longest absence + margin)
    // Decrypting one INPUT packet, measured once at start (core 1).
    uint32_t crypto_min_ns, crypto_avg_ns, crypto_max_ns;
    bool crypto_fast;  // the assembly ChaCha20 block is in use (passed its check)
    volatile bool reset_request;  // set by core 1, done by core 0 (diag_reset)
} pshb_diag_t;

extern pshb_diag_t g_diag;

// Fresh statistics (core 0; keeps the crypto measurement and the lead).
void diag_init(void);

#endif
