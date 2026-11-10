// Input state between the network side and the USB side.
//
// Keyboard and mouse buttons are absolute state (last write wins).
// Mouse motion and wheels are relative: deltas accumulate until the next
// USB report; each report sends at most the representable range and the
// remainder is carried into the following report, so no motion is lost.
#ifndef PSHB_INPUT_STATE_H
#define PSHB_INPUT_STATE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "hid_desc.h"

#define PSHB_MOUSE_BUTTON_MASK 0x1F
// Accumulators saturate here so a stalled host cannot overflow them.
#define PSHB_MOTION_ACC_LIMIT (1L << 24)

typedef struct {
    uint8_t mods;
    uint8_t keys[6];
    uint8_t buttons;
    int32_t dx, dy, wheel, pan;
    bool kbd_dirty;
    bool buttons_dirty;
} input_state_t;

void input_state_init(input_state_t *s);

// Full keyboard state (modifier byte + 6 key slots, usage page 0x07).
void input_state_set_keyboard(input_state_t *s, uint8_t mods, const uint8_t keys[6]);
// Mouse buttons bit0..bit4 = left, right, middle, back, forward.
void input_state_set_buttons(input_state_t *s, uint8_t buttons);
// Relative motion; added to what is still pending.
void input_state_add_motion(input_state_t *s, int32_t dx, int32_t dy, int32_t wheel, int32_t pan);
// Release every key and button and drop pending motion (failsafe).
void input_state_release_all(input_state_t *s);
// Drop pending motion only (e.g. while USB is not mounted).
void input_state_drop_motion(input_state_t *s);
// Force both reports to be re-sent (e.g. after USB (re)mount).
void input_state_mark_dirty(input_state_t *s);

bool input_state_kbd_pending(const input_state_t *s);
bool input_state_mouse_pending(const input_state_t *s);

// Pack the keyboard report (PSHB_KBD_REPORT_LEN bytes) and clear the flag.
void input_state_take_kbd_report(input_state_t *s, uint8_t out[PSHB_KBD_REPORT_LEN]);
// Read-only variant for GET_REPORT requests.
void input_state_peek_kbd_report(const input_state_t *s, uint8_t out[PSHB_KBD_REPORT_LEN]);

// Pack one mouse report, consuming as much pending motion as fits.
// `out` must hold PSHB_MOUSE_REPORT_LEN bytes. Returns the report length
// (PSHB_MOUSE_BOOT_REPORT_LEN in boot protocol).
size_t input_state_take_mouse_report(input_state_t *s, uint8_t *out, bool boot_protocol);

#endif
