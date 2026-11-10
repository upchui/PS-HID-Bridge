// PS5 HID Bridge firmware -- entry point and core 0 loop.
//
// Core 0 owns the latency-critical input path: USB device stack and
// (from step 3) the UDP input socket. It never blocks.

#include <stdio.h>

#include "pico/stdlib.h"
#include "tusb.h"

#include "hid_out.h"
#include "input_state.h"
#include "pshb_config.h"
#if PSHB_TEST_PATTERN
#include "test_pattern.h"
#endif

static input_state_t g_input;

// LED: slow blink = waiting for USB host, solid = mounted.
static void led_service(void) {
    static absolute_time_t next;
    static bool on;
    if (hid_out_mounted()) {
        gpio_put(PICO_DEFAULT_LED_PIN, true);
        return;
    }
    if (time_reached(next)) {
        on = !on;
        gpio_put(PICO_DEFAULT_LED_PIN, on);
        next = make_timeout_time_ms(500);
    }
}

#if PSHB_TEST_PATTERN
// Moves the mouse along a square, starting a few seconds after mounting.
static void test_pattern_service(void) {
    static uint32_t step;
    static absolute_time_t next;
    if (!hid_out_mounted()) {
        step = 0;
        return;
    }
    if (hid_out_mounted_ms() < PSHB_TP_START_DELAY_MS || !time_reached(next)) return;
    int16_t dx, dy;
    test_pattern_delta(step++, &dx, &dy);
    input_state_add_motion(&g_input, dx, dy, 0, 0);
    next = make_timeout_time_ms(PSHB_TP_STEP_MS);
}
#endif

int main(void) {
    stdio_init_all();
    gpio_init(PICO_DEFAULT_LED_PIN);
    gpio_set_dir(PICO_DEFAULT_LED_PIN, GPIO_OUT);

    printf("ps5-hid-bridge: boot%s\n", PSHB_TEST_PATTERN ? " (test pattern)" : "");

    input_state_init(&g_input);
    hid_out_init(&g_input);
    tud_init(0);

    while (true) {
        tud_task();
#if PSHB_TEST_PATTERN
        test_pattern_service();
#endif
        hid_out_service();
        led_service();
    }
}
