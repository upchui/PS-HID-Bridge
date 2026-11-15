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
    CHECK_EQ_U(h.min_us, 0);
    CHECK_EQ_U(h.max_us, 990);
    // 50th sample is 490 µs -> bucket 450..500
    CHECK_EQ_U(latency_percentile(&h, 50), 500);
    // 95th sample is 940 µs -> bucket 900..950
    CHECK_EQ_U(latency_percentile(&h, 95), 950);
    // never above the maximum
    CHECK_EQ_U(latency_percentile(&h, 100), 990);
}

TEST(minimum_and_p99) {
    latency_hist_t h;
    latency_init(&h, 1);
    for (uint32_t i = 0; i < 200; i++) latency_add(&h, 20 + i % 10);  // 20 .. 29 µs
    latency_add(&h, 90);                                              // one slow one
    latency_add(&h, 95);
    CHECK_EQ_U(h.min_us, 20);
    CHECK_EQ_U(latency_percentile(&h, 50), 26);  // sample 101 of 202 is 25 µs -> bucket 25..26
    CHECK_EQ_U(latency_percentile(&h, 99), 30);  // rank 200 of 202: still 29 µs
    CHECK_EQ_U(latency_percentile(&h, 100), 95);
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
    RUN(minimum_and_p99);
    RUN(overflow_bucket_reports_maximum);
    return TEST_SUMMARY();
}
