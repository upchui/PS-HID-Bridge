#include "hid_out.h"

#include "pico/time.h"
#include "tusb.h"

#include "usb_descriptors.h"

static input_state_t *s_state;
static bool s_mouse_boot;
static uint8_t s_leds;
static absolute_time_t s_mounted_at;
static bool s_mounted;

void hid_out_init(input_state_t *state) {
    s_state = state;
}

void hid_out_service(void) {
    if (!s_mounted || tud_suspended()) {
        // Nobody is listening; don't let motion pile up.
        input_state_drop_motion(s_state);
        return;
    }

    if (input_state_kbd_pending(s_state) && tud_hid_n_ready(ITF_KEYBOARD)) {
        uint8_t report[PSHB_KBD_REPORT_LEN];
        input_state_take_kbd_report(s_state, report);
        tud_hid_n_report(ITF_KEYBOARD, 0, report, sizeof(report));
    }

    if (input_state_mouse_pending(s_state) && tud_hid_n_ready(ITF_MOUSE)) {
        uint8_t report[PSHB_MOUSE_REPORT_LEN];
        size_t len = input_state_take_mouse_report(s_state, report, s_mouse_boot);
        tud_hid_n_report(ITF_MOUSE, 0, report, (uint16_t)len);
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
    s_mounted = true;
    s_mounted_at = get_absolute_time();
    s_mouse_boot = false;  // report protocol is the default after configuration
    input_state_mark_dirty(s_state);
}

void tud_umount_cb(void) {
    s_mounted = false;
}

void tud_resume_cb(void) {
    input_state_mark_dirty(s_state);
}

void tud_hid_set_protocol_cb(uint8_t instance, uint8_t protocol) {
    if (instance == ITF_MOUSE) {
        s_mouse_boot = (protocol == HID_PROTOCOL_BOOT);
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
