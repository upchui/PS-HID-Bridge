// Settings storage: A/B slots, CRC, interrupted writes.
#include <stddef.h>
#include <string.h>

#include "crc32.h"
#include "mini_test.h"
#include "settings.h"

static uint8_t flash[2 * PSHB_SETTINGS_SECTOR];
static int fail_program_after = -1;  // simulate power loss: write only N bytes
static int erase_count;

static void f_read(void *ctx, uint32_t off, void *buf, size_t len) {
    (void)ctx;
    memcpy(buf, flash + off, len);
}

static bool f_erase(void *ctx, uint32_t off) {
    (void)ctx;
    memset(flash + off, 0xFF, PSHB_SETTINGS_SECTOR);
    erase_count++;
    return true;
}

static bool f_program(void *ctx, uint32_t off, const void *data, size_t len) {
    (void)ctx;
    const uint8_t *d = data;
    size_t n = fail_program_after >= 0 && (size_t)fail_program_after < len ? (size_t)fail_program_after : len;
    for (size_t i = 0; i < n; i++) flash[off + i] &= d[i];
    return n == len;
}

static const pshb_flash_t F = {NULL, f_read, f_erase, f_program};

static void blank(void) {
    memset(flash, 0xFF, sizeof(flash));
    fail_program_after = -1;
    erase_count = 0;
}

TEST(empty_flash_gives_defaults) {
    blank();
    pshb_settings_t s;
    CHECK(!settings_load(&F, &s));
    CHECK_EQ_U(s.ip_mode, PSHB_IP_DHCP);
    CHECK_EQ_U(s.udp_port, 45045);
    CHECK_EQ_U(s.pw_set, 0);
    CHECK_EQ_U(s.psk_set, 0);
}

TEST(save_and_load_alternate_slots) {
    blank();
    pshb_settings_t s;
    settings_defaults(&s);
    s.udp_port = 1111;
    CHECK(settings_save(&F, &s));
    CHECK_EQ_U(s.seq, 1);
    s.udp_port = 2222;
    CHECK(settings_save(&F, &s));
    CHECK_EQ_U(s.seq, 2);

    pshb_settings_t a, b;
    memcpy(&a, flash, sizeof(a));
    memcpy(&b, flash + PSHB_SETTINGS_SECTOR, sizeof(b));
    CHECK(settings_valid(&a) && settings_valid(&b));
    CHECK(a.udp_port != b.udp_port);  // different slots used

    pshb_settings_t l;
    CHECK(settings_load(&F, &l));
    CHECK_EQ_U(l.udp_port, 2222);
    CHECK_EQ_U(l.seq, 2);
}

TEST(interrupted_write_keeps_previous) {
    blank();
    pshb_settings_t s;
    settings_defaults(&s);
    s.udp_port = 1000;
    CHECK(settings_save(&F, &s));
    s.udp_port = 2000;
    CHECK(settings_save(&F, &s));
    // Power fails in the middle of the third write.
    s.udp_port = 3000;
    fail_program_after = 100;
    CHECK(!settings_save(&F, &s));
    fail_program_after = -1;
    pshb_settings_t l;
    CHECK(settings_load(&F, &l));
    CHECK_EQ_U(l.udp_port, 2000);
    // The next save overwrites the broken slot, not the good one.
    s.udp_port = 4000;
    CHECK(settings_save(&F, &s));
    CHECK(settings_load(&F, &l));
    CHECK_EQ_U(l.udp_port, 4000);
}

TEST(corrupted_copy_is_ignored) {
    blank();
    pshb_settings_t s;
    settings_defaults(&s);
    s.udp_port = 1234;
    CHECK(settings_save(&F, &s));
    s.udp_port = 5678;
    CHECK(settings_save(&F, &s));
    // Flip a bit in the newer copy (slot 1).
    flash[PSHB_SETTINGS_SECTOR + 40] ^= 0x01;
    pshb_settings_t l;
    CHECK(settings_load(&F, &l));
    CHECK_EQ_U(l.udp_port, 1234);
}

