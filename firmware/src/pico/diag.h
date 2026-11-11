// Latency diagnostics of the input path (shown in the web interface).
// Written by core 0 only; core 1 reads them for the status page (aligned
// 32-bit loads, a value that is being updated is at worst one sample off).
#ifndef PSHB_DIAG_H
#define PSHB_DIAG_H

#include <stdint.h>

#include "latency.h"

typedef struct {
    latency_hist_t usb;          // packet arrival -> report fetched by the host
    latency_hist_t poll_offset;  // start of the USB frame (SOF) -> report fetched
    latency_hist_t rx;           // packet noticed -> applied (SPI + decryption)
    uint32_t mouse_waits;        // motion arrived while the last report was not fetched yet
} pshb_diag_t;

extern pshb_diag_t g_diag;

#endif
