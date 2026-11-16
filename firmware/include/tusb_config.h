// TinyUSB configuration: full-speed device with two HID interfaces; in the
// controller-mode test images also a host (Pico-PIO-USB) for a DualSense.
#ifndef TUSB_CONFIG_H
#define TUSB_CONFIG_H

#include "pshb_config.h"

// CFG_TUSB_MCU and CFG_TUSB_OS are set by the pico-sdk's tinyusb integration.
#if PSHB_USBHOST_TEST
// Wiring test: the board's own USB only supplies power, no device stack.
#define CFG_TUD_ENABLED       0
#else
#define CFG_TUSB_RHPORT0_MODE (OPT_MODE_DEVICE | OPT_MODE_FULL_SPEED)
#define CFG_TUD_ENABLED       1
#endif
#define CFG_TUSB_DEBUG        0

#define CFG_TUD_ENDPOINT0_SIZE 64

#if PSHB_DS_PASSTHROUGH
// One HID interface, the DualSense's, passed through (dspass.c).
#define CFG_TUD_HID    0
#else
#define CFG_TUD_HID    2  // keyboard + mouse
#endif
#define CFG_TUD_CDC    0
#define CFG_TUD_MSC    0
#define CFG_TUD_MIDI   0
#define CFG_TUD_VENDOR 0

// Largest report is the 8-byte keyboard report.
#define CFG_TUD_HID_EP_BUFSIZE 8

// Events from the USB interrupt to tud_task(). With 16 (the default) a main
// loop stalled for ~8 ms (SPI mutex held by core 1) could drop a "transfer
// complete", which leaves that endpoint busy and the mouse dead until the
// next bus reset.
#define CFG_TUD_TASK_QUEUE_SZ 64

#if PSHB_USBHOST_TEST || PSHB_DS_PASSTHROUGH
// Host port: Pico-PIO-USB on root port 1 (D+ GP2, D- GP3), run by core 1.
#define CFG_TUSB_RHPORT1_MODE (OPT_MODE_HOST | OPT_MODE_FULL_SPEED)
#define CFG_TUH_RPI_PIO_USB   1
#define PSHB_TUH_RHPORT       1
#define CFG_TUH_DEVICE_MAX    1
#define CFG_TUH_HUB           0
#define CFG_TUH_HID           4
// The DualSense's configuration descriptor is 227 bytes, its HID report
// descriptor 273: with the default 256 the report descriptor is dropped.
#define CFG_TUH_ENUMERATION_BUFSIZE 512
#define CFG_TUH_HID_EPIN_BUFSIZE    64
#define CFG_TUH_HID_EPOUT_BUFSIZE   64
#endif

#endif
