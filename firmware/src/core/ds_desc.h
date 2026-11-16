// USB descriptor helpers for the DualSense passthrough test image
// (pico/dspass.c): the configuration the board shows the PlayStation is
// built from the one read from the real DualSense.
#ifndef PSHB_DS_DESC_H
#define PSHB_DS_DESC_H

#include <stdint.h>

#define DS_DESC_CONFIGURATION 0x02
#define DS_DESC_INTERFACE     0x04
#define DS_DESC_ENDPOINT      0x05
#define DS_DESC_IAD           0x0B
#define DS_DESC_HID           0x21
#define DS_CLASS_AUDIO        0x01
#define DS_CLASS_HID          0x03

// Bytes from the interface descriptor `itf` up to the next interface with
// another number (or an IAD, or `max`): the interface with all its
// alternate settings, class-specific and endpoint descriptors. 0 if `itf`
// is not an interface descriptor or a descriptor runs past `max`.
uint16_t ds_desc_interface_span(const uint8_t *itf, uint16_t max);

// The first descriptor of `type` within the interface `itf_num` of the
// configuration `cfg` (alternate setting 0), or NULL.
const uint8_t *ds_desc_find(const uint8_t *cfg, uint16_t len, uint8_t itf_num, uint8_t type);

// The first interface of `cls` in `cfg`: its number, or -1.
int ds_desc_find_class(const uint8_t *cfg, uint16_t len, uint8_t cls);

// The configuration with only the interface `itf_num`, renumbered to 0
// (header: bNumInterfaces 1, wTotalLength). Returns the length written to
// `out`, 0 if it does not fit or `itf_num` is missing.
uint16_t ds_desc_single_interface(const uint8_t *cfg, uint16_t len, uint8_t itf_num, uint8_t *out, uint16_t cap);

#endif
