// Minimal unit test helpers for the host build.
//
//   TEST(name) { CHECK(cond); CHECK_EQ_U(a, b); CHECK_MEM(a, b, n); }
//   int main(void) { RUN(name); return TEST_SUMMARY(); }
#ifndef PSHB_MINI_TEST_H
#define PSHB_MINI_TEST_H

#include <inttypes.h>
#include <stdio.h>
#include <string.h>

static int mt_failures;
static int mt_checks;

#define TEST(name) static void name(void)

#define RUN(name)                                   \
    do {                                            \
        int before_ = mt_failures;                  \
        name();                                     \
        printf("%s %s\n", mt_failures == before_ ? "PASS" : "FAIL", #name); \
    } while (0)

#define CHECK(cond)                                                    \
    do {                                                               \
        mt_checks++;                                                   \
        if (!(cond)) {                                                 \
            mt_failures++;                                             \
            fprintf(stderr, "%s:%d: CHECK(%s) failed\n", __FILE__, __LINE__, #cond); \
        }                                                              \
    } while (0)

#define CHECK_EQ_U(a, b)                                               \
    do {                                                               \
        unsigned long long a_ = (unsigned long long)(a);               \
        unsigned long long b_ = (unsigned long long)(b);               \
        mt_checks++;                                                   \
        if (a_ != b_) {                                                \
            mt_failures++;                                             \
            fprintf(stderr, "%s:%d: %s == %s failed: 0x%llx != 0x%llx\n", \
                    __FILE__, __LINE__, #a, #b, a_, b_);               \
        }                                                              \
    } while (0)

#define CHECK_EQ_I(a, b)                                               \
    do {                                                               \
        long long a_ = (long long)(a);                                 \
        long long b_ = (long long)(b);                                 \
        mt_checks++;                                                   \
        if (a_ != b_) {                                                \
            mt_failures++;                                             \
            fprintf(stderr, "%s:%d: %s == %s failed: %lld != %lld\n",  \
                    __FILE__, __LINE__, #a, #b, a_, b_);               \
        }                                                              \
    } while (0)

static inline void mt_hexdump(const char *label, const void *p, size_t n) {
    const unsigned char *b = p;
    fprintf(stderr, "  %s:", label);
    for (size_t i = 0; i < n; i++) fprintf(stderr, " %02x", b[i]);
    fprintf(stderr, "\n");
}

#define CHECK_MEM(a, b, n)                                             \
    do {                                                               \
        mt_checks++;                                                   \
        if (memcmp((a), (b), (n)) != 0) {                              \
            mt_failures++;                                             \
            fprintf(stderr, "%s:%d: memory %s != %s\n", __FILE__, __LINE__, #a, #b); \
            mt_hexdump("got ", (a), (n));                              \
            mt_hexdump("want", (b), (n));                              \
        }                                                              \
    } while (0)

#define TEST_SUMMARY()                                                 \
    (printf("%d checks, %d failures\n", mt_checks, mt_failures), mt_failures ? 1 : 0)

#endif
