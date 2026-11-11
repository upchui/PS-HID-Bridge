#include "latency.h"

#include <string.h>

void latency_init(latency_hist_t *h, uint32_t bucket_us) {
    memset(h, 0, sizeof(*h));
    h->bucket_us = bucket_us ? bucket_us : 1;
}

void latency_add(latency_hist_t *h, uint32_t us) {
    uint32_t i = us / h->bucket_us;
    if (i >= PSHB_LAT_BUCKETS) i = PSHB_LAT_BUCKETS - 1;
    h->buckets[i]++;
    h->count++;
    if (us > h->max_us) h->max_us = us;
}

uint32_t latency_percentile(const latency_hist_t *h, unsigned pct) {
    if (h->count == 0) return 0;
    if (pct > 100) pct = 100;
    // Rank of the sample we are looking for (nearest rank, at least 1).
    uint64_t rank = ((uint64_t)h->count * pct + 99) / 100;
    if (rank == 0) rank = 1;
    uint64_t seen = 0;
    for (uint32_t i = 0; i < PSHB_LAT_BUCKETS; i++) {
        seen += h->buckets[i];
        if (seen >= rank) {
            // The last bucket is open-ended: its bound is the maximum.
            if (i == PSHB_LAT_BUCKETS - 1) return h->max_us;
            uint32_t edge = (i + 1) * h->bucket_us;
            return edge < h->max_us ? edge : h->max_us;
        }
    }
    return h->max_us;
}
