#include "input_state.h"
#include "mini_test.h"

#if !PSHB_MOUSE_8BIT
static int16_t rd16(const uint8_t *p) {
    return (int16_t)(uint16_t)(p[0] | (p[1] << 8));
}
#endif

// Decode a report (report protocol) back into numbers.
typedef struct {
    int buttons, dx, dy, wheel, pan;
} mouse_t;

static mouse_t take_mouse(input_state_t *s) {
    uint8_t r[PSHB_MOUSE_REPORT_LEN];
    size_t n = input_state_take_mouse_report(s, r, false);
    mouse_t m;
    CHECK_EQ_U(n, PSHB_MOUSE_REPORT_LEN);
    m.buttons = r[0];
#if PSHB_MOUSE_8BIT
    m.dx = (int8_t)r[1];
    m.dy = (int8_t)r[2];
    m.wheel = (int8_t)r[3];
    m.pan = (int8_t)r[4];
#else
    m.dx = rd16(&r[1]);
    m.dy = rd16(&r[3]);
    m.wheel = (int8_t)r[5];
    m.pan = (int8_t)r[6];
#endif
    return m;
}

TEST(fresh_state_sends_empty_reports) {
    input_state_t s;
    input_state_init(&s);
    CHECK(input_state_kbd_pending(&s));
    CHECK(input_state_mouse_pending(&s));
    uint8_t r[PSHB_KBD_REPORT_LEN];
    static const uint8_t zero[PSHB_KBD_REPORT_LEN] = {0};
    input_state_take_kbd_report(&s, r);
    CHECK_MEM(r, zero, sizeof(r));
    CHECK(!input_state_kbd_pending(&s));
    mouse_t m = take_mouse(&s);
    CHECK_EQ_I(m.buttons | m.dx | m.dy | m.wheel | m.pan, 0);
    CHECK(!input_state_mouse_pending(&s));
}

TEST(keyboard_report_layout) {
    input_state_t s;
    input_state_init(&s);
    uint8_t r[PSHB_KBD_REPORT_LEN];
    input_state_take_kbd_report(&s, r);

    const uint8_t keys[6] = {0x04, 0x1D, 0, 0, 0, 0};  // A, Z
    input_state_set_keyboard(&s, 0x02, keys);           // left shift
    CHECK(input_state_kbd_pending(&s));
    input_state_take_kbd_report(&s, r);
    const uint8_t want[PSHB_KBD_REPORT_LEN] = {0x02, 0x00, 0x04, 0x1D, 0, 0, 0, 0};
    CHECK_MEM(r, want, sizeof(want));

    // Same state again: nothing new to send.
    input_state_set_keyboard(&s, 0x02, keys);
    CHECK(!input_state_kbd_pending(&s));
}

TEST(motion_accumulates_between_reports) {
    input_state_t s;
    input_state_init(&s);
    take_mouse(&s);
    input_state_add_motion(&s, 10, 20, 1, 0);
    input_state_add_motion(&s, 5, -3, 1, -1);
    CHECK(input_state_mouse_pending(&s));
    mouse_t m = take_mouse(&s);
    CHECK_EQ_I(m.dx, 15);
    CHECK_EQ_I(m.dy, 17);
    CHECK_EQ_I(m.wheel, 2);
    CHECK_EQ_I(m.pan, -1);
    CHECK(!input_state_mouse_pending(&s));
}

TEST(large_motion_is_split_without_loss) {
    input_state_t s;
    input_state_init(&s);
    take_mouse(&s);
    const int total_x = 100000, total_y = -70001;
    input_state_add_motion(&s, total_x, total_y, 300, -200);
    long sx = 0, sy = 0, sw = 0, sp = 0;
    int reports = 0;
    while (input_state_mouse_pending(&s) && reports < 10000) {
        mouse_t m = take_mouse(&s);
        CHECK(m.dx <= PSHB_MOUSE_XY_MAX && m.dx >= -PSHB_MOUSE_XY_MAX);
        CHECK(m.dy <= PSHB_MOUSE_XY_MAX && m.dy >= -PSHB_MOUSE_XY_MAX);
        sx += m.dx;
        sy += m.dy;
        sw += m.wheel;
        sp += m.pan;
        reports++;
    }
    CHECK_EQ_I(sx, total_x);
    CHECK_EQ_I(sy, total_y);
    CHECK_EQ_I(sw, 300);
    CHECK_EQ_I(sp, -200);
    // ceil(100000 / max) reports are needed for X.
    CHECK_EQ_I(reports, (total_x + PSHB_MOUSE_XY_MAX - 1) / PSHB_MOUSE_XY_MAX);
}

