// USB device, configuration and string descriptors (TinyUSB callbacks).
//
// Composite device without IAD: interface 0 = boot keyboard,
// interface 1 = boot mouse. Both use 1 ms interrupt IN endpoints.

#include <string.h>

#include "pico/unique_id.h"
#include "tusb.h"

#include "hid_desc.h"
#include "pshb_config.h"
#include "usb_descriptors.h"

static const tusb_desc_device_t k_device = {
    .bLength = sizeof(tusb_desc_device_t),
    .bDescriptorType = TUSB_DESC_DEVICE,
    .bcdUSB = 0x0200,
    .bDeviceClass = 0x00,  // defined per interface
    .bDeviceSubClass = 0x00,
    .bDeviceProtocol = 0x00,
    .bMaxPacketSize0 = CFG_TUD_ENDPOINT0_SIZE,
    .idVendor = PSHB_USB_VID,
    .idProduct = PSHB_USB_PID,
    .bcdDevice = PSHB_USB_BCD_DEVICE,
    .iManufacturer = STRID_MANUFACTURER,
    .iProduct = STRID_PRODUCT,
    .iSerialNumber = STRID_SERIAL,
    .bNumConfigurations = 1,
};

uint8_t const *tud_descriptor_device_cb(void) {
    return (uint8_t const *)&k_device;
}

#define CONFIG_TOTAL_LEN (TUD_CONFIG_DESC_LEN + 2 * TUD_HID_DESC_LEN)

static const uint8_t k_configuration[] = {
    TUD_CONFIG_DESCRIPTOR(1, ITF_COUNT, 0, CONFIG_TOTAL_LEN, 0, PSHB_USB_MAX_POWER_MA),
    TUD_HID_DESCRIPTOR(ITF_KEYBOARD, STRID_KEYBOARD, HID_ITF_PROTOCOL_KEYBOARD,
                       PSHB_HID_KBD_DESC_LEN, EP_KEYBOARD_IN, 8, 1),
    TUD_HID_DESCRIPTOR(ITF_MOUSE, STRID_MOUSE, HID_ITF_PROTOCOL_MOUSE,
                       PSHB_HID_MOUSE_DESC_LEN, EP_MOUSE_IN, 8, 1),
};

uint8_t const *tud_descriptor_configuration_cb(uint8_t index) {
    (void)index;
    return k_configuration;
}

uint8_t const *tud_hid_descriptor_report_cb(uint8_t instance) {
    return instance == ITF_KEYBOARD ? pshb_hid_kbd_desc : pshb_hid_mouse_desc;
}

static const char *const k_strings[] = {
    [STRID_MANUFACTURER] = "PS5 HID Bridge",
    [STRID_PRODUCT] = "PS5 HID Bridge",
    [STRID_SERIAL] = NULL,  // filled from the flash unique id
    [STRID_KEYBOARD] = "Keyboard",
    [STRID_MOUSE] = "Mouse",
};

static uint16_t s_desc_str[33];

uint16_t const *tud_descriptor_string_cb(uint8_t index, uint16_t langid) {
    (void)langid;
    size_t len;

    if (index == 0) {
        s_desc_str[1] = 0x0409;  // English (US)
        len = 1;
    } else {
        char serial[2 * PICO_UNIQUE_BOARD_ID_SIZE_BYTES + 1];
        const char *str;
        if (index == STRID_SERIAL) {
            pico_get_unique_board_id_string(serial, sizeof(serial));
            str = serial;
        } else if (index < TU_ARRAY_SIZE(k_strings) && k_strings[index]) {
            str = k_strings[index];
        } else {
            return NULL;
        }
        len = strlen(str);
        if (len > 32) len = 32;
        for (size_t i = 0; i < len; i++) s_desc_str[1 + i] = (uint8_t)str[i];
    }

    s_desc_str[0] = (uint16_t)((TUSB_DESC_STRING << 8) | (2 * len + 2));
    return s_desc_str;
}
