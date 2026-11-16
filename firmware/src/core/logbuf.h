// Log ring buffer of the test images (controller mode, see netlog.c): keeps
// the last `cap` bytes of text. Positions are absolute byte counts since the
// start, so a reader (the network log, the UART) knows what it has missed
// when the writer overtook it.
//
// Not thread-safe by itself: the caller serialises writers and readers.
#ifndef PSHB_LOGBUF_H
#define PSHB_LOGBUF_H

#include <stdint.h>

typedef struct {
    char *mem;
    uint32_t cap;
    uint64_t head;  // bytes written since the start
} logbuf_t;

void logbuf_init(logbuf_t *b, char *mem, uint32_t cap);

// Append `len` bytes (the oldest ones are overwritten when full).
void logbuf_write(logbuf_t *b, const char *s, uint32_t len);

// Oldest position still kept.
uint64_t logbuf_tail(const logbuf_t *b);

// Copy text from position `*pos` on into `out` (at most `max` bytes) and
// advance `*pos`. If `*pos` was overwritten in the meantime, reading starts
// at the oldest kept byte and `*lost` (if not NULL) gets the number of bytes
// skipped. With `whole_lines`, only complete lines are copied (up to the last
// '\n'), unless a single line is longer than `max`. Returns the bytes copied.
uint32_t logbuf_read(const logbuf_t *b, uint64_t *pos, char *out, uint32_t max, int whole_lines,
                     uint64_t *lost);

#endif
