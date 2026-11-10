// Build-time configuration of the PS5 HID Bridge firmware.
#ifndef PSHB_CONFIG_H
#define PSHB_CONFIG_H

// Firmware version reported in PONG: 0x00MMmmpp (major, minor, patch).
#ifndef PSHB_FW_VERSION
#define PSHB_FW_VERSION 0x000100
#endif

// Accept unencrypted protocol version 0 (debug/bring-up builds only).
#ifndef PSHB_PLAINTEXT_DEBUG
#define PSHB_PLAINTEXT_DEBUG 0
#endif

// W5500 SPI clock. clk_peri is 150 MHz, so 25 MHz and 37.5 MHz are exact.
#ifndef PSHB_W5500_SPI_HZ
#define PSHB_W5500_SPI_HZ (25 * 1000 * 1000)
#endif

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
