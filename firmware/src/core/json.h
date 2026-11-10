// Minimal JSON for the web API: a writer into a fixed buffer and a reader
// for flat objects ({"key": "str" | number | true | false | null, ...}).
#ifndef PSHB_JSON_H
#define PSHB_JSON_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

typedef struct {
    char *buf;
    size_t cap;
    size_t len;
    bool overflow;
    bool first;
} json_writer_t;

void jw_begin(json_writer_t *w, char *buf, size_t cap);
void jw_str(json_writer_t *w, const char *key, const char *val);
void jw_int(json_writer_t *w, const char *key, long long val);
void jw_bool(json_writer_t *w, const char *key, bool val);
// Finish the object; returns its length or 0 if the buffer was too small.
size_t jw_end(json_writer_t *w);

// Reader. All return false if the key is missing, the type differs or the
// document is not a flat object.
bool jr_str(const char *json, size_t len, const char *key, char *out, size_t cap);
bool jr_int(const char *json, size_t len, const char *key, long long *out);
bool jr_bool(const char *json, size_t len, const char *key, bool *out);

#endif
