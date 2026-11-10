#include "ota.h"

#include <stdio.h>
#include <string.h>

#include "boot/picobin.h"
#include "boot/picoboot_constants.h"
#include "hardware/flash.h"
#include "hardware/regs/addressmap.h"
#include "hardware/sync.h"
#include "pico/bootrom.h"

#include "uf2_writer.h"

static __attribute__((aligned(4))) uint8_t s_workarea[4 * 1024];
static uf2_writer_t s_uf2;
static uint32_t s_slot_start;  // physical flash offset of the target slot
static uint32_t s_slot_size;
static bool s_running;
static bool s_ready;

static bool erase_sector(void *ctx, uint32_t off) {
    (void)ctx;
    uint32_t irq = save_and_disable_interrupts();
    flash_range_erase(s_slot_start + off, UF2_SECTOR_SIZE);
    restore_interrupts(irq);
    return true;
}

static bool program_page(void *ctx, uint32_t off, const uint8_t data[UF2_PAYLOAD_SIZE]) {
    (void)ctx;
    uint32_t irq = save_and_disable_interrupts();
    flash_range_program(s_slot_start + off, data, UF2_PAYLOAD_SIZE);
    restore_interrupts(irq);
    // Read back through the uncached alias.
    return memcmp((const void *)(XIP_NOCACHE_NOALLOC_NOTRANSLATE_BASE + s_slot_start + off), data,
                  UF2_PAYLOAD_SIZE) == 0;
}

static const uf2_flash_t k_flash = {NULL, erase_sector, program_page};

bool ota_begin(uint32_t size, char *err, size_t cap) {
    if (size == 0 || size % UF2_BLOCK_SIZE != 0) {
        snprintf(err, cap, "This is not a UF2 file.");
        return false;
    }
    resident_partition_t part;
    rom_flash_flush_cache();
    int r = rom_get_uf2_target_partition(s_workarea, sizeof(s_workarea), UF2_FAMILY_RP2350_ARM_S, &part);
    if (r < 0) {
        snprintf(err, cap,
                 "No partition table. Install ps5-hid-bridge-pt.uf2 once via BOOTSEL.");
        return false;
    }
    uint32_t first = (part.permissions_and_location & PICOBIN_PARTITION_LOCATION_FIRST_SECTOR_BITS) >>
                     PICOBIN_PARTITION_LOCATION_FIRST_SECTOR_LSB;
    uint32_t last = (part.permissions_and_location & PICOBIN_PARTITION_LOCATION_LAST_SECTOR_BITS) >>
                    PICOBIN_PARTITION_LOCATION_LAST_SECTOR_LSB;
    s_slot_start = first * FLASH_SECTOR_SIZE;
    s_slot_size = (last + 1 - first) * FLASH_SECTOR_SIZE;
    uf2_writer_init(&s_uf2, &k_flash, s_slot_size, UF2_FAMILY_RP2350_ARM_S);
    s_running = true;
    s_ready = false;
    printf("ota: writing to flash 0x%06lx (%lu KiB)\n", (unsigned long)s_slot_start,
           (unsigned long)(s_slot_size / 1024));
    return true;
}

bool ota_write(const uint8_t *data, size_t len) {
    return s_running && uf2_writer_feed(&s_uf2, data, len);
}

bool ota_finish(uint32_t crc, char *err, size_t cap) {
    if (!s_running) {
        snprintf(err, cap, "No update in progress.");
        return false;
    }
    uf2_result_t r = uf2_writer_finish(&s_uf2, crc);
    s_running = false;
    if (r != UF2_OK) {
        snprintf(err, cap, "%s", uf2_error_text(r));
        return false;
    }
    printf("ota: %lu blocks written and verified\n", (unsigned long)s_uf2.blocks_written);
    s_ready = true;
    return true;
}

void ota_abort(void) {
    if (s_running) printf("ota: aborted\n");
    s_running = false;
    s_ready = false;
}

void ota_activate(void) {
    if (!s_ready) return;
    printf("ota: rebooting into the new firmware\n");
    rom_reboot(REBOOT2_FLAG_REBOOT_TYPE_FLASH_UPDATE, 1000, XIP_BASE + s_slot_start, 0);
    for (;;) {
        __wfi();
    }
}

bool ota_confirm_if_trial(void) {
    if (rom_get_last_boot_type() != BOOT_TYPE_FLASH_UPDATE) return false;
    int r = rom_explicit_buy(s_workarea, sizeof(s_workarea));
    printf("ota: new firmware confirmed (%d)\n", r);
    return r == 0;
}

const char *ota_running_slot(void) {
    boot_info_t info = {0};
    if (rom_get_boot_info(&info) < 0) return "-";
    switch (info.partition) {
    case 0: return "A";
    case 1: return "B";
    default: return "-";
    }
}
