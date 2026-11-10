#ifndef PSHB_HEXUTIL_H
#define PSHB_HEXUTIL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// Decode exactly 2*n hex digits (upper or lower case) into n bytes.
// Returns false on any other input; `out` may be partially written then.
bool hex_decode(const char *s, uint8_t *out, size_t n);

// Encode n bytes as 2*n lowercase hex digits plus a terminating NUL.
void hex_encode(const uint8_t *in, size_t n, char *out);

#endif
