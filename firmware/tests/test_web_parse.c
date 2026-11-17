#include <stdio.h>
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

// ---------------------------------------------------------------- Host header

static uint64_t fake_now(void *ctx) {
    (void)ctx;
    return 1000000;
}

static void fake_random(void *ctx, uint8_t *buf, size_t n) {
    (void)ctx;
    memset(buf, 0x5A, n);
}

static const web_backend_t k_backend = {.now_us = fake_now, .random = fake_random};

// The status code of "GET /api/clock" (no login needed) with this Host
// header (NULL: none at all).
static int status_for_host(web_app_t *app, const char *host) {
    char req[256];
    if (host) {
        snprintf(req, sizeof(req), "GET /api/clock HTTP/1.1\r\nHost: %s\r\n\r\n", host);
    } else {
        snprintf(req, sizeof(req), "GET /api/clock HTTP/1.1\r\n\r\n");
    }
    static web_conn_t c;
    web_conn_reset(&c, 0);
    web_conn_input(app, &c, (const uint8_t *)req, strlen(req));
    const uint8_t *out;
    size_t n = web_conn_output(&c, &out);
    int status = 0;
    char line[32];
    if (n >= 12) {
        memcpy(line, out, 12);
        line[12] = '\0';
        sscanf(line, "HTTP/1.1 %d", &status);
    }
    web_conn_close(app, &c);
    return status;
}

TEST(host_header) {
    static pshb_settings_t s;
    settings_defaults(&s);
    static web_app_t app;
    web_app_init(&app, &k_backend, &s);
    web_app_set_hostname(&app, "ps-hid-bridge-514d00");

    // Without a name of its own: addresses and the default name, also
    // under .local (mDNS) or the router's domain, any case, with a port.
    const char *ok[] = {
        NULL, "192.168.1.50", "192.168.1.50:80", "[fe80::1]:80", "localhost",
        "ps-hid-bridge-514d00", "ps-hid-bridge-514d00.local", "ps-hid-bridge-514d00.local:80",
        "PS-HID-Bridge-514D00.LOCAL", "ps-hid-bridge-514d00.local.", "ps-hid-bridge-514d00.local:8080",
        "ps-hid-bridge-514d00.fritz.box",
    };
    for (size_t i = 0; i < sizeof(ok) / sizeof(ok[0]); i++) {
        int st = status_for_host(&app, ok[i]);
        if (st != 200) fprintf(stderr, "  Host %s: %d\n", ok[i] ? ok[i] : "(none)", st);
        CHECK_EQ_I(st, 200);
    }
    const char *bad[] = {
        "evil.example", "evil.local", "local", ".local", "ps-hid-bridge-514d00x.local",
        "x.ps-hid-bridge-514d00.local", "ps-hid-bridge-514d0.local", "wohnzimmer-ps5.local",
        "192.168.1.50.evil.example",
    };
    for (size_t i = 0; i < sizeof(bad) / sizeof(bad[0]); i++) {
        int st = status_for_host(&app, bad[i]);
        if (st != 421) fprintf(stderr, "  Host %s: %d\n", bad[i], st);
        CHECK_EQ_I(st, 421);
    }

    // Named "Wohnzimmer PS5": its network name as well, the default name still.
    snprintf(s.name, sizeof(s.name), "Wohnzimmer PS5");
    const char *named[] = {
        "wohnzimmer-ps5.local", "Wohnzimmer-PS5.Local:80", "wohnzimmer-ps5", "WOHNZIMMER-PS5.LOCAL:8080",
        "ps-hid-bridge-514d00.local",
    };
    for (size_t i = 0; i < sizeof(named) / sizeof(named[0]); i++) {
        int st = status_for_host(&app, named[i]);
        if (st != 200) fprintf(stderr, "  Host %s: %d\n", named[i], st);
        CHECK_EQ_I(st, 200);
    }
    const char *not_named[] = {"wohnzimmer-ps5x.local", "wohnzimmer.local", "wohnzimmer ps5.local", "kueche.local"};
    for (size_t i = 0; i < sizeof(not_named) / sizeof(not_named[0]); i++) {
        CHECK_EQ_I(status_for_host(&app, not_named[i]), 421);
    }
}

int main(void) {
    RUN(ip_addresses);
    RUN(subnets);
    RUN(static_addresses);
    RUN(host_header);
    return TEST_SUMMARY();
}