TEST(negative_delta_encoding) {
    input_state_t s;
    input_state_init(&s);
    take_mouse(&s);
    input_state_add_motion(&s, -2, 1, -1, 0);
    uint8_t r[PSHB_MOUSE_REPORT_LEN];
    input_state_take_mouse_report(&s, r, false);
#if PSHB_MOUSE_8BIT
    const uint8_t want[] = {0x00, 0xFE, 0x01, 0xFF, 0x00};
#else
    const uint8_t want[] = {0x00, 0xFE, 0xFF, 0x01, 0x00, 0xFF, 0x00};
#endif
    CHECK_MEM(r, want, sizeof(want));
}

TEST(accumulator_saturates) {
    input_state_t s;
    input_state_init(&s);
    for (int i = 0; i < 1000; i++) input_state_add_motion(&s, INT32_MAX / 2, INT32_MIN / 2, 0, 0);
    CHECK_EQ_I(s.dx, PSHB_MOTION_ACC_LIMIT);
    CHECK_EQ_I(s.dy, -PSHB_MOTION_ACC_LIMIT);
}

TEST(buttons_are_masked_and_tracked) {
    input_state_t s;
    input_state_init(&s);
    take_mouse(&s);
    input_state_set_buttons(&s, 0xFF);
    CHECK(input_state_mouse_pending(&s));
    mouse_t m = take_mouse(&s);
    CHECK_EQ_I(m.buttons, 0x1F);
    input_state_set_buttons(&s, 0x1F);
    CHECK(!input_state_mouse_pending(&s));
}

TEST(boot_protocol_report) {
    input_state_t s;
    input_state_init(&s);
    uint8_t initial[PSHB_MOUSE_REPORT_LEN];
    input_state_take_mouse_report(&s, initial, true);  // the initial empty state
    input_state_set_buttons(&s, 0x1F);
    input_state_add_motion(&s, 300, -5, 4, 4);
    uint8_t r[PSHB_MOUSE_REPORT_LEN];
    size_t n = input_state_take_mouse_report(&s, r, true);
    CHECK_EQ_U(n, PSHB_MOUSE_BOOT_REPORT_LEN);
    CHECK_EQ_U(r[0], 0x07);  // only 3 buttons in boot protocol
    CHECK_EQ_I((int8_t)r[1], 127);
    CHECK_EQ_I((int8_t)r[2], -5);
    CHECK_EQ_I(s.wheel, 0);  // wheels dropped
    CHECK_EQ_I(s.dx, 300 - 127);
}

TEST(release_all_clears_everything) {
    input_state_t s;
    input_state_init(&s);
    uint8_t r[PSHB_KBD_REPORT_LEN];
    input_state_take_kbd_report(&s, r);
    take_mouse(&s);
    const uint8_t keys[6] = {0x04, 0, 0, 0, 0, 0};
    input_state_set_keyboard(&s, 0x01, keys);
    input_state_set_buttons(&s, 0x01);
    input_state_add_motion(&s, 50, 50, 1, 1);
    input_state_take_kbd_report(&s, r);
    take_mouse(&s);
    input_state_add_motion(&s, 7, 7, 0, 0);

    input_state_release_all(&s);
    CHECK(input_state_kbd_pending(&s));
    input_state_take_kbd_report(&s, r);
    static const uint8_t zero[PSHB_KBD_REPORT_LEN] = {0};
    CHECK_MEM(r, zero, sizeof(r));
    mouse_t m = take_mouse(&s);
    CHECK_EQ_I(m.buttons, 0);
    CHECK_EQ_I(m.dx, 0);
    CHECK(!input_state_mouse_pending(&s));
}

