#include "logbuf.h"

#include <string.h>

void logbuf_init(logbuf_t *b, char *mem, uint32_t cap) {
    b->mem = mem;
    b->cap = cap;
    b->head = 0;
}

void logbuf_write(logbuf_t *b, const char *s, uint32_t len) {
    if (b->cap == 0) return;
    if (len > b->cap) {  // only the end fits
        s += len - b->cap;
        b->head += len - b->cap;
        len = b->cap;
    }
    uint32_t at = (uint32_t)(b->head % b->cap);
    uint32_t first = b->cap - at < len ? b->cap - at : len;
    memcpy(b->mem + at, s, first);
    memcpy(b->mem, s + first, len - first);
    b->head += len;
}

uint64_t logbuf_tail(const logbuf_t *b) {
    return b->head > b->cap ? b->head - b->cap : 0;
}

uint32_t logbuf_read(const logbuf_t *b, uint64_t *pos, char *out, uint32_t max, int whole_lines,
                     uint64_t *lost) {
    uint64_t tail = logbuf_tail(b);
    if (lost) *lost = 0;
    if (*pos < tail) {
        if (lost) *lost = tail - *pos;
        *pos = tail;
    }
    if (*pos > b->head) *pos = b->head;  // a position from the future: nothing to read
    uint64_t avail = b->head - *pos;
    uint32_t n = avail < max ? (uint32_t)avail : max;
    if (n == 0) return 0;
    uint32_t at = (uint32_t)(*pos % b->cap);
    uint32_t first = b->cap - at < n ? b->cap - at : n;
    memcpy(out, b->mem + at, first);
    memcpy(out + first, b->mem, n - first);
    if (whole_lines) {
        uint32_t end = n;
        while (end > 0 && out[end - 1] != '\n') end--;
        // No complete line: wait for its end, unless it can never fit.
        if (end == 0) {
            if (n < max) return 0;
        } else {
            n = end;
        }
    }
    *pos += n;
    return n;
}
