#include "json.h"

#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// ---------------------------------------------------------------- writer

static void put(json_writer_t *w, const char *s, size_t n) {
    if (w->len + n + 1 > w->cap) {
        w->overflow = true;
        return;
    }
    memcpy(w->buf + w->len, s, n);
    w->len += n;
    w->buf[w->len] = '\0';
}

static void puts_(json_writer_t *w, const char *s) {
    put(w, s, strlen(s));
}

static void put_string(json_writer_t *w, const char *s) {
    put(w, "\"", 1);
    for (; *s; s++) {
        unsigned char c = (unsigned char)*s;
        if (c == '"' || c == '\\') {
            char e[2] = {'\\', (char)c};
            put(w, e, 2);
        } else if (c < 0x20) {
            char e[8];
            snprintf(e, sizeof(e), "\\u%04x", c);
            puts_(w, e);
        } else {
            put(w, (const char *)&c, 1);
        }
    }
    put(w, "\"", 1);
}

static void key(json_writer_t *w, const char *k) {
    if (!w->first) put(w, ",", 1);
    w->first = false;
    put_string(w, k);
    put(w, ":", 1);
}

void jw_begin(json_writer_t *w, char *buf, size_t cap) {
    w->buf = buf;
    w->cap = cap;
    w->len = 0;
    w->overflow = false;
    w->first = true;
    put(w, "{", 1);
}

void jw_str(json_writer_t *w, const char *k, const char *val) {
    key(w, k);
    put_string(w, val);
}

void jw_int(json_writer_t *w, const char *k, long long val) {
    // Formatted by hand: the printf the Pico SDK links mishandles "%lld"
    // (every value came out shifted left by 32 bits on the RP2350).
    char num[21];  // "-" + 19 digits + NUL
    char *p = num + sizeof(num);
    *--p = '\0';
    unsigned long long mag = val < 0 ? 0ULL - (unsigned long long)val : (unsigned long long)val;
    do {
        *--p = (char)('0' + mag % 10);
        mag /= 10;
    } while (mag);
    if (val < 0) *--p = '-';
    key(w, k);
    puts_(w, p);
}

void jw_bool(json_writer_t *w, const char *k, bool val) {
    key(w, k);
    puts_(w, val ? "true" : "false");
}

size_t jw_end(json_writer_t *w) {
    put(w, "}", 1);
    return w->overflow ? 0 : w->len;
}

// ---------------------------------------------------------------- reader

typedef struct {
    const char *p, *end;
} cur_t;

static void ws(cur_t *c) {
    while (c->p < c->end && (*c->p == ' ' || *c->p == '\t' || *c->p == '\n' || *c->p == '\r')) c->p++;
}

static int hexval(char ch) {
    if (ch >= '0' && ch <= '9') return ch - '0';
    if (ch >= 'a' && ch <= 'f') return ch - 'a' + 10;
    if (ch >= 'A' && ch <= 'F') return ch - 'A' + 10;
    return -1;
}

// Four hex digits of a \u escape at c->p; -1 if they are not there.
static long hex4(cur_t *c) {
    if (c->end - c->p < 4) return -1;
    long v = 0;
    for (int i = 0; i < 4; i++) {
        int h = hexval(c->p[i]);
        if (h < 0) return -1;
        v = (v << 4) | h;
    }
    c->p += 4;
    return v;
}

// Parse a string at c->p (which must point at '"'). Writes the unescaped
// value to out (if not NULL, NUL-terminated, truncation = failure). \u
// escapes become UTF-8 (as a browser sends the same text unescaped); \u0000
// and lone surrogates fail.
static bool parse_string(cur_t *c, char *out, size_t cap) {
    if (c->p >= c->end || *c->p != '"') return false;
    c->p++;
    size_t n = 0;
    while (c->p < c->end && *c->p != '"') {
        char bytes[4];
        size_t len = 1;
        char ch = *c->p++;
        if ((unsigned char)ch < 0x20) return false;
        bytes[0] = ch;
        if (ch == '\\') {
            if (c->p >= c->end) return false;
            char e = *c->p++;
            switch (e) {
            case '"': case '\\': case '/': bytes[0] = e; break;
            case 'b': bytes[0] = '\b'; break;
            case 'f': bytes[0] = '\f'; break;
            case 'n': bytes[0] = '\n'; break;
            case 'r': bytes[0] = '\r'; break;
            case 't': bytes[0] = '\t'; break;
            case 'u': {
                long v = hex4(c);
                if (v <= 0 || (v >= 0xDC00 && v <= 0xDFFF)) return false;
                if (v >= 0xD800 && v <= 0xDBFF) {  // high surrogate: the low one must follow
                    if (c->end - c->p < 2 || c->p[0] != '\\' || c->p[1] != 'u') return false;
                    c->p += 2;
                    long lo = hex4(c);
                    if (lo < 0xDC00 || lo > 0xDFFF) return false;
                    v = 0x10000 + ((v - 0xD800) << 10) + (lo - 0xDC00);
                }
                if (v < 0x80) {
                    bytes[0] = (char)v;
                } else if (v < 0x800) {
                    bytes[0] = (char)(0xC0 | (v >> 6));
                    bytes[1] = (char)(0x80 | (v & 0x3F));
                    len = 2;
                } else if (v < 0x10000) {
                    bytes[0] = (char)(0xE0 | (v >> 12));
                    bytes[1] = (char)(0x80 | ((v >> 6) & 0x3F));
                    bytes[2] = (char)(0x80 | (v & 0x3F));
                    len = 3;
                } else {
                    bytes[0] = (char)(0xF0 | (v >> 18));
                    bytes[1] = (char)(0x80 | ((v >> 12) & 0x3F));
                    bytes[2] = (char)(0x80 | ((v >> 6) & 0x3F));
                    bytes[3] = (char)(0x80 | (v & 0x3F));
                    len = 4;
                }
                break;
            }
            default: return false;
            }
        }
        if (out) {
            if (n + len >= cap) return false;
            memcpy(out + n, bytes, len);
            n += len;
        }
    }
    if (c->p >= c->end) return false;
    c->p++;  // closing quote
    if (out) out[n] = '\0';
    return true;
}

