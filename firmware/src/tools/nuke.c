// Recovery image (runs from RAM): erase the whole flash, then return to the
// USB bootloader. Use when nothing else helps: BOOTSEL, copy
// ps5-hid-bridge-nuke.uf2, then install partition table and firmware again.

#include "hardware/flash.h"
#include "pico/bootrom.h"
#include "pico/stdlib.h"

int main(void) {
    gpio_init(PICO_DEFAULT_LED_PIN);
    gpio_set_dir(PICO_DEFAULT_LED_PIN, GPIO_OUT);
    gpio_put(PICO_DEFAULT_LED_PIN, 1);
    flash_range_erase(0, PICO_FLASH_SIZE_BYTES);
    gpio_put(PICO_DEFAULT_LED_PIN, 0);
    reset_usb_boot(0, 0);
    return 0;
}
