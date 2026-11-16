#include "hid_out.h"

#include "hardware/gpio.h"
#include "pico/time.h"
#include "tusb.h"

#include "pshb_config.h"

#include "diag.h"
#include "usb_descriptors.h"
#include "usb_timing.h"

// Arm mouse reports shortly before the host fetches them (usb_timing.h)
// instead of right after the previous fetch.
#ifndef PSHB_MOUSE_LATE_ARM
#define PSHB_MOUSE_LATE_ARM 1
#endif

static input_state_t *s_state;
// Diagnostics, per interface: arrival of the oldest input that is not in a
// report yet, and of the input in the report waiting for the host (0 = none).
static uint64_t s_pending_since[ITF_COUNT];
static uint64_t s_armed_since[ITF_COUNT];
static bool s_mouse_boot;
static uint8_t s_leds;
static absolute_time_t s_mounted_at;
static bool s_mounted;
static usb_timing_t s_mouse_timing;
static uint64_t s_mouse_armed_at, s_mouse_last_fetch;

// Floor for the lead: the longest the main loop stayed away from here in the
// last second (that is mostly receiving and decrypting a datagram), plus a
// margin: a report due meanwhile is armed that much late.
#define FLOOR_MARGIN_US 20
#define FLOOR_WINDOW_US 1000000
static uint64_t s_last_service, s_window_start;
static uint32_t s_window_max_gap;

static void track_loop_gap(uint64_t now) {
    if (s_last_service) {
        uint32_t gap = (uint32_t)(now - s_last_service);
        if (gap > s_window_max_gap) s_window_max_gap = gap;
    }
    s_last_service = now;
    if (now - s_window_start >= FLOOR_WINDOW_US) {
        usb_timing_set_floor(&s_mouse_timing, s_window_max_gap + FLOOR_MARGIN_US);
        g_diag.mouse_floor_us = s_mouse_timing.floor_us;
        s_window_start = now;
        s_window_max_gap = 0;
    }
}

void hid_out_init(input_state_t *state) {
    s_state = state;
    usb_timing_init(&s_mouse_timing);
}

void hid_out_note_input(uint64_t arrival_us) {
    if (!s_mounted) return;
    if (input_state_kbd_pending(s_state) && !s_pending_since[ITF_KEYBOARD]) {
        s_pending_since[ITF_KEYBOARD] = arrival_us;
    }
    if (input_state_mouse_pending(s_state)) {
        if (!s_pending_since[ITF_MOUSE]) s_pending_since[ITF_MOUSE] = arrival_us;
        g_diag.mouse_inputs++;
        if (!tud_hid_n_ready(ITF_MOUSE)) g_diag.mouse_waits++;
    }
}

// A report was handed to the USB controller.
static void armed(uint8_t itf, bool still_pending) {
    s_armed_since[itf] = s_pending_since[itf];
    // What is still queued arrived at the same time or later; keeping the
    // older time errs on the side of a longer latency.
    if (!still_pending) s_pending_since[itf] = 0;
}

void hid_out_service(void) {
    if (!s_mounted || tud_suspended()) {
        // Nobody is listening: no motion piling up, and only the current
        // state goes out once the host is back (no replay of old states).
        input_state_collapse(s_state);
        for (int i = 0; i < ITF_COUNT; i++) s_pending_since[i] = s_armed_since[i] = 0;
        usb_timing_reset(&s_mouse_timing);
        return;
    }

    // Input that cancelled out before it went into a report (motion back and
    // forth within a frame, a key pressed and released) is no longer
    // waiting: forget its arrival, or the next input would seem to have
    // waited since then.
    if (!input_state_kbd_pending(s_state)) s_pending_since[ITF_KEYBOARD] = 0;
    if (!input_state_mouse_pending(s_state)) s_pending_since[ITF_MOUSE] = 0;

    if (input_state_kbd_pending(s_state) && tud_hid_n_ready(ITF_KEYBOARD)) {
        uint8_t report[PSHB_KBD_REPORT_LEN];
        input_state_take_kbd_report(s_state, report);
        if (tud_hid_n_report(ITF_KEYBOARD, 0, report, sizeof(report))) {
            armed(ITF_KEYBOARD, input_state_kbd_pending(s_state));
        } else {
            input_state_mark_dirty(s_state);  // never lose the newest state
        }
    }

    uint64_t now = time_us_64();
    track_loop_gap(now);
    if (input_state_mouse_pending(s_state) && tud_hid_n_ready(ITF_MOUSE) &&
        (!PSHB_MOUSE_LATE_ARM || usb_timing_should_arm(&s_mouse_timing, now))) {
        uint8_t report[PSHB_MOUSE_REPORT_LEN];
        size_t len = input_state_take_mouse_report(s_state, report, s_mouse_boot);
        if (PSHB_MOUSE_LATE_ARM &&
            usb_timing_was_late(&s_mouse_timing, now, s_pending_since[ITF_MOUSE], s_mouse_last_fetch)) {
            g_diag.mouse_misses++;  // this motion should have gone out a frame earlier
            usb_timing_late(&s_mouse_timing);
        }
        if (tud_hid_n_report(ITF_MOUSE, 0, report, (uint16_t)len)) {
            armed(ITF_MOUSE, input_state_mouse_pending(s_state));
            usb_timing_armed(&s_mouse_timing, now);
            s_mouse_armed_at = now;
        } else {
            input_state_mark_dirty(s_state);  // never lose the newest state
        }
    }
}

