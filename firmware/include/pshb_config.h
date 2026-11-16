// Build-time configuration of the PS HID Bridge firmware.
#ifndef PSHB_CONFIG_H
#define PSHB_CONFIG_H

// The firmware version (PSHB_FW_VERSION, 0xMMmmpppp) is in the generated
// pshb_version.h (cmake/version.cmake).

// Accept unencrypted protocol version 0 (debug/bring-up builds only).
#ifndef PSHB_PLAINTEXT_DEBUG
#define PSHB_PLAINTEXT_DEBUG 0
#endif

// Development only: pre-shared key as 64 hex digits, used until settings
// exist (step 6). Without it the board rejects every encrypted packet.
// #define PSHB_DEV_PSK "00112233..."

// W5500 SPI clock: the standard one, and the fast one (a setting in the web
// interface and the app, used after a self-test at start, see
// w5500_port.c). The SPI divider is even and clk_peri follows the CPU
// clock: at 150 MHz that gives 25 and 37.5 MHz (above the 33.3 MHz the
// W5500 datasheet guarantees), at 200 MHz 25 and 33.3 MHz.
#ifndef PSHB_W5500_SPI_HZ
#define PSHB_W5500_SPI_HZ (25 * 1000 * 1000)
#endif
#define PSHB_W5500_SPI_FAST_HZ (37500 * 1000)

// Measuring points (docs/latency.md), all off in the normal image:
// PSHB_TIMING_GPIO: GP6 is high from "INTn noticed" until the input is
//   applied, GP7 toggles on every mouse report the host fetches (GP2/GP3
//   belong to the USB host port, see PSHB_USBHOST_PIN_DP);
// PSHB_TIMING_UART: a latency summary on the debug UART every 10 s;
// PSHB_BENCH: POST /api/bench/sink takes uploads and throws them away
//   (network load like a firmware update, without flash writes).
#ifndef PSHB_TIMING_GPIO
#define PSHB_TIMING_GPIO 0
#endif
#define PSHB_TIMING_PIN_RX    6
#define PSHB_TIMING_PIN_FETCH 7
#ifndef PSHB_TIMING_UART
#define PSHB_TIMING_UART 0
#endif
#ifndef PSHB_BENCH
#define PSHB_BENCH 0
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

// Test image for the update rollback: never confirms itself after a
// network update, so the bootrom must fall back to the previous firmware.
#ifndef PSHB_OTA_NO_CONFIRM
#define PSHB_OTA_NO_CONFIRM 0
#endif

// Build the slow "mouse draws a square" test image (no network needed).
#ifndef PSHB_TEST_PATTERN
#define PSHB_TEST_PATTERN 0
#endif

// USB host port for a real DualSense (controller mode, see README): a USB-A
// socket with D+ on GP2 and D- on GP3, driven by Pico-PIO-USB on core 1.
// VBUS of the socket is the board's VBUS (pin 40).
#define PSHB_USBHOST_PIN_DP 2
// Test images of the controller mode (no input from the app):
// PSHB_USBHOST_TEST: the host port alone, logs what is plugged in;
// PSHB_DS_PASSTHROUGH: a DualSense on the host port is passed through to
// the PlayStation (its authentication included).
#ifndef PSHB_USBHOST_TEST
#define PSHB_USBHOST_TEST 0
#endif
#ifndef PSHB_DS_PASSTHROUGH
#define PSHB_DS_PASSTHROUGH 0
#endif

#endif
