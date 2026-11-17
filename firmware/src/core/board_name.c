#include "board_name.h"

#include <stdio.h>
#include <string.h>

void pshb_board_name(const uint8_t mac[6], char *out, size_t cap) {
    snprintf(out, cap, "ps-hid-bridge-%02x%02x%02x", mac[3], mac[4], mac[5]);
}

// Length of the UTF-8 sequence starting at `s` (1..4), 0 if it is not
// well-formed (overlong, surrogate, beyond U+10FFFF, cut short).
static size_t utf8_seq(const unsigned char *s) {
    unsigned char c = s[0];
    if (c < 0x80) return 1;
    size_t n;
    unsigned long cp;
    if ((c & 0xE0) == 0xC0) {
        n = 2;
        cp = c & 0x1F;
    } else if ((c & 0xF0) == 0xE0) {
        n = 3;
        cp = c & 0x0F;
    } else if ((c & 0xF8) == 0xF0) {
        n = 4;
        cp = c & 0x07;
    } else {
        return 0;
    }
    for (size_t i = 1; i < n; i++) {
        if ((s[i] & 0xC0) != 0x80) return 0;  // also catches the NUL at the end
        cp = (cp << 6) | (s[i] & 0x3F);
    }
    static const unsigned long min[] = {0, 0, 0x80, 0x800, 0x10000};
    if (cp < min[n] || cp > 0x10FFFF || (cp >= 0xD800 && cp <= 0xDFFF)) return 0;
    if (cp >= 0x80 && cp < 0xA0) return 0;  // C1 control characters
    return n;
}

bool pshb_name_valid(const char *name) {
    size_t len = 0;
    while (len < PSHB_NAME_SIZE && name[len]) len++;
    if (len >= PSHB_NAME_SIZE) return false;
    if (len == 0) return true;
    if (name[0] == ' ' || name[len - 1] == ' ') return false;
    const unsigned char *s = (const unsigned char *)name;
    for (size_t i = 0; i < len;) {
        if (s[i] < 0x20 || s[i] == 0x7F) return false;
        size_t n = utf8_seq(s + i);
        if (n == 0) return false;
        i += n;
    }
    return true;
}

void pshb_hostname_from_name(const char *name, const char *fallback, char *out, size_t cap) {
    size_t limit = cap < PSHB_NAME_SIZE ? cap : PSHB_NAME_SIZE;
    size_t k = 0;
    bool dash = false;  // a "-" is due before the next letter or digit
    const unsigned char *s = (const unsigned char *)name;
    for (size_t i = 0; s[i] && k + 1 < limit;) {
        const char *put = NULL;
        char one[2] = {0, 0};
        size_t step = 1;
        if ((s[i] >= 'a' && s[i] <= 'z') || (s[i] >= '0' && s[i] <= '9')) {
            one[0] = (char)s[i];
            put = one;
        } else if (s[i] >= 'A' && s[i] <= 'Z') {
            one[0] = (char)(s[i] - 'A' + 'a');
            put = one;
        } else if (s[i] == 0xC3 && s[i + 1]) {
            // ä ö ü Ä Ö Ü ß in UTF-8
            switch (s[i + 1]) {
            case 0xA4: case 0x84: put = "ae"; break;
            case 0xB6: case 0x96: put = "oe"; break;
            case 0xBC: case 0x9C: put = "ue"; break;
            case 0x9F: put = "ss"; break;
            default: break;
            }
            step = 2;
        } else if (s[i] >= 0x80) {
            // Skip the rest of any other character.
            size_t n = utf8_seq(s + i);
            step = n ? n : 1;
        }
        i += step;
        if (!put) {
            dash = k > 0;
            continue;
        }
        size_t need = strlen(put) + (dash ? 1 : 0);
        if (k + need + 1 > limit) break;
        if (dash) out[k++] = '-';
        dash = false;
        for (const char *p = put; *p; p++) out[k++] = *p;
    }
    out[k] = '\0';
    if (k == 0) snprintf(out, cap, "%s", fallback);
}
