#include "crc32.h"
#include "mini_test.h"

TEST(crc32_check_value) {
    // Standard check value for CRC-32/ISO-HDLC.
    CHECK_EQ_U(crc32("123456789", 9), 0xCBF43926u);
}

TEST(crc32_empty) {
    CHECK_EQ_U(crc32("", 0), 0u);
}

TEST(crc32_incremental) {
    const char *msg = "The quick brown fox jumps over the lazy dog";
    size_t n = strlen(msg);
    uint32_t whole = crc32(msg, n);
    CHECK_EQ_U(whole, 0x414FA339u);
    for (size_t split = 0; split <= n; split++) {
        uint32_t c = crc32_update(0, msg, split);
        c = crc32_update(c, msg + split, n - split);
        CHECK_EQ_U(c, whole);
    }
}

int main(void) {
    RUN(crc32_check_value);
    RUN(crc32_empty);
    RUN(crc32_incremental);
    return TEST_SUMMARY();
}
