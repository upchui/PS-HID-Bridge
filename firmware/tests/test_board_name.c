#include "board_name.h"
#include "mini_test.h"
#include "settings.h"

static const char *host(const char *name) {
    static char out[PSHB_NAME_SIZE];
    pshb_hostname_from_name(name, "ps-hid-bridge-96c33d", out, sizeof(out));
    return out;
}

TEST(default_name_from_mac) {
    const uint8_t mac[6] = {0x02, 0x68, 0x9d, 0x96, 0xc3, 0x3d};
    char name[PSHB_NAME_SIZE];
    pshb_board_name(mac, name, sizeof(name));
    CHECK(strcmp(name, "ps-hid-bridge-96c33d") == 0);
}

TEST(valid_names) {
    CHECK(pshb_name_valid(""));
    CHECK(pshb_name_valid("Wohnzimmer PS5"));
    CHECK(pshb_name_valid("Küche \xc3\xa4\xc3\xb6\xc3\xbc"));  // UTF-8 umlauts
    CHECK(pshb_name_valid("\xf0\x9f\x8e\xae Gaming"));         // emoji (4 bytes)
    CHECK(pshb_name_valid("1234567890123456789012345678901"));  // 31 bytes
    CHECK(!pshb_name_valid("12345678901234567890123456789012"));  // 32 bytes
    CHECK(!pshb_name_valid(" leading"));
    CHECK(!pshb_name_valid("trailing "));
    CHECK(!pshb_name_valid("tab\there"));
    CHECK(!pshb_name_valid("del\x7f"));
    CHECK(!pshb_name_valid("\x01"));                 // what a test image left in old settings
    CHECK(!pshb_name_valid("bad \xc3"));             // cut short
    CHECK(!pshb_name_valid("bad \xc0\xaf"));         // overlong
    CHECK(!pshb_name_valid("bad \xed\xa0\x80"));     // surrogate
    CHECK(!pshb_name_valid("bad \xc2\x85"));         // C1 control character
    CHECK(!pshb_name_valid("bad \xff"));
    char unterminated[PSHB_NAME_SIZE];
    memset(unterminated, 'x', sizeof(unterminated));
    CHECK(!pshb_name_valid(unterminated));
}

TEST(network_names) {
    CHECK(strcmp(host("Wohnzimmer PS5"), "wohnzimmer-ps5") == 0);
    CHECK(strcmp(host("K\xc3\xbc" "che \xc3\x84rger Stra\xc3\x9f" "e"), "kueche-aerger-strasse") == 0);
    CHECK(strcmp(host("  --Hello__World!!  "), "hello-world") == 0);
    CHECK(strcmp(host("\xf0\x9f\x8e\xae PS5"), "ps5") == 0);
    CHECK(strcmp(host("caf\xc3\xa9"), "caf") == 0);          // other letters are left out
    CHECK(strcmp(host(""), "ps-hid-bridge-96c33d") == 0);      // no name: the default
    CHECK(strcmp(host("!!! ???"), "ps-hid-bridge-96c33d") == 0);
    CHECK(strcmp(host("\xf0\x9f\x8e\xae"), "ps-hid-bridge-96c33d") == 0);
    // At most 31 characters, never ending in "-" or cutting "ae" in half.
    CHECK(strcmp(host("abcdefghijklmnopqrstuvwxyz abcd efgh"), "abcdefghijklmnopqrstuvwxyz-abcd") == 0);
    CHECK(strcmp(host("abcdefghijklmnopqrstuvwxyzabcd x"), "abcdefghijklmnopqrstuvwxyzabcd") == 0);
    CHECK(strcmp(host("abcdefghijklmnopqrstuvwxyzabcd\xc3\xa4"), "abcdefghijklmnopqrstuvwxyzabcd") == 0);
    char small[8];
    pshb_hostname_from_name("Wohnzimmer", "x", small, sizeof(small));
    CHECK(strcmp(small, "wohnzim") == 0);
}

TEST(stored_name) {
    pshb_settings_t s;
    settings_defaults(&s);
    CHECK(strcmp(settings_board_name(&s), "") == 0);
    // Settings from before the name: zeros, and on one board a 1 in the
    // byte before it.
    s.reserved0 = 1;
    CHECK(strcmp(settings_board_name(&s), "") == 0);
    s.name[0] = 0x01;
    CHECK(strcmp(settings_board_name(&s), "") == 0);
    memset(s.name, 0, sizeof(s.name));
    strcpy(s.name, "Wohnzimmer PS5");
    CHECK(strcmp(settings_board_name(&s), "Wohnzimmer PS5") == 0);
}

int main(void) {
    RUN(default_name_from_mac);
    RUN(valid_names);
    RUN(network_names);
    RUN(stored_name);
    return TEST_SUMMARY();
}
