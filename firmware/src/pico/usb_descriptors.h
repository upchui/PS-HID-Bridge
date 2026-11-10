#ifndef PSHB_USB_DESCRIPTORS_H
#define PSHB_USB_DESCRIPTORS_H

// Interface numbers == TinyUSB HID instance numbers.
enum {
    ITF_KEYBOARD = 0,
    ITF_MOUSE = 1,
    ITF_COUNT,
};

#define EP_KEYBOARD_IN 0x81
#define EP_MOUSE_IN    0x82

enum {
    STRID_LANGID = 0,
    STRID_MANUFACTURER,
    STRID_PRODUCT,
    STRID_SERIAL,
    STRID_KEYBOARD,
    STRID_MOUSE,
};

#endif
