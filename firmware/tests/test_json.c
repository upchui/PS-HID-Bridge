#include "json.h"
#include "mini_test.h"

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
    CHECK(jr_str(j, len, "pw", s, sizeof(s)) && strcmp(s, "Gr?\"n\\") == 0);
    CHECK(jr_str(j, len, "umlaut", s, sizeof(s)) && strcmp(s, "Grün") == 0);  // raw UTF-8 kept
    CHECK(!jr_str(j, len, "udp_port", s, sizeof(s)));  // wrong type
    CHECK(!jr_int(j, len, "missing", &v));
    CHECK(!jr_str(j, len, "mode", s, 3));               // too small
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

int main(void) {
    RUN(writer);
    RUN(reader);
    RUN(reader_rejects_garbage);
    return TEST_SUMMARY();
}
