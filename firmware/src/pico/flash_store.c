#include "flash_store.h"

#include <string.h>

#include "hardware/flash.h"
#include "hardware/regs/addressmap.h"
#include "hardware/sync.h"

// Reads go through the uncached, untranslated XIP alias: with a partition
// table only the boot partition is mapped at XIP_BASE, and the cache could
// still hold the old contents right after programming.
static void fs_read(void *ctx, uint32_t off, void *buf, size_t len) {
    (void)ctx;
    memcpy(buf, (const void *)(XIP_NOCACHE_NOALLOC_NOTRANSLATE_BASE + PSHB_SETTINGS_FLASH_OFFSET + off), len);
}

static bool fs_erase(void *ctx, uint32_t off) {
    (void)ctx;
    uint32_t irq = save_and_disable_interrupts();
    flash_range_erase(PSHB_SETTINGS_FLASH_OFFSET + off, PSHB_SETTINGS_SECTOR);
    restore_interrupts(irq);
    return true;
}

static bool fs_program(void *ctx, uint32_t off, const void *data, size_t len) {
    (void)ctx;
    if (len % FLASH_PAGE_SIZE != 0) return false;
    uint32_t irq = save_and_disable_interrupts();
    flash_range_program(PSHB_SETTINGS_FLASH_OFFSET + off, data, len);
    restore_interrupts(irq);
    return true;
}

const pshb_flash_t g_settings_flash = {NULL, fs_read, fs_erase, fs_program};
