// Settings area in flash: two 4 KiB sectors at 2016 KiB (declared as a
// partition in partitions/pt.json, so firmware updates never touch it).
// The last 24 KiB stay unused: every SDK-built UF2 carries an RP2350-E10
// workaround block at 0x10ffff00, which on a 2 MiB flash lands in the last
// sector.
#ifndef PSHB_FLASH_STORE_H
#define PSHB_FLASH_STORE_H

#include "settings.h"

#define PSHB_SETTINGS_FLASH_OFFSET (PICO_FLASH_SIZE_BYTES - 32u * 1024u)

// Flash access for settings.c. Erase/program must only run on core 1 while
// core 0 executes from RAM (copy_to_ram build).
extern const pshb_flash_t g_settings_flash;

#endif
