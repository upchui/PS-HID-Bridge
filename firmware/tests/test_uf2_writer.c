// UF2 writer against a fake flash: synthetic files, real-world quirks
// (absolute E10 block, foreign families), corruption, and -- if available --
// the real firmware UF2 from the build (PSHB_TEST_UF2 / PSHB_TEST_BIN).
#include <stdlib.h>

#include "crc32.h"
#include "mini_test.h"
#include "protocol.h"
#include "uf2_writer.h"

#define SLOT_SIZE (1000u * 1024u)

static uint8_t flash[SLOT_SIZE];
static uint8_t erased_map[SLOT_SIZE / UF2_SECTOR_SIZE];
static int erase_calls, program_calls;

static bool f_erase(void *ctx, uint32_t off) {
    (void)ctx;
    memset(flash + off, 0xFF, UF2_SECTOR_SIZE);
    erased_map[off / UF2_SECTOR_SIZE] = 1;
    erase_calls++;
    return true;
}

static bool f_program(void *ctx, uint32_t off, const uint8_t data[UF2_PAYLOAD_SIZE]) {
    (void)ctx;
    if (!erased_map[off / UF2_SECTOR_SIZE]) return false;  // must erase first
    for (int i = 0; i < UF2_PAYLOAD_SIZE; i++) flash[off + i] &= data[i];
    program_calls++;
    return true;
}

static const uf2_flash_t F = {NULL, f_erase, f_program};

static void reset_flash(void) {
    memset(flash, 0x00, sizeof(flash));  // garbage that must be erased
    memset(erased_map, 0, sizeof(erased_map));
    erase_calls = program_calls = 0;
}

static void make_block(uint8_t *b, uint32_t addr, uint32_t no, uint32_t total, uint32_t family, uint8_t fill) {
    memset(b, 0, UF2_BLOCK_SIZE);
    pshb_wr32(b + 0, 0x0A324655u);
    pshb_wr32(b + 4, 0x9E5D5157u);
    pshb_wr32(b + 8, 0x00002000u);
    pshb_wr32(b + 12, addr);
    pshb_wr32(b + 16, 256);
    pshb_wr32(b + 20, no);
    pshb_wr32(b + 24, total);
    pshb_wr32(b + 28, family);
    for (int i = 0; i < 256; i++) b[32 + i] = (uint8_t)(fill + i);
    pshb_wr32(b + 508, 0x0AB16F30u);
}

// File: optional absolute block first (like the SDK's E10 workaround), then n image blocks.
static size_t make_file(uint8_t *out, uint32_t n, bool abs_block) {
    size_t len = 0;
    if (abs_block) {
        make_block(out, 0x10FFFF00u, 0, 1, UF2_FAMILY_ABSOLUTE, 0xEE);
        len += UF2_BLOCK_SIZE;
    }
    for (uint32_t i = 0; i < n; i++) {
        make_block(out + len, UF2_FLASH_BASE + i * 256, i, n, UF2_FAMILY_RP2350_ARM_S, (uint8_t)i);
        len += UF2_BLOCK_SIZE;
    }
    return len;
}

static uf2_result_t run(const uint8_t *file, size_t len, size_t chunk, uint32_t crc) {
    static uf2_writer_t w;
    uf2_writer_init(&w, &F, SLOT_SIZE, UF2_FAMILY_RP2350_ARM_S);
    for (size_t off = 0; off < len; off += chunk) {
        size_t n = len - off < chunk ? len - off : chunk;
        if (!uf2_writer_feed(&w, file + off, n)) break;
    }
    return uf2_writer_finish(&w, crc);
}

TEST(synthetic_image_any_chunking) {
    static uint8_t file[(40 + 1) * UF2_BLOCK_SIZE];
    size_t len = make_file(file, 40, true);
    uint32_t crc = crc32(file, len);
    size_t chunks[] = {1, 7, 333, 512, 1024, 100000};
    for (size_t i = 0; i < sizeof(chunks) / sizeof(chunks[0]); i++) {
        reset_flash();
        CHECK_EQ_I(run(file, len, chunks[i], crc), UF2_OK);
        CHECK_EQ_I(program_calls, 40);
        CHECK_EQ_I(erase_calls, 3);  // 40 pages = 10 KiB = 3 sectors
        for (uint32_t b = 0; b < 40; b++) {
            CHECK_EQ_U(flash[b * 256], (uint8_t)b);
            CHECK_EQ_U(flash[b * 256 + 255], (uint8_t)(b + 255));
        }
    }
}

