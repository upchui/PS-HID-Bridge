// When to hand the next mouse report to the USB controller.
//
// The host fetches the mouse endpoint once per frame (1 ms at full speed),
// always at about the same point in the frame. A report armed right after
// the previous fetch would sit in the controller for almost a frame, and
// motion arriving meanwhile would wait for the frame after that. Instead the
// report is armed shortly before the expected fetch, so it carries all
// motion up to then.
//
// The fetch times come from the "report complete" callback. From them the
// next fetch is predicted; the lead before it adapts: a missed fetch (the
// report went out a frame later) makes it longer, a long run without misses
// shorter. A miss costs one frame once and never loses motion.
#ifndef PSHB_USB_TIMING_H
#define PSHB_USB_TIMING_H

#include <stdbool.h>
#include <stdint.h>

#define USB_TIMING_PERIOD_US 1000
#define USB_TIMING_LEAD_START_US 150
#define USB_TIMING_LEAD_MIN_US 60
#define USB_TIMING_LEAD_MAX_US 500
#define USB_TIMING_LEAD_UP_US 50
#define USB_TIMING_LEAD_DOWN_US 10
// Fetches in a row without a miss before the lead gets shorter.
#define USB_TIMING_HITS_FOR_DOWN 2000
// A phase older than this is not trusted (clock drift, host pauses).
#define USB_TIMING_PHASE_VALID_US 1000000

typedef struct {
    uint64_t phase_us;     // estimated time of a fetch (0 = unknown)
    uint64_t armed_for_us; // fetch the armed report is meant for (0 = none/unknown)
    uint32_t lead_us;      // arm this long before the expected fetch
    uint32_t hits;         // fetches in a row without a miss
} usb_timing_t;

// Phase unknown, lead back to the start value.
void usb_timing_init(usb_timing_t *t);
// Phase unknown (after a bus reset, suspend, protocol change); keeps the lead.
void usb_timing_reset(usb_timing_t *t);
// The next fetch the host is expected to make after `now` (0 = unknown).
uint64_t usb_timing_next_fetch(const usb_timing_t *t, uint64_t now);
// True if a report should be armed now (always true while the phase is unknown).
bool usb_timing_should_arm(const usb_timing_t *t, uint64_t now);
// A report was armed at `now`.
void usb_timing_armed(usb_timing_t *t, uint64_t now);
// The host fetched the armed report at `now`. Returns true if it went out a
// frame (or more) later than planned, i.e. the planned fetch was missed.
bool usb_timing_fetched(usb_timing_t *t, uint64_t now);

#endif
