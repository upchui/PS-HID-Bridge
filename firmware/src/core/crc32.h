#ifndef PSHB_CRC32_H
#define PSHB_CRC32_H

#include <stddef.h>
#include <stdint.h>

// CRC-32 (IEEE 802.3, reflected, poly 0xEDB88320) -- same as zlib/JS.
// Incremental use: crc = crc32_update(0, a, n); crc = crc32_update(crc, b, m);
uint32_t crc32_update(uint32_t crc, const void *data, size_t len);

static inline uint32_t crc32(const void *data, size_t len) {
    return crc32_update(0, data, len);
}

#endif
