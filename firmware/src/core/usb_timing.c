#include "usb_timing.h"

void usb_timing_init(usb_timing_t *t) {
    t->lead_us = USB_TIMING_LEAD_START_US;
    t->floor_us = USB_TIMING_LEAD_MIN_US;
    usb_timing_reset(t);
}

void usb_timing_reset(usb_timing_t *t) {
    t->phase_us = 0;
    t->armed_for_us = 0;
    t->hits = 0;
}

uint64_t usb_timing_next_fetch(const usb_timing_t *t, uint64_t now) {
    if (!t->phase_us || now < t->phase_us || now - t->phase_us > USB_TIMING_PHASE_VALID_US) return 0;
    uint64_t frames = (now - t->phase_us) / USB_TIMING_PERIOD_US + 1;
    return t->phase_us + frames * USB_TIMING_PERIOD_US;
}

bool usb_timing_should_arm(const usb_timing_t *t, uint64_t now) {
    return usb_timing_due_within(t, now, 0);
}

bool usb_timing_due_within(const usb_timing_t *t, uint64_t now, uint32_t within_us) {
    uint64_t next = usb_timing_next_fetch(t, now);
    return !next || now + within_us + t->lead_us >= next;
}

void usb_timing_armed(usb_timing_t *t, uint64_t now) {
    t->armed_for_us = usb_timing_next_fetch(t, now);
}

bool usb_timing_fetched(usb_timing_t *t, uint64_t now) {
    uint64_t planned = t->armed_for_us;
    t->armed_for_us = 0;
    bool skipped = planned && now > planned + USB_TIMING_PERIOD_US / 2;
    if (planned && !skipped && ++t->hits >= USB_TIMING_HITS_FOR_DOWN) {
        t->hits = 0;
        if (t->lead_us >= t->floor_us + USB_TIMING_LEAD_DOWN_US) t->lead_us -= USB_TIMING_LEAD_DOWN_US;
    }

    // New phase. The callback runs in the main loop, so every measured time
    // is late by a varying amount: an earlier measurement is closer to the
    // real fetch and is taken as it is; a later one only moves the phase a
    // little (follows clock drift without jumping on every slow loop).
    uint64_t expected = usb_timing_next_fetch(t, now - USB_TIMING_PERIOD_US / 2);
    if (!expected || now <= expected) {
        t->phase_us = now;
    } else {
        t->phase_us = expected + (now - expected + 15) / 16;  // rounded up: drift of < 1 µs per frame
    }
    return skipped;
}

bool usb_timing_was_late(const usb_timing_t *t, uint64_t now, uint64_t pending_since, uint64_t last_fetch) {
    uint64_t target = usb_timing_next_fetch(t, now);
    if (!target || !pending_since || target < USB_TIMING_PERIOD_US + t->lead_us) return false;
    // The window for the fetch before `target` opened here.
    uint64_t window = target - USB_TIMING_PERIOD_US - t->lead_us;
    return pending_since < window && last_fetch < window;
}

void usb_timing_late(usb_timing_t *t) {
    t->hits = 0;
    t->lead_us += USB_TIMING_LEAD_UP_US;
    if (t->lead_us > USB_TIMING_LEAD_MAX_US) t->lead_us = USB_TIMING_LEAD_MAX_US;
}

void usb_timing_set_floor(usb_timing_t *t, uint32_t floor_us) {
    if (floor_us < USB_TIMING_LEAD_MIN_US) floor_us = USB_TIMING_LEAD_MIN_US;
    if (floor_us > USB_TIMING_LEAD_MAX_US) floor_us = USB_TIMING_LEAD_MAX_US;
    t->floor_us = floor_us;
    if (t->lead_us < floor_us) t->lead_us = floor_us;
}
