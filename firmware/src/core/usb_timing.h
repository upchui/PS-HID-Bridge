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
// next fetch is predicted; the lead before it adapts:
//  * a missed poll -- motion was waiting before the window for a fetch
//    opened, the endpoint was free, and still the report was armed only
//    for the fetch after it (the main loop was busy) -- makes it longer;
//  * a long run of fetches without one makes it shorter, but never below
//    the floor: the longest time the main loop was away from USB lately
//    (receiving and decrypting a datagram) plus a margin.
// A report the host fetches a frame later than planned although it was
// armed in time means the host skipped a poll; that is counted, but the
// lead stays (a longer lead could not help, it only adds latency: on
// average an input waits half a frame plus the lead).
#ifndef PSHB_USB_TIMING_H
#define PSHB_USB_TIMING_H

#include <stdbool.h>
#include <stdint.h>

#define USB_TIMING_PERIOD_US 1000
#define USB_TIMING_LEAD_START_US 150
#define USB_TIMING_LEAD_MIN_US 60   // lowest floor
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
    uint32_t floor_us;     // the lead never gets shorter than this
    uint32_t hits;         // fetches in a row without a missed poll
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
// The host fetched the armed report at `now`. Returns true if that was a
// frame (or more) later than planned although it was armed in time: the
// host skipped a poll (the lead stays).
bool usb_timing_fetched(usb_timing_t *t, uint64_t now);

// Arming at `now` for the next fetch: was a poll missed? True if motion was
// waiting (since `pending_since`) before the window for the previous fetch
// opened, while the endpoint was already free (last fetch `last_fetch`).
bool usb_timing_was_late(const usb_timing_t *t, uint64_t now, uint64_t pending_since, uint64_t last_fetch);

// A poll was missed: make the lead longer.
void usb_timing_late(usb_timing_t *t);

// The lead's lower bound (clamped to LEAD_MIN..LEAD_MAX); raises the lead
// at once if it is below.
void usb_timing_set_floor(usb_timing_t *t, uint32_t floor_us);

#endif
