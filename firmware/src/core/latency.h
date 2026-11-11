// Small latency histogram for the board's diagnostics (portable, no heap).
// Values are in microseconds; percentiles are reported as the upper edge of
// their bucket, so they never understate the latency.
#ifndef PSHB_LATENCY_H
#define PSHB_LATENCY_H

#include <stdint.h>

#define PSHB_LAT_BUCKETS 64

typedef struct {
    uint32_t bucket_us;                  // width of one bucket
    uint32_t count;
    uint32_t max_us;
    uint32_t buckets[PSHB_LAT_BUCKETS];  // last bucket also takes everything above
} latency_hist_t;

void latency_init(latency_hist_t *h, uint32_t bucket_us);
void latency_add(latency_hist_t *h, uint32_t us);
// pct = 1..100; 0 if nothing was recorded.
uint32_t latency_percentile(const latency_hist_t *h, unsigned pct);

#endif
