// Board web interface: routes, sessions, settings changes.
// Portable: the platform provides a backend (status, flash, reboot, random)
// and moves bytes between sockets and web_conn_t.
//
// Security:
//  * login with Argon2id password, growing lockout after failed attempts
//  * session cookie (HttpOnly, SameSite=Strict), 30 min idle timeout
//  * every POST needs the header "X-PSHB: 1" (no cross-site requests)
//  * until the factory password is changed, only /api/password works
#ifndef PSHB_WEB_H
#define PSHB_WEB_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "http.h"
#include "monocypher.h"
#include "settings.h"

#define WEB_SESSIONS 2
#define WEB_SESSION_IDLE_US (30ull * 60 * 1000000)

typedef struct {
    uint32_t fw_version;
    uint32_t uptime_ms;
    uint8_t mac[6];
    uint8_t ip[4], mask[4], gw[4], dns[4];
    bool link_up;
    const char *net_state;  // "No link", "DHCP running", "Ready", ...
    uint8_t usb_status;     // PSHB_STATUS_* bits
    bool failsafe;
    uint32_t rx_ok, rx_auth_fail, rx_replay, rx_filtered, rx_malformed;
    const char *partition;  // active firmware slot ("A", "B" or "-")

    // Input path diagnostics measured by core 0 since boot (count 0 = none).
    uint32_t lat_usb_count;                   // reports fetched by the host
    uint32_t lat_usb_p50_us, lat_usb_p95_us;  // packet arrival -> report fetched
    uint32_t lat_usb_max_us;
    uint32_t lat_rx_p50_us, lat_rx_max_us;  // packet noticed -> applied (SPI + decryption)
    uint32_t mouse_inputs;   // packets with mouse input
    uint32_t mouse_waits;    // ... of them arrived while a report was already armed
    uint32_t mouse_misses;   // reports armed too late for their fetch (one frame later)
    uint32_t mouse_lead_us;  // reports are armed this long before the fetch (0 = right away)
} web_status_t;

// One stage of the input path: count, minimum, median, 99th percentile and
// maximum in microseconds (percentiles are bucket upper edges).
typedef struct {
    uint32_t n, min, p50, p99, max;
} web_lat_t;

// Detailed latency diagnostics (GET /api/diag, see docs/latency.md).
typedef struct {
    web_lat_t ready;   // INTn -> input applied ("packet -> report ready")
    web_lat_t ready1;  // the same, first datagram of a receive round only
    web_lat_t detect;  // INTn -> noticed by the main loop
    web_lat_t bus;     // core 0 waits for the SPI bus (per receive round)
    web_lat_t spi;     // reading one datagram over SPI
    web_lat_t handle;  // checks + decryption + apply
    web_lat_t usb;     // INTn -> report fetched by the host
    web_lat_t gap;     // between two mouse reports fetched by the host
    web_lat_t armed;   // mouse report armed -> fetched
    uint32_t rounds, datagrams;  // receive rounds and datagrams read in them
    uint32_t core0_spi;          // SPI transactions by core 0
    uint32_t cr_busy;            // command register still busy at the next command
    uint32_t mouse_inputs, mouse_waits, mouse_fetches, mouse_misses, mouse_lead_us;
    uint32_t mouse_host_skips;   // armed in time, fetched a frame later (the host skipped)
    uint32_t mouse_floor_us;     // lowest lead right now
    uint32_t crypto_min_ns, crypto_avg_ns, crypto_max_ns;  // decrypting one INPUT packet
    bool crypto_fast;            // optimised ChaCha20 for this CPU in use
    uint32_t crypto_helped;      // packets whose second ChaCha20 block the other core computed
    uint32_t crypto_alone;       // ... and the ones computed by one core alone
    uint32_t spi_hz;             // SPI clock to the W5500
} web_diag_t;

// Clock speeds as they are (the settings say what is wanted).
typedef struct {
    uint32_t cpu_hz, spi_hz;
    bool cpu_fell_back;    // 200 MHz wanted, but it did not get through its trial
    bool spi_fast_failed;  // the fast SPI clock failed its self-test
} web_clock_t;

