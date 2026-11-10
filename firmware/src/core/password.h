// Web interface password: Argon2id (Monocypher), 64 KiB, 3 passes.
// Until a password has been set, the factory default is accepted and the
// web interface insists on changing it.
#ifndef PSHB_PASSWORD_H
#define PSHB_PASSWORD_H

#include <stdbool.h>
#include <stdint.h>

#include "settings.h"

#define PSHB_DEFAULT_PASSWORD "ps5-hid-bridge"
#define PSHB_PASSWORD_MIN_LEN 8
#define PSHB_PASSWORD_MAX_LEN 64

// Raw Argon2id with the board's parameters (exposed for tests).
void password_hash(const char *password, const uint8_t salt[16], uint8_t out[32]);

bool password_verify(const pshb_settings_t *s, const char *password);

// Hash and store a new password (salt from `random16`). Does not save to flash.
void password_set(pshb_settings_t *s, const char *password, const uint8_t random16[16]);

#endif
