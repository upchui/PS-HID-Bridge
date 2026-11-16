// Web interface password: Argon2id (Monocypher), 64 KiB, 3 passes.
// Until a password has been set, the factory default is accepted and the
// web interface insists on changing it.
#ifndef PSHB_PASSWORD_H
#define PSHB_PASSWORD_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "settings.h"

#define PSHB_DEFAULT_PASSWORD "ps-hid-bridge"
// Length in characters (as the browser counts them), UTF-8: up to 4 bytes each.
#define PSHB_PASSWORD_MIN_LEN 8
#define PSHB_PASSWORD_MAX_LEN 64
#define PSHB_PASSWORD_MAX_BYTES (4 * PSHB_PASSWORD_MAX_LEN)

// Characters of a UTF-8 text (bytes that do not continue a character).
size_t password_chars(const char *password);

// Raw Argon2id with the board's parameters (exposed for tests).
void password_hash(const char *password, const uint8_t salt[16], uint8_t out[32]);

bool password_verify(const pshb_settings_t *s, const char *password);

// Hash and store a new password (salt from `random16`). Does not save to flash.
void password_set(pshb_settings_t *s, const char *password, const uint8_t random16[16]);

#endif
