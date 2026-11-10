#include "http.h"

#include <stdlib.h>
#include <string.h>
#include <strings.h>

void http_req_init(http_req_t *r) {
    memset(r, 0, sizeof(*r));
    r->state = HTTP_PARSE_HEADERS;
}

static void fail(http_req_t *r, int status) {
    r->state = HTTP_PARSE_ERROR;
    r->error_status = status;
}

void http_req_decide(http_req_t *r, bool stream) {
    r->body_decided = true;
    r->stream = stream;
    if (r->state == HTTP_PARSE_BODY && !stream && r->content_length > HTTP_MAX_BODY) fail(r, 413);
}

static bool header_is(const char *line, const char *name, const char **value) {
    size_t n = strlen(name);
    if (strncasecmp(line, name, n) != 0 || line[n] != ':') return false;
    const char *v = line + n + 1;
    while (*v == ' ' || *v == '\t') v++;
    *value = v;
    return true;
}

static void parse_request_line(http_req_t *r) {
    char *sp1 = strchr(r->line, ' ');
    char *sp2 = sp1 ? strchr(sp1 + 1, ' ') : NULL;
    if (!sp1 || !sp2 || strncmp(sp2 + 1, "HTTP/1.", 7) != 0) {
        fail(r, 400);
        return;
    }
    *sp1 = '\0';
    *sp2 = '\0';
    r->method = strcmp(r->line, "GET") == 0 ? HTTP_GET : strcmp(r->line, "POST") == 0 ? HTTP_POST : HTTP_OTHER;
    char *path = sp1 + 1;
    char *q = strchr(path, '?');
    if (q) *q = '\0';
    if (path[0] != '/') {
        fail(r, 400);
        return;
    }
    if (strlen(path) >= sizeof(r->path)) {
        fail(r, 414);
        return;
    }
    strcpy(r->path, path);
}

static void parse_header(http_req_t *r) {
    const char *v;
    if (header_is(r->line, "Content-Length", &v)) {
        char *end;
        unsigned long n = strtoul(v, &end, 10);
        if (end == v || *end != '\0' || n > 16u * 1024 * 1024) {
            fail(r, 400);
            return;
        }
        r->content_length = (uint32_t)n;
    } else if (header_is(r->line, "X-PSHB", &v)) {
        r->csrf_header = strcmp(v, "1") == 0;
    } else if (header_is(r->line, "X-PSHB-CRC32", &v)) {
        char *end;
        unsigned long n = strtoul(v, &end, 16);
        r->has_crc = end != v && *end == '\0';
        r->crc = (uint32_t)n;
    } else if (header_is(r->line, "Cookie", &v)) {
        const char *p = strstr(v, "pshb_session=");
        while (p && p != v && p[-1] != ' ' && p[-1] != ';') p = strstr(p + 1, "pshb_session=");
        if (p) {
            p += strlen("pshb_session=");
            size_t n = strcspn(p, "; ");
            if (n < sizeof(r->session)) {
                memcpy(r->session, p, n);
                r->session[n] = '\0';
            }
        }
    } else if (header_is(r->line, "Transfer-Encoding", &v)) {
        fail(r, 501);  // chunked uploads are not supported
    }
}

// A complete header line (without CRLF) is in r->line.
static void line_done(http_req_t *r) {
    if (!r->request_line_done) {
        if (r->line_len == 0) return;  // tolerate leading empty lines
        r->request_line_done = true;
        parse_request_line(r);
        return;
    }
    if (r->line_len == 0) {
        r->state = r->content_length ? HTTP_PARSE_BODY : HTTP_PARSE_DONE;
        r->body_decided = r->content_length == 0;
        return;
    }
    parse_header(r);
}

size_t http_req_feed(http_req_t *r, const uint8_t *data, size_t len, http_body_cb on_body, void *ctx) {
    size_t i = 0;
    while (i < len && r->state == HTTP_PARSE_HEADERS) {
        char c = (char)data[i++];
        if (c == '\n') {
            if (r->line_len && r->line[r->line_len - 1] == '\r') r->line_len--;
            r->line[r->line_len] = '\0';
            line_done(r);
            r->line_len = 0;
        } else if (r->line_len + 1 >= sizeof(r->line)) {
            fail(r, 431);
        } else {
            r->line[r->line_len++] = c;
        }
    }

    if (r->state == HTTP_PARSE_BODY) {
        if (!r->body_decided) return i;  // caller decides first (http_req_decide)
        size_t want = r->content_length - r->body_received;
        size_t n = len - i < want ? len - i : want;
        if (r->stream) {
            if (n && on_body) on_body(ctx, data + i, n);
        } else {
            memcpy(r->body + r->body_received, data + i, n);
            r->body[r->body_received + n] = '\0';
        }
        r->body_received += (uint32_t)n;
        i += n;
        if (r->body_received == r->content_length) r->state = HTTP_PARSE_DONE;
    }
    return i;
}