typedef enum { V_STR, V_NUM, V_TRUE, V_FALSE, V_NULL } vtype_t;

static bool skip_value(cur_t *c, vtype_t *type) {
    ws(c);
    if (c->p >= c->end) return false;
    const char *start = c->p;
    if (*c->p == '"') {
        *type = V_STR;
        return parse_string(c, NULL, 0);
    }
    if (*c->p == '-' || (*c->p >= '0' && *c->p <= '9')) {
        *type = V_NUM;
        c->p++;
        while (c->p < c->end && ((*c->p >= '0' && *c->p <= '9') || *c->p == '.' || *c->p == 'e' ||
                                 *c->p == 'E' || *c->p == '+' || *c->p == '-'))
            c->p++;
        return true;
    }
    static const struct { const char *word; vtype_t t; } words[] = {
        {"true", V_TRUE}, {"false", V_FALSE}, {"null", V_NULL}};
    for (size_t i = 0; i < 3; i++) {
        size_t n = strlen(words[i].word);
        if ((size_t)(c->end - start) >= n && memcmp(start, words[i].word, n) == 0) {
            c->p += n;
            *type = words[i].t;
            return true;
        }
    }
    return false;
}

// Locate `key`; on success *val points at the value and *type is set.
static bool find(const char *json, size_t len, const char *key, cur_t *val, vtype_t *type) {
    cur_t c = {json, json + len};
    ws(&c);
    if (c.p >= c.end || *c.p != '{') return false;
    c.p++;
    ws(&c);
    if (c.p < c.end && *c.p == '}') return false;
    for (;;) {
        char k[48];
        ws(&c);
        if (!parse_string(&c, k, sizeof(k))) return false;
        ws(&c);
        if (c.p >= c.end || *c.p != ':') return false;
        c.p++;
        ws(&c);
        cur_t start = c;
        vtype_t t;
        if (!skip_value(&c, &t)) return false;
        ws(&c);
        // A value ends at ',' or '}' ("12abc" is not the number 12).
        if (c.p >= c.end || (*c.p != ',' && *c.p != '}')) return false;
        if (strcmp(k, key) == 0) {
            *val = start;
            *type = t;
            return true;
        }
        if (*c.p == '}') return false;
        c.p++;
    }
}

bool jr_str(const char *json, size_t len, const char *key, char *out, size_t cap) {
    cur_t v;
    vtype_t t;
    if (!find(json, len, key, &v, &t) || t != V_STR) return false;
    return parse_string(&v, out, cap);
}

bool jr_int(const char *json, size_t len, const char *key, long long *out) {
    cur_t v;
    vtype_t t;
    if (!find(json, len, key, &v, &t) || t != V_NUM) return false;
    char num[21];  // "-" and 19 digits
    size_t n = 0;
    while (v.p < v.end && ((*v.p >= '0' && *v.p <= '9') || *v.p == '-')) {
        if (n + 1 >= sizeof(num)) return false;  // too long: never truncate
        num[n++] = *v.p++;
    }
    num[n] = '\0';
    if (n == 0 || (v.p < v.end && (*v.p == '.' || *v.p == 'e' || *v.p == 'E'))) return false;
    char *endp;
    errno = 0;
    *out = strtoll(num, &endp, 10);
    return *endp == '\0' && errno != ERANGE;
}

bool jr_bool(const char *json, size_t len, const char *key, bool *out) {
    cur_t v;
    vtype_t t;
    if (!find(json, len, key, &v, &t) || (t != V_TRUE && t != V_FALSE)) return false;
    *out = t == V_TRUE;
    return true;
}
