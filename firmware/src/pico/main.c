// PS5 HID Bridge firmware -- entry point (core 0).
//
// Step 1 skeleton: blink the on-board LED and print a banner on UART0.

#include <stdio.h>

#include "pico/stdlib.h"

int main(void) {
    stdio_init_all();
    gpio_init(PICO_DEFAULT_LED_PIN);
    gpio_set_dir(PICO_DEFAULT_LED_PIN, GPIO_OUT);

    printf("ps5-hid-bridge: boot\n");

    bool on = false;
    while (true) {
        on = !on;
        gpio_put(PICO_DEFAULT_LED_PIN, on);
        sleep_ms(500);
    }
}
