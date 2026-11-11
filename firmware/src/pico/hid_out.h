// USB side of the input path: turns input_state into HID reports.
// Core 0 only.
#ifndef PSHB_HID_OUT_H
#define PSHB_HID_OUT_H

#include <stdbool.h>
#include <stdint.h>

#include "input_state.h"

void hid_out_init(input_state_t *state);

// Send pending reports if the endpoints are free. Call after tud_task().
void hid_out_service(void);

// Input that arrived at `arrival_us` was applied to the input state (for
// the latency diagnostics: arrival -> report fetched by the host).
void hid_out_note_input(uint64_t arrival_us);

bool hid_out_mounted(void);
// Milliseconds since the host configured the device (0 if not mounted).
uint32_t hid_out_mounted_ms(void);
// Keyboard LED state from the host (bit0 NumLock, bit1 CapsLock, ...).
uint8_t hid_out_leds(void);

#endif
