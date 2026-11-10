#include "input_state.h"

#include <string.h>

static int32_t clamp32(int32_t v, int32_t lo, int32_t hi) {
    return v < lo ? lo : (v > hi ? hi : v);
}

static int32_t sat_add(int32_t acc, int32_t delta) {
    int64_t v = (int64_t)acc + delta;
    if (v > PSHB_MOTION_ACC_LIMIT) return PSHB_MOTION_ACC_LIMIT;
    if (v < -PSHB_MOTION_ACC_LIMIT) return -PSHB_MOTION_ACC_LIMIT;
    return (int32_t)v;
}

// Take the part of *acc that fits into [-max, max]; keep the remainder.
static int32_t take(int32_t *acc, int32_t max) {
    int32_t part = clamp32(*acc, -max, max);
    *acc -= part;
    return part;
}

#if !PSHB_MOUSE_8BIT
static void put_le16(uint8_t *p, int32_t v) {
    uint16_t u = (uint16_t)(int16_t)v;
    p[0] = (uint8_t)(u & 0xFF);
    p[1] = (uint8_t)(u >> 8);
}
#endif

void input_state_init(input_state_t *s) {
    memset(s, 0, sizeof(*s));
    input_state_mark_dirty(s);
}

void input_state_set_keyboard(input_state_t *s, uint8_t mods, const uint8_t keys[6]) {
    if (s->mods != mods || memcmp(s->keys, keys, sizeof(s->keys)) != 0) {
        s->mods = mods;
        memcpy(s->keys, keys, sizeof(s->keys));
        s->kbd_dirty = true;
    }
}

void input_state_set_buttons(input_state_t *s, uint8_t buttons) {
    buttons &= PSHB_MOUSE_BUTTON_MASK;
    if (s->buttons != buttons) {
        s->buttons = buttons;
        s->buttons_dirty = true;
    }
}

void input_state_add_motion(input_state_t *s, int32_t dx, int32_t dy, int32_t wheel, int32_t pan) {
    s->dx = sat_add(s->dx, dx);
    s->dy = sat_add(s->dy, dy);
    s->wheel = sat_add(s->wheel, wheel);
    s->pan = sat_add(s->pan, pan);
}

void input_state_release_all(input_state_t *s) {
    static const uint8_t no_keys[6] = {0};
    input_state_set_keyboard(s, 0, no_keys);
    input_state_set_buttons(s, 0);
    input_state_drop_motion(s);
}

void input_state_drop_motion(input_state_t *s) {
    s->dx = s->dy = s->wheel = s->pan = 0;
}

void input_state_mark_dirty(input_state_t *s) {
    s->kbd_dirty = true;
    s->buttons_dirty = true;
}

bool input_state_kbd_pending(const input_state_t *s) {
    return s->kbd_dirty;
}

bool input_state_mouse_pending(const input_state_t *s) {
    return s->buttons_dirty || s->dx || s->dy || s->wheel || s->pan;
}

void input_state_peek_kbd_report(const input_state_t *s, uint8_t out[PSHB_KBD_REPORT_LEN]) {
    out[0] = s->mods;
    out[1] = 0;
    memcpy(&out[2], s->keys, 6);
}

void input_state_take_kbd_report(input_state_t *s, uint8_t out[PSHB_KBD_REPORT_LEN]) {
    input_state_peek_kbd_report(s, out);
    s->kbd_dirty = false;
}

size_t input_state_take_mouse_report(input_state_t *s, uint8_t *out, bool boot_protocol) {
    s->buttons_dirty = false;
    if (boot_protocol) {
        out[0] = s->buttons & 0x07;
        out[1] = (uint8_t)(int8_t)take(&s->dx, 127);
        out[2] = (uint8_t)(int8_t)take(&s->dy, 127);
        // The boot report has no wheels; drop them instead of letting them pile up.
        s->wheel = s->pan = 0;
        return PSHB_MOUSE_BOOT_REPORT_LEN;
    }

    out[0] = s->buttons;
#if PSHB_MOUSE_8BIT
    out[1] = (uint8_t)(int8_t)take(&s->dx, PSHB_MOUSE_XY_MAX);
    out[2] = (uint8_t)(int8_t)take(&s->dy, PSHB_MOUSE_XY_MAX);
    out[3] = (uint8_t)(int8_t)take(&s->wheel, 127);
    out[4] = (uint8_t)(int8_t)take(&s->pan, 127);
#else
    put_le16(&out[1], take(&s->dx, PSHB_MOUSE_XY_MAX));
    put_le16(&out[3], take(&s->dy, PSHB_MOUSE_XY_MAX));
    out[5] = (uint8_t)(int8_t)take(&s->wheel, 127);
    out[6] = (uint8_t)(int8_t)take(&s->pan, 127);
#endif
    return PSHB_MOUSE_REPORT_LEN;
}
