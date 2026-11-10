// HID report descriptors and report layouts of the bridge.
//
// Interface 0: boot-compatible keyboard, 8-byte report, no report ID:
//   [0] modifiers  [1] reserved  [2..7] key usages (page 0x07)
//   Output report (LEDs) 1 byte.
// Interface 1: mouse, no report ID:
//   16-bit: [0] buttons(5 bits) [1..2] dx i16 LE [3..4] dy i16 LE [5] wheel i8 [6] pan i8
//   8-bit:  [0] buttons(5 bits) [1] dx i8 [2] dy i8 [3] wheel i8 [4] pan i8
//   Boot protocol: [0] buttons(3 bits) [1] dx i8 [2] dy i8
#ifndef PSHB_HID_DESC_H
#define PSHB_HID_DESC_H

#include <stdint.h>

#include "pshb_config.h"

#define PSHB_KBD_REPORT_LEN        8
#define PSHB_MOUSE_BOOT_REPORT_LEN 3

#if PSHB_MOUSE_8BIT
#define PSHB_MOUSE_REPORT_LEN   5
#define PSHB_MOUSE_XY_MAX       127
#define PSHB_HID_MOUSE_DESC_LEN 71
#else
#define PSHB_MOUSE_REPORT_LEN   7
#define PSHB_MOUSE_XY_MAX       32767
#define PSHB_HID_MOUSE_DESC_LEN 73
#endif

#define PSHB_HID_KBD_DESC_LEN 65

extern const uint8_t pshb_hid_kbd_desc[PSHB_HID_KBD_DESC_LEN];
extern const uint8_t pshb_hid_mouse_desc[PSHB_HID_MOUSE_DESC_LEN];

#endif
