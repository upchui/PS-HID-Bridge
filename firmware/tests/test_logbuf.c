#include "logbuf.h"
#include "mini_test.h"

static char mem[16];

TEST(reads_what_was_written) {
    logbuf_t b;
    logbuf_init(&b, mem, sizeof mem);
    logbuf_write(&b, "ab\n", 3);
    char out[32];
    uint64_t pos = 0, lost = 99;
    CHECK_EQ_U(logbuf_read(&b, &pos, out, sizeof out, 0, &lost), 3);
    CHECK_MEM(out, "ab\n", 3);
    CHECK_EQ_U(pos, 3);
    CHECK_EQ_U(lost, 0);
    CHECK_EQ_U(logbuf_read(&b, &pos, out, sizeof out, 0, &lost), 0);
}

TEST(wraps_around_and_counts_what_was_lost) {
    logbuf_t b;
    logbuf_init(&b, mem, sizeof mem);
    logbuf_write(&b, "0123456789", 10);
    uint64_t pos = 0;
    char out[32];
    CHECK_EQ_U(logbuf_read(&b, &pos, out, 4, 0, NULL), 4);  // "0123"
    logbuf_write(&b, "abcdefghij", 10);                     // 20 bytes: 0-3 gone
    uint64_t lost = 0;
    CHECK_EQ_U(logbuf_tail(&b), 4);
    CHECK_EQ_U(logbuf_read(&b, &pos, out, sizeof out, 0, &lost), 16);
    CHECK_EQ_U(lost, 0);  // the reader was at 4, the oldest kept byte
    CHECK_MEM(out, "456789abcdefghij", 16);

    logbuf_write(&b, "KLMNOPQRSTUVWXYZ0123", 20);  // longer than the buffer
    pos = 10;
    CHECK_EQ_U(logbuf_read(&b, &pos, out, sizeof out, 0, &lost), 16);
    CHECK_EQ_U(lost, 40 - 16 - 10);
    CHECK_MEM(out, "OPQRSTUVWXYZ0123", 16);
    CHECK_EQ_U(pos, 40);
}

TEST(whole_lines_only) {
    logbuf_t b;
    logbuf_init(&b, mem, sizeof mem);
    logbuf_write(&b, "one\ntwo\nthr", 11);
    uint64_t pos = 0;
    char out[32];
    CHECK_EQ_U(logbuf_read(&b, &pos, out, sizeof out, 1, NULL), 8);
    CHECK_MEM(out, "one\ntwo\n", 8);
    CHECK_EQ_U(logbuf_read(&b, &pos, out, sizeof out, 1, NULL), 0);  // "thr" not finished
    logbuf_write(&b, "ee\n", 3);
    CHECK_EQ_U(logbuf_read(&b, &pos, out, sizeof out, 1, NULL), 6);
    CHECK_MEM(out, "three\n", 6);
    // A line longer than the reader's buffer comes in pieces.
    logbuf_write(&b, "abcdefgh\n", 9);
    CHECK_EQ_U(logbuf_read(&b, &pos, out, 4, 1, NULL), 4);
    CHECK_MEM(out, "abcd", 4);
}

TEST(position_beyond_the_end_reads_nothing) {
    logbuf_t b;
    logbuf_init(&b, mem, sizeof mem);
    logbuf_write(&b, "x\n", 2);
    uint64_t pos = 100;
    char out[8];
    CHECK_EQ_U(logbuf_read(&b, &pos, out, sizeof out, 0, NULL), 0);
    CHECK_EQ_U(pos, 2);
}

int main(void) {
    RUN(reads_what_was_written);
    RUN(wraps_around_and_counts_what_was_lost);
    RUN(whole_lines_only);
    RUN(position_beyond_the_end_reads_nothing);
    return TEST_SUMMARY();
}
