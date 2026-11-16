#include "json.h"
#include "mini_test.h"

#include <limits.h>
#include <stdio.h>

TEST(writer) {
    char buf[128];
    json_writer_t w;
    jw_begin(&w, buf, sizeof(buf));
    jw_str(&w, "name", "a\"b\\c\n");
    jw_int(&w, "n", -42);
    jw_bool(&w, "ok", true);
    size_t n = jw_end(&w);
    CHECK(n > 0);
    CHECK(strcmp(buf, "{\"name\":\"a\\\"b\\\\c\\u000a\",\"n\":-42,\"ok\":true}") == 0);

    char tiny[8];
    jw_begin(&w, tiny, sizeof(tiny));
    jw_str(&w, "long", "value");
    CHECK_EQ_U(jw_end(&w), 0);  // overflow detected
}

TEST(writer_int_roundtrip) {
    static const long long vals[] = {0, 1, -1, 45045, 4294967295LL, 4294967296LL,
                                     LLONG_MAX, LLONG_MIN};
    static const char *const want[] = {"0", "1", "-1", "45045", "4294967295", "4294967296",
                                       "9223372036854775807", "-9223372036854775808"};
    for (size_t i = 0; i < sizeof(vals) / sizeof(vals[0]); i++) {
        char buf[64], exp[64];
        json_writer_t w;
        jw_begin(&w, buf, sizeof(buf));
        jw_int(&w, "v", vals[i]);
        size_t n = jw_end(&w);
        CHECK(n > 0);
        snprintf(exp, sizeof(exp), "{\"v\":%s}", want[i]);
        CHECK(strcmp(buf, exp) == 0);
        long long back;
        CHECK(jr_int(buf, n, "v", &back) && back == vals[i]);
    }
}

TEST(reader) {
    const char *j = " { \"mode\" : \"static\", \"udp_port\": 45045, \"on\": false,"
                    " \"pw\": \"Gr\\u00fc\\\"n\\\\\", \"x\": null, \"umlaut\": \"Grün\" } ";
    size_t len = strlen(j);
    char s[32];
    long long v;
    bool b;
    CHECK(jr_str(j, len, "mode", s, sizeof(s)) && strcmp(s, "static") == 0);
    CHECK(jr_int(j, len, "udp_port", &v) && v == 45045);
    CHECK(jr_bool(j, len, "on", &b) && !b);
    CHECK(jr_str(j, len, "pw", s, sizeof(s)) && strcmp(s, "Grü\"n\\") == 0);  // \u escape -> UTF-8
    CHECK(jr_str(j, len, "umlaut", s, sizeof(s)) && strcmp(s, "Grün") == 0);  // raw UTF-8 kept
    CHECK(!jr_str(j, len, "udp_port", s, sizeof(s)));  // wrong type
    CHECK(!jr_int(j, len, "missing", &v));
    CHECK(!jr_str(j, len, "mode", s, 3));               // too small
}

TEST(reader_unicode_escapes) {
    char s[16];
    // The same text escaped (Python's json.dumps) and raw (a browser).
    const char *a = "{\"a\":\"\\u20ac \\ud83d\\ude00\"}";
    CHECK(jr_str(a, strlen(a), "a", s, sizeof(s)) && strcmp(s, "\xe2\x82\xac \xf0\x9f\x98\x80") == 0);
    const char *nul = "{\"a\":\"x\\u0000y\"}";
    CHECK(!jr_str(nul, strlen(nul), "a", s, sizeof(s)));  // would cut the text short
    const char *lone = "{\"a\":\"\\ud83d\"}";
    CHECK(!jr_str(lone, strlen(lone), "a", s, sizeof(s)));  // high surrogate alone
    const char *low = "{\"a\":\"\\ude00\"}";
    CHECK(!jr_str(low, strlen(low), "a", s, sizeof(s)));
    const char *tight = "{\"a\":\"\\u00fc\"}";
    CHECK(!jr_str(tight, strlen(tight), "a", s, 2));  // two bytes need room for the NUL too
    CHECK(jr_str(tight, strlen(tight), "a", s, 3) && strcmp(s, "\xc3\xbc") == 0);
}

TEST(reader_rejects_garbage) {
    long long v;
    char s[8];
    CHECK(!jr_int("", 0, "a", &v));
    CHECK(!jr_int("[1]", 3, "a", &v));
    CHECK(!jr_int("{\"a\":1.5}", 9, "a", &v));
    CHECK(!jr_str("{\"a\":\"x", 7, "a", s, sizeof(s)));
    CHECK(!jr_int("{\"a\" 1}", 7, "a", &v));
    CHECK(!jr_str("{\"a\":\"\x01\"}", 8, "a", s, sizeof(s)));
}

TEST(reader_checks_what_follows_a_value) {
    long long v;
#define J(s) s, sizeof(s) - 1
    CHECK(!jr_int(J("{\"a\":12abc}"), "a", &v));
    CHECK(!jr_int(J("{\"a\":12 3}"), "a", &v));
    CHECK(!jr_int(J("{\"b\":truex,\"a\":1}"), "a", &v));
    CHECK(jr_int(J("{\"a\": 12 }"), "a", &v) && v == 12);
    CHECK(jr_int(J("{\"a\":-7,\"b\":1}"), "a", &v) && v == -7);
    // Too long or out of range: rejected, never truncated.
    CHECK(!jr_int(J("{\"a\":0000000000000000000000045045}"), "a", &v));
    CHECK(!jr_int(J("{\"a\":99999999999999999999}"), "a", &v));
    CHECK(jr_int(J("{\"a\":9223372036854775807}"), "a", &v) && v == 9223372036854775807LL);
#undef J
}

int main(void) {
    RUN(writer);
    RUN(writer_int_roundtrip);
    RUN(reader);
    RUN(reader_unicode_escapes);
    RUN(reader_rejects_garbage);
    RUN(reader_checks_what_follows_a_value);
    return TEST_SUMMARY();
}
