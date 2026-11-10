#include "uf2_writer.h"

#include <string.h>

#include "crc32.h"
#include "protocol.h"

#define UF2_MAGIC_START0 0x0A324655u
#define UF2_MAGIC_START1 0x9E5D5157u
#define UF2_MAGIC_END    0x0AB16F30u
#define UF2_FLAG_NOT_MAIN_FLASH 0x00000001u
#define UF2_FLAG_FAMILY_ID      0x00002000u

void uf2_writer_init(uf2_writer_t *w, const uf2_flash_t *flash, uint32_t slot_size, uint32_t family) {
    memset(w, 0, sizeof(*w));
    w->flash = flash;
    w->slot_size = slot_size > UF2_MAX_SECTORS * UF2_SECTOR_SIZE ? UF2_MAX_SECTORS * UF2_SECTOR_SIZE : slot_size;
    w->family = family;
}

static bool fail(uf2_writer_t *w, uf2_result_t r) {
    if (w->error == UF2_OK) w->error = r;
    return false;
}

static bool bit_test_set(uint8_t *bits, uint32_t n) {
    bool was = bits[n / 8] & (1u << (n % 8));
    bits[n / 8] |= (uint8_t)(1u << (n % 8));
    return was;
}

static bool handle_block(uf2_writer_t *w, const uint8_t *b) {
    if (pshb_rd32(b) != UF2_MAGIC_START0 || pshb_rd32(b + 4) != UF2_MAGIC_START1 ||
        pshb_rd32(b + 508) != UF2_MAGIC_END) {
        return fail(w, UF2_ERR_MAGIC);
    }
    uint32_t flags = pshb_rd32(b + 8);
    uint32_t addr = pshb_rd32(b + 12);
    uint32_t size = pshb_rd32(b + 16);
    uint32_t block_no = pshb_rd32(b + 20);
    uint32_t num_blocks = pshb_rd32(b + 24);
    uint32_t family = pshb_rd32(b + 28);

    // Only blocks for our family go to flash; others (absolute E10 block,
    // RISC-V images in combined files, ...) are skipped.
    if ((flags & UF2_FLAG_NOT_MAIN_FLASH) || !(flags & UF2_FLAG_FAMILY_ID) || family != w->family) return true;

    if (size != UF2_PAYLOAD_SIZE || (addr & (UF2_PAYLOAD_SIZE - 1)) != 0 || addr < UF2_FLASH_BASE) {
        return fail(w, UF2_ERR_ADDRESS);
    }
    uint32_t off = addr - UF2_FLASH_BASE;
    if (off + UF2_PAYLOAD_SIZE > w->slot_size) return fail(w, UF2_ERR_TOO_BIG);

    if (w->num_blocks == 0) {
        if (num_blocks == 0 || num_blocks > w->slot_size / UF2_PAYLOAD_SIZE) return fail(w, UF2_ERR_TOO_BIG);
        w->num_blocks = num_blocks;
    }
    if (num_blocks != w->num_blocks || block_no >= num_blocks) return fail(w, UF2_ERR_SEQUENCE);
    if (bit_test_set(w->seen, block_no)) return fail(w, UF2_ERR_SEQUENCE);

    uint32_t sector = off / UF2_SECTOR_SIZE;
    if (!bit_test_set(w->erased, sector)) {
        if (!w->flash->erase_sector(w->flash->ctx, sector * UF2_SECTOR_SIZE)) return fail(w, UF2_ERR_FLASH);
    }
    if (!w->flash->program_page(w->flash->ctx, off, b + 32)) return fail(w, UF2_ERR_FLASH);
    w->blocks_written++;
    if (off + UF2_PAYLOAD_SIZE > w->image_end) w->image_end = off + UF2_PAYLOAD_SIZE;
    return true;
}

bool uf2_writer_feed(uf2_writer_t *w, const uint8_t *data, size_t len) {
    if (w->error != UF2_OK) return false;
    w->crc = crc32_update(w->crc, data, len);
    w->total_bytes += (uint32_t)len;
    while (len) {
        size_t n = UF2_BLOCK_SIZE - w->block_fill;
        if (n > len) n = len;
        memcpy(w->block + w->block_fill, data, n);
        w->block_fill += (uint32_t)n;
        data += n;
        len -= n;
        if (w->block_fill == UF2_BLOCK_SIZE) {
            w->block_fill = 0;
            if (!handle_block(w, w->block)) return false;
        }
    }
    return true;
}

uf2_result_t uf2_writer_finish(uf2_writer_t *w, uint32_t expected_crc) {
    if (w->error != UF2_OK) return w->error;
    if (w->block_fill != 0 || w->total_bytes == 0) return w->error = UF2_ERR_SIZE;
    if (w->num_blocks == 0) return w->error = UF2_ERR_FAMILY;
    if (w->blocks_written != w->num_blocks) return w->error = UF2_ERR_INCOMPLETE;
    if (w->crc != expected_crc) return w->error = UF2_ERR_CRC;
    return UF2_OK;
}

const char *uf2_error_text(uf2_result_t r) {
    switch (r) {
    case UF2_OK: return "OK";
    case UF2_ERR_MAGIC: return "Not a valid UF2 file";
    case UF2_ERR_FAMILY: return "The file contains no firmware for this board (RP2350 ARM)";
    case UF2_ERR_ADDRESS: return "Invalid addresses in the UF2 file";
    case UF2_ERR_SEQUENCE: return "UF2 blocks are duplicated or out of order";
    case UF2_ERR_INCOMPLETE: return "The UF2 file is incomplete";
    case UF2_ERR_TOO_BIG: return "The firmware is too large for the slot";
    case UF2_ERR_FLASH: return "Writing to flash failed";
    case UF2_ERR_CRC: return "Checksum mismatch (transfer error)";
    case UF2_ERR_SIZE: return "The file size does not match the UF2 format";
    }
    return "Unknown error";
}