bool hid_out_mounted(void) {
    return s_mounted;
}

uint32_t hid_out_mounted_ms(void) {
    if (!s_mounted) return 0;
    return (uint32_t)(absolute_time_diff_us(s_mounted_at, get_absolute_time()) / 1000);
}

uint8_t hid_out_leds(void) {
    return s_leds;
}

// ---------------------------------------------------------------- TinyUSB callbacks

void tud_mount_cb(void) {
    usb_timing_reset(&s_mouse_timing);
    s_mounted = true;
    s_mounted_at = get_absolute_time();
    s_mouse_boot = false;  // report protocol is the default after configuration
    // A bus reset does not unmount, so states kept piling up while the host
    // re-enumerated: send only the current state, no replay of old ones.
    input_state_collapse(s_state);
}

void tud_umount_cb(void) {
    s_mounted = false;
}

// The host fetched a report (both times are taken in tud_task(), so they
// are late by about the same main-loop delay).
void tud_hid_report_complete_cb(uint8_t instance, uint8_t const *report, uint16_t len) {
    (void)report;
    (void)len;
    if (instance >= ITF_COUNT) return;
    uint64_t now = time_us_64();
    if (instance == ITF_MOUSE) {
        g_diag.mouse_fetches++;
        if (s_mouse_last_fetch) latency_add(&g_diag.gap, (uint32_t)(now - s_mouse_last_fetch));
        s_mouse_last_fetch = now;
        if (s_mouse_armed_at) latency_add(&g_diag.armed, (uint32_t)(now - s_mouse_armed_at));
        s_mouse_armed_at = 0;
#if PSHB_TIMING_GPIO
        gpio_xor_mask(1u << PSHB_TIMING_PIN_FETCH);
#endif
        if (usb_timing_fetched(&s_mouse_timing, now)) g_diag.mouse_host_skips++;
#if PSHB_MOUSE_LATE_ARM
        g_diag.mouse_lead_us = s_mouse_timing.lead_us;
#endif
    }
    if (s_armed_since[instance]) {
        latency_add(&g_diag.usb, (uint32_t)(now - s_armed_since[instance]));
        s_armed_since[instance] = 0;
    }
}

void tud_resume_cb(void) {
    input_state_mark_dirty(s_state);
}

void tud_hid_set_protocol_cb(uint8_t instance, uint8_t protocol) {
    if (instance == ITF_MOUSE) {
        s_mouse_boot = (protocol == HID_PROTOCOL_BOOT);
        usb_timing_reset(&s_mouse_timing);
    }
    input_state_mark_dirty(s_state);
}

uint16_t tud_hid_get_report_cb(uint8_t instance, uint8_t report_id, hid_report_type_t report_type,
                               uint8_t *buffer, uint16_t reqlen) {
    (void)report_id;
    if (report_type != HID_REPORT_TYPE_INPUT) return 0;
    if (instance == ITF_KEYBOARD && reqlen >= PSHB_KBD_REPORT_LEN) {
        input_state_peek_kbd_report(s_state, buffer);
        return PSHB_KBD_REPORT_LEN;
    }
    if (instance == ITF_MOUSE && reqlen >= PSHB_MOUSE_REPORT_LEN) {
        // Buttons only; motion is never reported twice.
        for (int i = 0; i < PSHB_MOUSE_REPORT_LEN; i++) buffer[i] = 0;
        buffer[0] = s_state->buttons;
        return s_mouse_boot ? PSHB_MOUSE_BOOT_REPORT_LEN : PSHB_MOUSE_REPORT_LEN;
    }
    return 0;
}

void tud_hid_set_report_cb(uint8_t instance, uint8_t report_id, hid_report_type_t report_type,
                           uint8_t const *buffer, uint16_t bufsize) {
    (void)report_id;
    if (instance == ITF_KEYBOARD && bufsize >= 1 &&
        (report_type == HID_REPORT_TYPE_OUTPUT || report_type == HID_REPORT_TYPE_INVALID)) {
        s_leds = buffer[0];
    }
}
