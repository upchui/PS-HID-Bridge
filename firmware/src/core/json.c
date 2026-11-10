#include "json.h"

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
    char num[24];
    snprintf(num, sizeof(num), "%lld", val);
    key(w, k);
    puts_(w, num);
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

// Parse a string at c->p (which must point at '"'). Writes the unescaped
// value to out (if not NULL, NUL-terminated, truncation = failure).
static bool parse_string(cur_t *c, char *out, size_t cap) {
    if (c->p >= c->end || *c->p != '"') return false;
    c->p++;
    size_t n = 0;
    while (c->p < c->end && *c->p != '"') {
        char ch = *c->p++;
        if ((unsigned char)ch < 0x20) return false;
        if (ch == '\\') {
            if (c->p >= c->end) return false;
            char e = *c->p++;
            switch (e) {
            case '"': case '\\': case '/': ch = e; break;
            case 'b': ch = '\b'; break;
            case 'f': ch = '\f'; break;
            case 'n': ch = '\n'; break;
            case 'r': ch = '\r'; break;
            case 't': ch = '\t'; break;
            case 'u': {
                if (c->end - c->p < 4) return false;
                int v = 0;
                for (int i = 0; i < 4; i++) {
                    int h = hexval(c->p[i]);
                    if (h < 0) return false;
                    v = (v << 4) | h;
                }
                c->p += 4;
                ch = v < 0x80 ? (char)v : '?';  // only ASCII escapes are expected
                break;
            }
            default: return false;
            }
        }
        if (out) {
            if (n + 1 >= cap) return false;
            out[n++] = ch;
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
        if (strcmp(k, key) == 0) {
            *val = start;
            *type = t;
            return true;
        }
        ws(&c);
        if (c.p >= c.end) return false;
        if (*c.p == '}') return false;
        if (*c.p != ',') return false;
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
    char num[24];
    size_t n = 0;
    while (v.p < v.end && n + 1 < sizeof(num) && ((*v.p >= '0' && *v.p <= '9') || *v.p == '-')) {
        num[n++] = *v.p++;
    }
    num[n] = '\0';
    if (n == 0 || (v.p < v.end && (*v.p == '.' || *v.p == 'e' || *v.p == 'E'))) return false;
    char *endp;
    *out = strtoll(num, &endp, 10);
    return *endp == '\0';
}

bool jr_bool(const char *json, size_t len, const char *key, bool *out) {
    cur_t v;
    vtype_t t;
    if (!find(json, len, key, &v, &t) || (t != V_TRUE && t != V_FALSE)) return false;
    *out = t == V_TRUE;
    return true;
}
