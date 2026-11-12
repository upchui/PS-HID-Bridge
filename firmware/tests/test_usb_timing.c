#include "mini_test.h"
#include "usb_timing.h"

#define T0 10000000ULL  // some time after boot
#define P USB_TIMING_PERIOD_US

TEST(unknown_phase_arms_at_once) {
    usb_timing_t t;
    usb_timing_init(&t);
    CHECK(usb_timing_should_arm(&t, T0));
    usb_timing_armed(&t, T0);
    CHECK(!usb_timing_fetched(&t, T0 + 700));  // no plan, no miss
    CHECK_EQ_U(t.phase_us, T0 + 700);
}

TEST(arms_shortly_before_the_next_fetch) {
    usb_timing_t t;
    usb_timing_init(&t);
    usb_timing_fetched(&t, T0);
    uint32_t lead = t.lead_us;
    CHECK(!usb_timing_should_arm(&t, T0 + 10));
    CHECK(!usb_timing_should_arm(&t, T0 + P - lead - 1));
    CHECK(usb_timing_should_arm(&t, T0 + P - lead));
    CHECK(usb_timing_should_arm(&t, T0 + P - 1));
    // Just after the expected fetch: wait for the one after it.
    CHECK(!usb_timing_should_arm(&t, T0 + P + 1));
    CHECK(usb_timing_should_arm(&t, T0 + 5 * P - lead));
    CHECK_EQ_U(usb_timing_next_fetch(&t, T0 + 3 * P + 400), T0 + 4 * P);
}

TEST(hit_keeps_lead_and_phase) {
    usb_timing_t t;
    usb_timing_init(&t);
    usb_timing_fetched(&t, T0);
    usb_timing_armed(&t, T0 + P - 100);
    CHECK_EQ_U(t.armed_for_us, T0 + P);
    CHECK(!usb_timing_fetched(&t, T0 + P + 8));  // 8 µs main-loop delay
    CHECK_EQ_U(t.lead_us, USB_TIMING_LEAD_START_US);
    CHECK(t.phase_us >= T0 + P && t.phase_us <= T0 + P + 1);  // a late measurement barely moves it
}

TEST(earlier_measurement_is_taken) {
    usb_timing_t t;
    usb_timing_init(&t);
    usb_timing_fetched(&t, T0 + 30);  // this one was measured late
    usb_timing_fetched(&t, T0 + P + 2);
    CHECK_EQ_U(t.phase_us, T0 + P + 2);
}

TEST(miss_makes_the_lead_longer) {
    usb_timing_t t;
    usb_timing_init(&t);
    usb_timing_fetched(&t, T0);
    usb_timing_armed(&t, T0 + P - 20);
    CHECK(usb_timing_fetched(&t, T0 + 2 * P + 5));  // went out a frame later
    CHECK_EQ_U(t.lead_us, USB_TIMING_LEAD_START_US + USB_TIMING_LEAD_UP_US);
    // The phase stays on the frame grid.
    uint64_t next = usb_timing_next_fetch(&t, T0 + 2 * P + 100);
    CHECK(next >= T0 + 3 * P && next <= T0 + 3 * P + 1);
    for (int i = 0; i < 50; i++) {
        uint64_t f = T0 + (uint64_t)(3 + 2 * i) * P;
        usb_timing_armed(&t, f - 1);
        usb_timing_fetched(&t, f + P);
    }
    CHECK_EQ_U(t.lead_us, USB_TIMING_LEAD_MAX_US);
}

TEST(long_run_of_hits_makes_the_lead_shorter) {
    usb_timing_t t;
    usb_timing_init(&t);
    usb_timing_fetched(&t, T0);
    uint64_t now = T0;
    for (int i = 0; i < 100 * USB_TIMING_HITS_FOR_DOWN; i++) {
        now += P;
        usb_timing_armed(&t, now - t.lead_us);
        CHECK(!usb_timing_fetched(&t, now));
    }
    CHECK_EQ_U(t.lead_us, USB_TIMING_LEAD_MIN_US);
}

TEST(follows_clock_drift) {
    // The host's frame is 1000.05 µs of the board's clock (after 20 s a
    // whole millisecond off a fixed grid), and the main loop notices each
    // fetch 0..19 µs late. The main loop runs every 10 µs and arms as soon
    // as it is told to; every report must be armed before its fetch.
    usb_timing_t t;
    usb_timing_init(&t);
    usb_timing_fetched(&t, T0);
    int late = 0, misses = 0;
    uint64_t prev = T0;
    for (int i = 1; i < 40000; i++) {
        uint64_t fetch = T0 + (uint64_t)i * P + (uint64_t)i / 20;
        uint64_t now = prev + 10;
        while (now < fetch && !usb_timing_should_arm(&t, now)) now += 10;
        if (now >= fetch) {
            late++;
            continue;
        }
        CHECK(fetch - now <= USB_TIMING_LEAD_MAX_US + 10);  // not armed early
        usb_timing_armed(&t, now);
        uint64_t noticed = fetch + (uint64_t)(i * 7) % 20;
        misses += usb_timing_fetched(&t, noticed);
        prev = noticed;
    }
    CHECK_EQ_U(late, 0);
    CHECK_EQ_U(misses, 0);
    CHECK(t.lead_us < USB_TIMING_LEAD_START_US);  // no misses: got shorter
}

TEST(old_phase_is_not_trusted) {
    usb_timing_t t;
    usb_timing_init(&t);
    usb_timing_fetched(&t, T0);
    CHECK(!usb_timing_should_arm(&t, T0 + USB_TIMING_PHASE_VALID_US - 500));
    CHECK(usb_timing_should_arm(&t, T0 + USB_TIMING_PHASE_VALID_US + 500));
    usb_timing_reset(&t);
    CHECK(usb_timing_should_arm(&t, T0 + 10));
}

int main(void) {
    RUN(unknown_phase_arms_at_once);
    RUN(arms_shortly_before_the_next_fetch);
    RUN(hit_keeps_lead_and_phase);
    RUN(earlier_measurement_is_taken);
    RUN(miss_makes_the_lead_longer);
    RUN(long_run_of_hits_makes_the_lead_shorter);
    RUN(follows_clock_drift);
    RUN(old_phase_is_not_trusted);
    return TEST_SUMMARY();
}
