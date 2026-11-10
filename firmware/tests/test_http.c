// HTTP request parser: arbitrary split points, limits and header handling.
#include <stdlib.h>

#include "http.h"
#include "mini_test.h"

static const char REQ[] =
    "POST /api/login?x=1 HTTP/1.1\r\n"
    "Host: 192.168.1.50\r\n"
    "Content-Type: application/json\r\n"
    "x-pshb: 1\r\n"
    "Cookie: theme=dark; pshb_session=0123456789abcdef0123456789abcdef\r\n"
    "Content-Length: 21\r\n"
    "\r\n"
    "{\"password\":\"geheim\"}";

static void feed_all(http_req_t *r, const char *data, size_t len, size_t chunk) {
    size_t off = 0;
    while (off < len && r->state != HTTP_PARSE_DONE && r->state != HTTP_PARSE_ERROR) {
        size_t n = len - off < chunk ? len - off : chunk;
        size_t used = 0;
        while (used < n) {
            size_t k = http_req_feed(r, (const uint8_t *)data + off + used, n - used, NULL, NULL);
            used += k;
            if (r->state == HTTP_PARSE_BODY && !r->body_decided) {
                http_req_decide(r, false);
                continue;
            }
            if (k == 0) break;
        }
        off += n;
    }
}

static void check_parsed(const http_req_t *r) {
    CHECK_EQ_I(r->state, HTTP_PARSE_DONE);
    CHECK_EQ_I(r->method, HTTP_POST);
    CHECK(strcmp(r->path, "/api/login") == 0);
    CHECK(r->csrf_header);
    CHECK(strcmp(r->session, "0123456789abcdef0123456789abcdef") == 0);
    CHECK_EQ_U(r->content_length, 21);
    CHECK(strcmp(r->body, "{\"password\":\"geheim\"}") == 0);
}

TEST(whole_request) {
    http_req_t r;
    http_req_init(&r);
    feed_all(&r, REQ, sizeof(REQ) - 1, sizeof(REQ));
    check_parsed(&r);
}

TEST(every_chunk_size) {
    for (size_t chunk = 1; chunk < sizeof(REQ); chunk++) {
        http_req_t r;
        http_req_init(&r);
        feed_all(&r, REQ, sizeof(REQ) - 1, chunk);
        check_parsed(&r);
    }
}

TEST(random_splits) {
    srand(42);
    for (int round = 0; round < 500; round++) {
        http_req_t r;
        http_req_init(&r);
        size_t off = 0, len = sizeof(REQ) - 1;
        while (off < len) {
            size_t n = 1 + (size_t)rand() % 40;
            if (n > len - off) n = len - off;
            feed_all(&r, REQ + off, n, n);
            off += n;
        }
        check_parsed(&r);
    }
}

TEST(get_without_body) {
    http_req_t r;
    http_req_init(&r);
    const char req[] = "GET / HTTP/1.1\r\nHost: x\r\n\r\n";
    feed_all(&r, req, sizeof(req) - 1, 5);
    CHECK_EQ_I(r.state, HTTP_PARSE_DONE);
    CHECK_EQ_I(r.method, HTTP_GET);
    CHECK(strcmp(r.path, "/") == 0);
    CHECK(!r.csrf_header);
    CHECK_EQ_U(r.session[0], 0);
}

TEST(errors) {
    struct {
        const char *req;
        int status;
    } cases[] = {
        {"GARBAGE\r\n\r\n", 400},
        {"GET noslash HTTP/1.1\r\n\r\n", 400},
        {"POST /api/x HTTP/1.1\r\nContent-Length: 5000\r\n\r\n", 413},
        {"POST /api/x HTTP/1.1\r\nContent-Length: abc\r\n\r\n", 400},
        {"POST /api/x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n", 501},
    };
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        http_req_t r;
        http_req_init(&r);
        feed_all(&r, cases[i].req, strlen(cases[i].req), 7);
        CHECK_EQ_I(r.state, HTTP_PARSE_ERROR);
        CHECK_EQ_I(r.error_status, cases[i].status);
    }
    // Over-long path and header line.
    char big[700];
    http_req_t r;
    http_req_init(&r);
    snprintf(big, sizeof(big), "GET /%0100d HTTP/1.1\r\n\r\n", 0);
    feed_all(&r, big, strlen(big), 64);
    CHECK_EQ_I(r.error_status, 414);
    http_req_init(&r);
    memset(big, 'a', sizeof(big));
    memcpy(big, "GET / HTTP/1.1\r\nX: ", 19);
    feed_all(&r, big, sizeof(big), 64);
    CHECK_EQ_I(r.error_status, 431);
}

static size_t s_streamed;
static uint32_t s_stream_sum;

static void on_body(void *ctx, const uint8_t *data, size_t len) {
    (void)ctx;
    for (size_t i = 0; i < len; i++) s_stream_sum += data[i];
    s_streamed += len;
}

TEST(streamed_body) {
    // Large body handed to a callback (firmware upload).
    static char req[100 + 5000];
    int head = snprintf(req, sizeof(req), "POST /api/ota HTTP/1.1\r\nContent-Length: 5000\r\nX-PSHB-CRC32: deadBEEF\r\n\r\n");
    uint32_t want = 0;
    for (int i = 0; i < 5000; i++) {
        req[head + i] = (char)(i * 7);
        want += (uint8_t)(i * 7);
    }
    http_req_t r;
    http_req_init(&r);
    s_streamed = 0;
    s_stream_sum = 0;
    size_t off = 0, total = (size_t)head + 5000;
    while (off < total && r.state != HTTP_PARSE_DONE) {
        size_t n = total - off < 333 ? total - off : 333;
        size_t used = http_req_feed(&r, (const uint8_t *)req + off, n, on_body, NULL);
        if (r.state == HTTP_PARSE_BODY && !r.body_decided) {
            CHECK(r.has_crc);
            CHECK_EQ_U(r.crc, 0xDEADBEEF);
            http_req_decide(&r, true);
        }
        off += used;
    }
    CHECK_EQ_I(r.state, HTTP_PARSE_DONE);
    CHECK_EQ_U(s_streamed, 5000);
    CHECK_EQ_U(s_stream_sum, want);
}

int main(void) {
    RUN(whole_request);
    RUN(every_chunk_size);
    RUN(random_splits);
    RUN(get_without_body);
    RUN(errors);
    RUN(streamed_body);
    return TEST_SUMMARY();
}
