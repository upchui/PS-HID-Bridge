#include "password.h"

#include <string.h>

#include "monocypher.h"

#define ARGON2_BLOCKS 64  // 1 KiB each

static uint64_t s_work_area[ARGON2_BLOCKS * 1024 / sizeof(uint64_t)];

void password_hash(const char *password, const uint8_t salt[16], uint8_t out[32]) {
    crypto_argon2_config cfg = {
        .algorithm = CRYPTO_ARGON2_ID,
        .nb_blocks = ARGON2_BLOCKS,
        .nb_passes = 3,
        .nb_lanes = 1,
    };
    crypto_argon2_inputs in = {
        .pass = (const uint8_t *)password,
        .pass_size = (uint32_t)strlen(password),
        .salt = salt,
        .salt_size = 16,
    };
    crypto_argon2(out, 32, s_work_area, cfg, in, crypto_argon2_no_extras);
    crypto_wipe(s_work_area, sizeof(s_work_area));
}

bool password_verify(const pshb_settings_t *s, const char *password) {
    if (strlen(password) > PSHB_PASSWORD_MAX_LEN) return false;
    if (!s->pw_set) {
        // Factory default; compare in constant time over the padded buffer.
        uint8_t a[PSHB_PASSWORD_MAX_LEN + 1] = {0}, b[PSHB_PASSWORD_MAX_LEN + 1] = {0};
        memcpy(a, password, strlen(password));
        memcpy(b, PSHB_DEFAULT_PASSWORD, sizeof(PSHB_DEFAULT_PASSWORD) - 1);
        uint8_t diff = 0;
        for (size_t i = 0; i < sizeof(a); i++) diff |= (uint8_t)(a[i] ^ b[i]);
        return diff == 0;
    }
    uint8_t h[32];
    password_hash(password, s->pw_salt, h);
    bool ok = crypto_verify32(h, s->pw_hash) == 0;
    crypto_wipe(h, sizeof(h));
    return ok;
}

void password_set(pshb_settings_t *s, const char *password, const uint8_t random16[16]) {
    memcpy(s->pw_salt, random16, 16);
    password_hash(password, s->pw_salt, s->pw_hash);
    s->pw_set = 1;
}
