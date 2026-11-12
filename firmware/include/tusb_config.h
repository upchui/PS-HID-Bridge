// TinyUSB configuration: full-speed device with two HID interfaces.
#ifndef TUSB_CONFIG_H
#define TUSB_CONFIG_H

// CFG_TUSB_MCU and CFG_TUSB_OS are set by the pico-sdk's tinyusb integration.
#define CFG_TUSB_RHPORT0_MODE (OPT_MODE_DEVICE | OPT_MODE_FULL_SPEED)
#define CFG_TUD_ENABLED       1
#define CFG_TUSB_DEBUG        0

#define CFG_TUD_ENDPOINT0_SIZE 64

#define CFG_TUD_HID    2  // keyboard + mouse
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

#endif
