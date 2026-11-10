#include "hexutil.h"

static int nibble(char c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}

bool hex_decode(const char *s, uint8_t *out, size_t n) {
    for (size_t i = 0; i < n; i++) {
        int hi = nibble(s[2 * i]);
        if (hi < 0) return false;
        int lo = nibble(s[2 * i + 1]);
        if (lo < 0) return false;
        out[i] = (uint8_t)((hi << 4) | lo);
    }
    return s[2 * n] == '\0';
}

void hex_encode(const uint8_t *in, size_t n, char *out) {
    static const char digits[] = "0123456789abcdef";
    for (size_t i = 0; i < n; i++) {
        out[2 * i] = digits[in[i] >> 4];
        out[2 * i + 1] = digits[in[i] & 0x0F];
    }
    out[2 * n] = '\0';
}
