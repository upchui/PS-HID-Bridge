#include <string.h>

#include "mini_test.h"
#include "web.h"

TEST(ip_addresses) {
    uint8_t ip[4] = {9, 9, 9, 9};
    CHECK(web_parse_ip("192.168.1.20", ip));
    CHECK(ip[0] == 192 && ip[1] == 168 && ip[2] == 1 && ip[3] == 20);
    CHECK(web_parse_ip("0.0.0.0", ip) && ip[3] == 0);
    CHECK(web_parse_ip("255.255.255.255", ip) && ip[0] == 255);
    CHECK(web_parse_ip("010.1.1.1", ip) && ip[0] == 10);  // decimal, not octal

    uint8_t keep[4] = {1, 2, 3, 4};
    const char *bad[] = {"", "1.2.3", "1.2.3.4.5", "256.1.1.1", "1.2.3.4 ", " 1.2.3.4", "+1.2.3.4",
                         "-0.1.2.3", "1..2.3", "1.2.3.4x", "4294967297.1.1.1", "1.2.3.1000", "a.b.c.d"};
    for (size_t i = 0; i < sizeof(bad) / sizeof(bad[0]); i++) {
        memcpy(ip, keep, 4);
        CHECK(!web_parse_ip(bad[i], ip));
        CHECK(memcmp(ip, keep, 4) == 0);  // untouched on failure
    }
}

TEST(subnets) {
    uint8_t ip[4], prefix;
    CHECK(web_parse_cidr("192.168.1.20/24", ip, &prefix));
    CHECK(prefix == 24 && ip[3] == 0);
    CHECK(web_parse_cidr("10.0.0.1", ip, &prefix) && prefix == 32);
    CHECK(!web_parse_cidr("10.0.0.1/33", ip, &prefix));
    CHECK(!web_parse_cidr("10.0.0.1/", ip, &prefix));
    CHECK(!web_parse_cidr(" 10.0.0.1/8", ip, &prefix));
}

TEST(static_addresses) {
    const uint8_t m24[4] = {255, 255, 255, 0}, m31[4] = {255, 255, 255, 254};
    CHECK(web_host_ip_valid((const uint8_t[]){192, 168, 1, 20}, m24));
    CHECK(web_host_ip_valid((const uint8_t[]){10, 14, 0, 21}, m24));
    CHECK(!web_host_ip_valid((const uint8_t[]){192, 168, 1, 0}, m24));    // network
    CHECK(!web_host_ip_valid((const uint8_t[]){192, 168, 1, 255}, m24));  // broadcast
    CHECK(!web_host_ip_valid((const uint8_t[]){127, 0, 0, 1}, m24));
    CHECK(!web_host_ip_valid((const uint8_t[]){0, 1, 2, 3}, m24));
    CHECK(!web_host_ip_valid((const uint8_t[]){224, 0, 0, 1}, m24));
    CHECK(!web_host_ip_valid((const uint8_t[]){255, 255, 255, 255}, m24));
    CHECK(web_host_ip_valid((const uint8_t[]){10, 0, 0, 0}, m31));  // /31: both usable
}

int main(void) {
    RUN(ip_addresses);
    RUN(subnets);
    RUN(static_addresses);
    return TEST_SUMMARY();
}
