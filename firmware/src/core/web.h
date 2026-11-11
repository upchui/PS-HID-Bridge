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
    uint32_t lat_poll_offset_us;  // median: start of the USB frame -> report fetched
    uint32_t lat_rx_p50_us, lat_rx_max_us;  // packet noticed -> applied (SPI + decryption)
    uint32_t mouse_waits;  // motion arrived while the previous report was not fetched yet
} web_status_t;

typedef struct {
    void *ctx;
    void (*status)(void *ctx, web_status_t *out);
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