typedef struct {
    void *ctx;
    void (*status)(void *ctx, web_status_t *out);
    // Clock speeds in use (optional).
    void (*clock)(void *ctx, web_clock_t *out);
    // Latency diagnostics, and starting them over (both optional).
    void (*diag)(void *ctx, web_diag_t *out);
    void (*diag_reset)(void *ctx);
    bool (*save)(void *ctx, const pshb_settings_t *s);
    // New key / sender filter for the input path (takes effect immediately).
    void (*apply)(void *ctx, const pshb_settings_t *s);
    void (*reboot)(void *ctx);
    void (*factory_reset)(void *ctx);  // erase settings, then reboot
    void (*random)(void *ctx, uint8_t *buf, size_t n);
    uint64_t (*now_us)(void *ctx);

    // Firmware update: the UF2 file is streamed in; errors as readable text.
    bool (*ota_begin)(void *ctx, uint32_t size, char *err, size_t err_cap);
    bool (*ota_write)(void *ctx, const uint8_t *data, size_t len);
    bool (*ota_finish)(void *ctx, uint32_t crc, char *err, size_t err_cap);
    void (*ota_abort)(void *ctx);
    // Boot the new firmware (called after the response has been sent).
    void (*ota_activate)(void *ctx);
} web_backend_t;

typedef struct {
    const web_backend_t *be;
    pshb_settings_t *settings;
    struct {
        char token[33];
        uint64_t last_us;
        bool used;
    } sessions[WEB_SESSIONS];
    uint32_t failed_logins;
    uint64_t lockout_until_us;
    // The connection streaming a firmware image (one update at a time).
    const void *ota_owner;
    // Firmware upload signed with the key (see protocol/SPEC.md): the last
    // challenge (good for one upload until ota_nonce_until_us, 0 = none),
    // whether the running upload is signed, the body hash its (already
    // checked) signature covers, and the hash of the body so far.
    uint8_t ota_nonce[16];
    uint64_t ota_nonce_until_us;
    bool ota_signed;
    uint8_t ota_signed_hash[32];
    crypto_blake2b_ctx ota_hash;
    // Measurement builds: POST /api/bench/sink takes uploads like a firmware
    // update and throws them away (network load without flash writes).
    bool bench_sink;
} web_app_t;

typedef enum { WEB_AFTER_NONE, WEB_AFTER_REBOOT, WEB_AFTER_FACTORY_RESET, WEB_AFTER_OTA } web_after_t;

typedef struct {
    http_req_t req;
    char head[512];
    size_t head_len;
    const uint8_t *body;
    size_t body_len;
    char dyn[1536];
    size_t sent;
    bool responding;
    web_after_t after;
    uint64_t started_us;
    bool ota;          // this request streams a firmware image
    bool ota_failed;
    bool sink;         // this request streams into the bench sink
    uint32_t sink_bytes;
    bool close_early;  // answered before the body arrived: close after sending
} web_conn_t;

void web_app_init(web_app_t *app, const web_backend_t *be, pshb_settings_t *settings);

void web_conn_reset(web_conn_t *c, uint64_t now_us);

// Feed received bytes; once the request is complete a response is prepared.
void web_conn_input(web_app_t *app, web_conn_t *c, const uint8_t *data, size_t len);

// Next contiguous piece of the response (0 = nothing to send right now).
size_t web_conn_output(const web_conn_t *c, const uint8_t **data);

// `n` bytes were sent. When the response is complete, deferred actions
// (reboot, factory reset) run and web_conn_done() becomes true.
void web_conn_sent(web_app_t *app, web_conn_t *c, size_t n);

bool web_conn_done(const web_conn_t *c);

// The connection is being closed (done, timeout or client gone). Aborts an
// unfinished firmware upload.
void web_conn_close(web_app_t *app, web_conn_t *c);

// Parse "a.b.c.d"; false on anything else.
bool web_parse_ip(const char *s, uint8_t out[4]);

// Parse "a.b.c.d" (prefix 32) or "a.b.c.d/N" with N = 0..32. Host bits are
// cleared, so "192.168.1.20/24" gives 192.168.1.0 and prefix 24.
bool web_parse_cidr(const char *s, uint8_t ip[4], uint8_t *prefix);

// Longest text form of a sender filter: "255.255.255.255/32".
#define WEB_CIDR_STR_LEN 19

#endif