TEST(mark_dirty_resends_state) {
    input_state_t s;
    input_state_init(&s);
    uint8_t r[PSHB_KBD_REPORT_LEN];
    input_state_take_kbd_report(&s, r);
    take_mouse(&s);
    input_state_mark_dirty(&s);
    CHECK(input_state_kbd_pending(&s));
    CHECK(input_state_mouse_pending(&s));
}

TEST(short_tap_within_one_frame_is_kept) {
    input_state_t s;
    input_state_init(&s);
    uint8_t r[PSHB_KBD_REPORT_LEN];
    input_state_take_kbd_report(&s, r);
    take_mouse(&s);
    const uint8_t a[6] = {0x04, 0, 0, 0, 0, 0}, none[6] = {0};
    input_state_set_keyboard(&s, 0, a);     // down ...
    input_state_set_keyboard(&s, 0, none);  // ... and up before the next USB frame
    input_state_set_buttons(&s, 0x01);
    input_state_set_buttons(&s, 0x00);
    input_state_take_kbd_report(&s, r);
    CHECK_EQ_U(r[2], 0x04);
    CHECK(input_state_kbd_pending(&s));
    input_state_take_kbd_report(&s, r);
    CHECK_EQ_U(r[2], 0x00);
    CHECK(!input_state_kbd_pending(&s));
    CHECK_EQ_I(take_mouse(&s).buttons, 0x01);
    CHECK_EQ_I(take_mouse(&s).buttons, 0x00);
    CHECK(!input_state_mouse_pending(&s));
}

TEST(queue_overflow_keeps_newest) {
    input_state_t s;
    input_state_init(&s);
    uint8_t r[PSHB_KBD_REPORT_LEN];
    input_state_take_kbd_report(&s, r);
    for (uint8_t k = 1; k <= 20; k++) {
        const uint8_t keys[6] = {k, 0, 0, 0, 0, 0};
        input_state_set_keyboard(&s, 0, keys);
    }
    int n = 0;
    uint8_t last = 0;
    while (input_state_kbd_pending(&s)) {
        input_state_take_kbd_report(&s, r);
        last = r[2];
        n++;
    }
    CHECK_EQ_I(n, PSHB_STATE_QUEUE_LEN);
    CHECK_EQ_U(last, 20);  // the final state always arrives
}

TEST(collapse_keeps_only_the_current_state) {
    // USB asleep while input keeps coming: on wake-up only the current
    // state may go out, not a replay of what happened meanwhile.
    input_state_t s;
    input_state_init(&s);
    uint8_t r[PSHB_KBD_REPORT_LEN];
    input_state_take_kbd_report(&s, r);
    for (uint8_t k = 4; k < 10; k++) {
        const uint8_t keys[6] = {k, 0, 0, 0, 0, 0};
        input_state_set_keyboard(&s, 0, keys);
        input_state_set_buttons(&s, k & 1);
    }
    input_state_add_motion(&s, 5, 5, 0, 0);
    input_state_collapse(&s);
    int n = 0;
    while (input_state_kbd_pending(&s)) {
        input_state_take_kbd_report(&s, r);
        n++;
    }
    CHECK_EQ_I(n, 1);
    CHECK_EQ_U(r[2], 9);
    uint8_t m[PSHB_MOUSE_REPORT_LEN];
    CHECK(input_state_mouse_pending(&s));
    input_state_take_mouse_report(&s, m, false);
    CHECK_EQ_U(m[0], 1);  // buttons of the last state
    CHECK(!input_state_mouse_pending(&s));  // motion dropped, one button report
}

int main(void) {
    RUN(collapse_keeps_only_the_current_state);
    RUN(fresh_state_sends_empty_reports);
    RUN(keyboard_report_layout);
    RUN(motion_accumulates_between_reports);
    RUN(large_motion_is_split_without_loss);
    RUN(negative_delta_encoding);
    RUN(accumulator_saturates);
    RUN(buttons_are_masked_and_tracked);
    RUN(boot_protocol_report);
    RUN(release_all_clears_everything);
    RUN(mark_dirty_resends_state);
    RUN(short_tap_within_one_frame_is_kept);
    RUN(queue_overflow_keeps_newest);
    return TEST_SUMMARY();
}
