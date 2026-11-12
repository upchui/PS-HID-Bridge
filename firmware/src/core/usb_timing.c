#include "usb_timing.h"

void usb_timing_init(usb_timing_t *t) {
    t->lead_us = USB_TIMING_LEAD_START_US;
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
    uint64_t next = usb_timing_next_fetch(t, now);
    return !next || now + t->lead_us >= next;
}

void usb_timing_armed(usb_timing_t *t, uint64_t now) {
    t->armed_for_us = usb_timing_next_fetch(t, now);
}

bool usb_timing_fetched(usb_timing_t *t, uint64_t now) {
    bool missed = false;
    uint64_t planned = t->armed_for_us;
    t->armed_for_us = 0;
    if (planned && now > planned + USB_TIMING_PERIOD_US / 2) {
        missed = true;
        t->hits = 0;
        t->lead_us += USB_TIMING_LEAD_UP_US;
        if (t->lead_us > USB_TIMING_LEAD_MAX_US) t->lead_us = USB_TIMING_LEAD_MAX_US;
    } else if (planned && ++t->hits >= USB_TIMING_HITS_FOR_DOWN) {
        t->hits = 0;
        if (t->lead_us >= USB_TIMING_LEAD_MIN_US + USB_TIMING_LEAD_DOWN_US) {
            t->lead_us -= USB_TIMING_LEAD_DOWN_US;
        }
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
    return missed;
}