TEST(absolute_block_is_not_written) {
    static uint8_t file[2 * UF2_BLOCK_SIZE];
    size_t len = make_file(file, 1, true);
    reset_flash();
    CHECK_EQ_I(run(file, len, len, crc32(file, len)), UF2_OK);
    CHECK_EQ_I(program_calls, 1);
}

TEST(errors) {
    static uint8_t file[(20 + 1) * UF2_BLOCK_SIZE];
    size_t len = make_file(file, 20, false);
    uint32_t crc = crc32(file, len);

    reset_flash();
    CHECK_EQ_I(run(file, len, 512, crc ^ 1), UF2_ERR_CRC);
    CHECK_EQ_I(run(file, len - UF2_BLOCK_SIZE, 512, crc32(file, len - UF2_BLOCK_SIZE)), UF2_ERR_INCOMPLETE);
    CHECK_EQ_I(run(file, len - 100, 512, crc32(file, len - 100)), UF2_ERR_SIZE);

    // Duplicate block.
    static uint8_t dup[21 * UF2_BLOCK_SIZE];
    memcpy(dup, file, len);
    memcpy(dup + len, file, UF2_BLOCK_SIZE);
    CHECK_EQ_I(run(dup, len + UF2_BLOCK_SIZE, 512, crc32(dup, len + UF2_BLOCK_SIZE)), UF2_ERR_SEQUENCE);

    // Bad magic.
    uint8_t save = file[3 * UF2_BLOCK_SIZE + 508];
    file[3 * UF2_BLOCK_SIZE + 508] ^= 0xFF;
    CHECK_EQ_I(run(file, len, 512, crc32(file, len)), UF2_ERR_MAGIC);
    file[3 * UF2_BLOCK_SIZE + 508] = save;

    // Address beyond the slot.
    static uint8_t big[UF2_BLOCK_SIZE];
    make_block(big, UF2_FLASH_BASE + SLOT_SIZE, 0, 1, UF2_FAMILY_RP2350_ARM_S, 0);
    CHECK_EQ_I(run(big, sizeof(big), 512, crc32(big, sizeof(big))), UF2_ERR_TOO_BIG);

    // Only foreign families (e.g. an RP2040 image).
    static uint8_t foreign[UF2_BLOCK_SIZE];
    make_block(foreign, UF2_FLASH_BASE, 0, 1, 0xe48bff56u, 0);
    CHECK_EQ_I(run(foreign, sizeof(foreign), 512, crc32(foreign, sizeof(foreign))), UF2_ERR_FAMILY);

    // Misaligned address.
    static uint8_t mis[UF2_BLOCK_SIZE];
    make_block(mis, UF2_FLASH_BASE + 10, 0, 1, UF2_FAMILY_RP2350_ARM_S, 0);
    CHECK_EQ_I(run(mis, sizeof(mis), 512, crc32(mis, sizeof(mis))), UF2_ERR_ADDRESS);
}

static uint8_t *read_file(const char *path, size_t *len) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *buf = malloc((size_t)n);
    if (fread(buf, 1, (size_t)n, f) != (size_t)n) {
        free(buf);
        fclose(f);
        return NULL;
    }
    fclose(f);
    *len = (size_t)n;
    return buf;
}

TEST(real_firmware_uf2_matches_bin) {
    const char *uf2_path = getenv("PSHB_TEST_UF2");
    const char *bin_path = getenv("PSHB_TEST_BIN");
    if (!uf2_path || !bin_path) {
        printf("  (skipped: PSHB_TEST_UF2/PSHB_TEST_BIN not set)\n");
        return;
    }
    size_t uf2_len = 0, bin_len = 0;
    uint8_t *uf2 = read_file(uf2_path, &uf2_len);
    uint8_t *bin = read_file(bin_path, &bin_len);
    CHECK(uf2 && bin);
    if (!uf2 || !bin) return;
    reset_flash();
    CHECK_EQ_I(run(uf2, uf2_len, 1000, crc32(uf2, uf2_len)), UF2_OK);
    CHECK(bin_len <= SLOT_SIZE);
    CHECK(memcmp(flash, bin, bin_len) == 0);
    free(uf2);
    free(bin);
}

int main(void) {
    RUN(synthetic_image_any_chunking);
    RUN(absolute_block_is_not_written);
    RUN(errors);
    RUN(real_firmware_uf2_matches_bin);
    return TEST_SUMMARY();
}
