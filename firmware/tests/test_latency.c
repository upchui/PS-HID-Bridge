#include "latency.h"
#include "mini_test.h"

TEST(empty) {
    latency_hist_t h;
    latency_init(&h, 50);
    CHECK_EQ_U(latency_percentile(&h, 50), 0);
    CHECK_EQ_U(h.count, 0);
}

TEST(percentiles_are_bucket_upper_edges) {
    latency_hist_t h;
    latency_init(&h, 50);
    for (uint32_t i = 0; i < 100; i++) latency_add(&h, i * 10);  // 0 .. 990 µs
    CHECK_EQ_U(h.count, 100);
    CHECK_EQ_U(h.max_us, 990);
    // 50th sample is 490 µs -> bucket 450..500
    CHECK_EQ_U(latency_percentile(&h, 50), 500);
    // 95th sample is 940 µs -> bucket 900..950
    CHECK_EQ_U(latency_percentile(&h, 95), 950);
    // never above the maximum
    CHECK_EQ_U(latency_percentile(&h, 100), 990);
}

TEST(overflow_bucket_reports_maximum) {
    latency_hist_t h;
    latency_init(&h, 10);
    latency_add(&h, 5);
    latency_add(&h, 100000);
    CHECK_EQ_U(latency_percentile(&h, 50), 10);  // upper edge of the first bucket
    CHECK_EQ_U(latency_percentile(&h, 100), 100000);
}

int main(void) {
    RUN(empty);
    RUN(percentiles_are_bucket_upper_edges);
    RUN(overflow_bucket_reports_maximum);
    return TEST_SUMMARY();
}