TEST(sequence_wraps) {
    blank();
    pshb_settings_t s;
    settings_defaults(&s);
    s.udp_port = 6;
    CHECK(settings_save(&F, &s));  // slot 0, seq 1
    // Patch slot 0 to the last sequence number (with a valid CRC).
    pshb_settings_t tmp;
    memcpy(&tmp, flash, sizeof(tmp));
    tmp.seq = 0xFFFFFFFFu;
    tmp.crc = crc32(&tmp, offsetof(pshb_settings_t, crc));
    memcpy(flash, &tmp, sizeof(tmp));
    s.udp_port = 7;
    CHECK(settings_save(&F, &s));
    CHECK_EQ_U(s.seq, 0);  // wrapped
    pshb_settings_t l;
    CHECK(settings_load(&F, &l));
    CHECK_EQ_U(l.udp_port, 7);  // 0 counts as newer than 0xFFFFFFFF
}

TEST(erase_resets) {
    blank();
    pshb_settings_t s;
    settings_defaults(&s);
    s.psk_set = 1;
    CHECK(settings_save(&F, &s));
    CHECK(settings_erase(&F));
    pshb_settings_t l;
    CHECK(!settings_load(&F, &l));
    CHECK_EQ_U(l.psk_set, 0);
}

static void filter_of(const char ip[4], uint8_t host_bits, uint32_t *net, uint32_t *mask) {
    pshb_settings_t s;
    settings_defaults(&s);
    memcpy(s.allowed_ip, ip, 4);
    s.allowed_host_bits = host_bits;
    settings_sender_filter(&s, net, mask);
}

TEST(sender_filter_from_settings) {
    uint32_t net, mask;
    // Settings stored before the subnet field existed have 0 there: one address.
    filter_of("\xC0\xA8\x01\x14", 0, &net, &mask);
    CHECK_EQ_U(net, 0xC0A80114u);
    CHECK_EQ_U(mask, 0xFFFFFFFFu);
    // 192.168.1.20 with 8 host bits = 192.168.1.0/24
    filter_of("\xC0\xA8\x01\x14", 8, &net, &mask);
    CHECK_EQ_U(net, 0xC0A80100u);
    CHECK_EQ_U(mask, 0xFFFFFF00u);
    // 0.0.0.0 = any sender, whatever the prefix
    filter_of("\0\0\0\0", 0, &net, &mask);
    CHECK_EQ_U(mask, 0);
    CHECK_EQ_U(net, 0);
    filter_of("\x0A\0\0\0", 32, &net, &mask);
    CHECK_EQ_U(mask, 0);
    CHECK_EQ_U(settings_prefix_mask(0), 0);
    CHECK_EQ_U(settings_prefix_mask(1), 0x80000000u);
    CHECK_EQ_U(settings_prefix_mask(24), 0xFFFFFF00u);
    CHECK_EQ_U(settings_prefix_mask(32), 0xFFFFFFFFu);
}

TEST(subnet_survives_save_and_load) {
    blank();
    pshb_settings_t s;
    settings_defaults(&s);
    memcpy(s.allowed_ip, "\x0A\x01\0\0", 4);
    s.allowed_host_bits = 16;
    CHECK(settings_save(&F, &s));
    pshb_settings_t l;
    CHECK(settings_load(&F, &l));
    CHECK_EQ_U(l.allowed_host_bits, 16);
}

int main(void) {
    RUN(empty_flash_gives_defaults);
    RUN(save_and_load_alternate_slots);
    RUN(interrupted_write_keeps_previous);
    RUN(corrupted_copy_is_ignored);
    RUN(sequence_wraps);
    RUN(erase_resets);
    RUN(sender_filter_from_settings);
    RUN(subnet_survives_save_and_load);
    return TEST_SUMMARY();
}
