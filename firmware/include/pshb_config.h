// Build-time configuration of the PS5 HID Bridge firmware.
#ifndef PSHB_CONFIG_H
#define PSHB_CONFIG_H

// USB identity. 1209:0001 is the pid.codes *Test PID*: fine for private,
// in-house use, but NOT unique -- do not distribute devices with it.
#define PSHB_USB_VID        0x1209
#define PSHB_USB_PID        0x0001
#define PSHB_USB_BCD_DEVICE 0x0100
#define PSHB_USB_MAX_POWER_MA 150

// Mouse X/Y resolution: 0 = 16-bit relative axes (default),
// 1 = 8-bit fallback in case a host rejects the 16-bit descriptor.
#ifndef PSHB_MOUSE_8BIT
#define PSHB_MOUSE_8BIT 0
#endif

// Build the slow "mouse draws a square" test image (no network needed).
#ifndef PSHB_TEST_PATTERN
#define PSHB_TEST_PATTERN 0
#endif

#endif
