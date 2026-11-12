// Latency diagnostics of the input path (shown in the web interface).
// Written by core 0 only; core 1 reads them for the status page (aligned
// 32-bit loads, a value that is being updated is at worst one sample off).
#ifndef PSHB_DIAG_H
#define PSHB_DIAG_H

#include <stdint.h>

#include "latency.h"

typedef struct {
    latency_hist_t usb;          // packet arrival -> report fetched by the host
    latency_hist_t rx;           // packet noticed -> applied (SPI + decryption)
    uint32_t mouse_inputs;       // packets with mouse input
    uint32_t mouse_waits;        // ... of them arrived while a report was already armed
    uint32_t mouse_misses;       // reports armed too late for the fetch they were meant for
    uint32_t mouse_lead_us;      // reports are armed this long before the fetch (0 = right away)
} pshb_diag_t;

extern pshb_diag_t g_diag;

#endif
