// Incremental HTTP/1.1 request parser for the board's web interface.
// Bytes may arrive in arbitrary pieces. Only what the web API needs is
// extracted; everything else is skipped.
#ifndef PSHB_HTTP_H
#define PSHB_HTTP_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define HTTP_MAX_LINE 512
#define HTTP_MAX_PATH 64
#define HTTP_MAX_BODY 1024

typedef enum { HTTP_GET, HTTP_POST, HTTP_OTHER } http_method_t;

typedef enum {
    HTTP_PARSE_HEADERS,
    HTTP_PARSE_BODY,   // headers complete; body follows (see body_decided)
    HTTP_PARSE_DONE,
    HTTP_PARSE_ERROR,  // see error_status
} http_parse_state_t;

typedef void (*http_body_cb)(void *ctx, const uint8_t *data, size_t len);

typedef struct {
    http_method_t method;
    char path[HTTP_MAX_PATH];
    uint32_t content_length;
    bool csrf_header;         // "X-PSHB: 1" present
    char session[40];         // value of the pshb_session cookie
    bool has_crc;
    uint32_t crc;             // X-PSHB-CRC32 (hex)
    bool has_ota_auth;
    uint8_t ota_auth[32];     // X-PSHB-OTA-Auth (64 hex): MAC of a signed firmware upload

    // Body: buffered (<= HTTP_MAX_BODY) or streamed through a callback.
    // After the headers the parser pauses until the caller sets
    // body_decided (and optionally stream).
    bool body_decided;
    bool stream;
    char body[HTTP_MAX_BODY + 1];
    uint32_t body_received;

    http_parse_state_t state;
    int error_status;         // HTTP status to answer with on error
    char line[HTTP_MAX_LINE];
    size_t line_len;
    bool request_line_done;
} http_req_t;

void http_req_init(http_req_t *r);

// After the headers: buffer the body (stream = false, at most HTTP_MAX_BODY
// bytes, otherwise the request fails with 413) or hand it to the callback.
void http_req_decide(http_req_t *r, bool stream);

// Consume bytes; returns how many were used. Stops early when the headers
// are complete and the body handling is not decided yet, and after the
// request is complete (extra bytes are not consumed).
size_t http_req_feed(http_req_t *r, const uint8_t *data, size_t len, http_body_cb on_body, void *ctx);

#endif
