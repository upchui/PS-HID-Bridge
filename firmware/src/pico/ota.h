// Firmware update over the network using the RP2350 bootrom's A/B
// partitions with "try before you buy" (TBYB):
//  1. the UF2 is written into the partition the bootrom names as UF2 target
//     (always the one we are NOT running from),
//  2. a FLASH_UPDATE reboot boots the new image on trial,
//  3. the new firmware checks itself and "buys" the image; if it never does
//     (crash, hang), the next reset returns to the previous firmware.
#ifndef PSHB_OTA_H
#define PSHB_OTA_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

bool ota_begin(uint32_t size, char *err, size_t err_cap);
bool ota_write(const uint8_t *data, size_t len);
bool ota_finish(uint32_t crc, char *err, size_t err_cap);
void ota_abort(void);
// Reboot into the freshly written image (does not return).
void ota_activate(void);

// At boot (core 0, before core 1 starts): if we were booted on trial after
// an update, confirm the image. Returns true if an update was confirmed.
bool ota_confirm_if_trial(void);

// The flash has a partition table (ps-hid-bridge-pt.uf2 was installed).
// Without one there are no slots: an update overwrites the running firmware
// and there is nothing to fall back to.
bool ota_has_partition_table(void);

// "A", "B", "none" (no partition table) or "?" (not identified).
const char *ota_running_slot(void);

#endif
