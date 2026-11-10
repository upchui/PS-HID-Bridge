#include "mini_test.h"
#include "monocypher.h"
#include "password.h"

TEST(argon2id_rfc9106) {
    // RFC 9106, section 5.3 (Argon2id): 32 KiB, 3 passes, 4 lanes, secret and ad.
    uint8_t pass[32], salt[16], secret[8], ad[12];
    memset(pass, 1, sizeof(pass));
    memset(salt, 2, sizeof(salt));
    memset(secret, 3, sizeof(secret));
    memset(ad, 4, sizeof(ad));
    static uint64_t work[32 * 1024 / 8];
    crypto_argon2_config cfg = {CRYPTO_ARGON2_ID, 32, 3, 4};
    crypto_argon2_inputs in = {pass, salt, sizeof(pass), sizeof(salt)};
    crypto_argon2_extras ex = {secret, ad, sizeof(secret), sizeof(ad)};
    uint8_t tag[32];
    crypto_argon2(tag, sizeof(tag), work, cfg, in, ex);
    const uint8_t want[32] = {0x0d, 0x64, 0x0d, 0xf5, 0x8d, 0x78, 0x76, 0x6c, 0x08, 0xc0, 0x37,
                              0xa3, 0x4a, 0x8b, 0x53, 0xc9, 0xd0, 0x1e, 0xf0, 0x45, 0x2d, 0x75,
                              0xb6, 0x5e, 0xb5, 0x25, 0x20, 0xe9, 0x6b, 0x01, 0xe6, 0x59};
    CHECK_MEM(tag, want, 32);
}

TEST(default_then_own_password) {
    pshb_settings_t s;
    settings_defaults(&s);
    CHECK(password_verify(&s, PSHB_DEFAULT_PASSWORD));
    CHECK(!password_verify(&s, "ps5-hid-bridgeX"));
    CHECK(!password_verify(&s, ""));

    const uint8_t salt[16] = {9, 8, 7};
    password_set(&s, "Geheimes Passwort", salt);
    CHECK(s.pw_set);
    CHECK(password_verify(&s, "Geheimes Passwort"));
    CHECK(!password_verify(&s, PSHB_DEFAULT_PASSWORD));
    CHECK(!password_verify(&s, "geheimes Passwort"));
}

TEST(salt_matters) {
    uint8_t a[32], b[32];
    const uint8_t s1[16] = {1}, s2[16] = {2};
    password_hash("same", s1, a);
    password_hash("same", s2, b);
    CHECK(memcmp(a, b, 32) != 0);
}

int main(void) {
    RUN(argon2id_rfc9106);
    RUN(default_then_own_password);
    RUN(salt_matters);
    return TEST_SUMMARY();
}
