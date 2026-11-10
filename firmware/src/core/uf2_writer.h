// Streaming UF2 writer for firmware updates over the network.
//
// Accepts the UF2 file in arbitrary pieces, keeps only blocks of the
// expected family (rp2350-arm-s), skips everything else (e.g. the RP2350-E10
// "absolute" block every SDK UF2 carries), checks addresses against the
// target slot, erases each 4 KiB sector before its first write and programs
// 256-byte pages. At the end every block must have arrived exactly once and
// the CRC-32 of the whole file must match.
#ifndef PSHB_UF2_WRITER_H
#define PSHB_UF2_WRITER_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define UF2_BLOCK_SIZE        512
#define UF2_PAYLOAD_SIZE      256
#define UF2_FAMILY_RP2350_ARM_S 0xe48bff59u
#define UF2_FAMILY_ABSOLUTE     0xe48bff57u
#define UF2_FLASH_BASE        0x10000000u
#define UF2_SECTOR_SIZE       4096u
#define UF2_MAX_SECTORS       512u  // 2 MiB

typedef struct {
    void *ctx;
    // Offsets are relative to the start of the target slot.
    bool (*erase_sector)(void *ctx, uint32_t offset);
    bool (*program_page)(void *ctx, uint32_t offset, const uint8_t data[UF2_PAYLOAD_SIZE]);
} uf2_flash_t;

typedef enum {
    UF2_OK = 0,
    UF2_ERR_MAGIC,       // not a UF2 file / corrupted block
    UF2_ERR_FAMILY,      // no block for our chip
    UF2_ERR_ADDRESS,     // block outside the slot or misaligned
    UF2_ERR_SEQUENCE,    // inconsistent block numbering / duplicates
    UF2_ERR_INCOMPLETE,  // blocks missing at the end
    UF2_ERR_TOO_BIG,     // image larger than the slot
    UF2_ERR_FLASH,       // erase/program failed
    UF2_ERR_CRC,         // file checksum mismatch
    UF2_ERR_SIZE,        // total size is not a multiple of 512
} uf2_result_t;

typedef struct {
    const uf2_flash_t *flash;
    uint32_t slot_size;
    uint32_t family;

    uint8_t block[UF2_BLOCK_SIZE];
    uint32_t block_fill;
    uint32_t total_bytes;
    uint32_t crc;

    uint32_t num_blocks;     // of our family, from the first block
    uint32_t blocks_written;
    uint8_t seen[UF2_MAX_SECTORS * (UF2_SECTOR_SIZE / UF2_PAYLOAD_SIZE) / 8];
    uint8_t erased[UF2_MAX_SECTORS / 8];
    uint32_t image_end;      // highest written offset + 256
    uf2_result_t error;
} uf2_writer_t;

void uf2_writer_init(uf2_writer_t *w, const uf2_flash_t *flash, uint32_t slot_size, uint32_t family);

// Feed file bytes. Returns false once an error occurred (see w->error).
bool uf2_writer_feed(uf2_writer_t *w, const uint8_t *data, size_t len);

// All data received: check completeness and the file CRC.
uf2_result_t uf2_writer_finish(uf2_writer_t *w, uint32_t expected_crc);

const char *uf2_error_text(uf2_result_t r);

#endif
