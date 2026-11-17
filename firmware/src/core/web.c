#include "web.h"

#include <stdio.h>
#include <string.h>
#include <strings.h>

#include "board_name.h"
#include "hexutil.h"
#include "json.h"
#include "monocypher.h"
#include "password.h"
#include "protocol.h"
#include "web_assets.h"

#define COOKIE_NAME "pshb_session"

// Signed firmware updates (protocol/SPEC.md, "Firmware update with the key").
#define OTA_KEY_LABEL "pshb-ota-key-v1"
#define OTA_MAC_LABEL "pshb-ota-v2"
#define REQ_MAC_LABEL "pshb-req-v1"

static void ota_key(const uint8_t psk[32], uint8_t out[32]);
#define OTA_CHALLENGE_US 60000000ull

// ---------------------------------------------------------------- helpers

bool web_parse_ip(const char *s, uint8_t out[4]) {
    // Exactly a.b.c.d: digits only (no spaces or signs), 1-3 digits, <= 255.
    uint8_t v[4];
    for (int i = 0; i < 4; i++) {
        unsigned n = 0;
        int digits = 0;
        while (*s >= '0' && *s <= '9') {
            n = n * 10 + (unsigned)(*s++ - '0');
            if (++digits > 3 || n > 255) return false;
        }
        if (digits == 0) return false;
        v[i] = (uint8_t)n;
        if (i < 3 && *s++ != '.') return false;
    }
    if (*s != '\0') return false;
    memcpy(out, v, 4);
    return true;
}

bool web_parse_cidr(const char *s, uint8_t ip[4], uint8_t *prefix) {
    char addr[16];
    const char *slash = strchr(s, '/');
    size_t n = slash ? (size_t)(slash - s) : strlen(s);
    if (n >= sizeof(addr)) return false;
    memcpy(addr, s, n);
    addr[n] = '\0';
    if (!web_parse_ip(addr, ip)) return false;

    unsigned p = 32;
    if (slash) {
        const char *d = slash + 1;
        size_t len = strlen(d);
        if (len < 1 || len > 2) return false;
        p = 0;
        for (size_t i = 0; i < len; i++) {
            if (d[i] < '0' || d[i] > '9') return false;
            p = p * 10 + (unsigned)(d[i] - '0');
        }
        if (p > 32) return false;
    }
    uint32_t mask = settings_prefix_mask(p);
    for (int i = 0; i < 4; i++) ip[i] &= (uint8_t)(mask >> (24 - 8 * i));
    *prefix = (uint8_t)p;
    return true;
}

static void ip_str(const uint8_t ip[4], char out[16]) {
    snprintf(out, 16, "%u.%u.%u.%u", ip[0], ip[1], ip[2], ip[3]);
}

static bool ip_zero(const uint8_t ip[4]) {
    return !(ip[0] | ip[1] | ip[2] | ip[3]);
}

// "" (any sender), "a.b.c.d" (one address) or "a.b.c.d/N" (subnet).
static void sender_filter_str(const pshb_settings_t *s, char out[WEB_CIDR_STR_LEN + 1]) {
    uint32_t net, mask;
    settings_sender_filter(s, &net, &mask);
    if (mask == 0) {
        out[0] = '\0';
        return;
    }
    int len = snprintf(out, WEB_CIDR_STR_LEN + 1, "%u.%u.%u.%u", (unsigned)(net >> 24), (unsigned)(net >> 16) & 0xFF,
                       (unsigned)(net >> 8) & 0xFF, (unsigned)net & 0xFF);
    if (mask != 0xFFFFFFFFu) {
        snprintf(out + len, (size_t)(WEB_CIDR_STR_LEN + 1 - len), "/%u", 32u - s->allowed_host_bits);
    }
}

static uint32_t ip_u32(const uint8_t a[4]) {
    return ((uint32_t)a[0] << 24) | ((uint32_t)a[1] << 16) | ((uint32_t)a[2] << 8) | a[3];
}

bool web_host_ip_valid(const uint8_t ip[4], const uint8_t mask[4]) {
    if (ip[0] == 0 || ip[0] == 127 || ip[0] >= 224) return false;
    uint32_t hostmask = ~ip_u32(mask);
    uint32_t host = ip_u32(ip) & hostmask;
    // /31 and /32 have no network and broadcast address.
    return hostmask <= 1 || (host != 0 && host != hostmask);
}

static bool mask_valid(const uint8_t m[4]) {
    uint32_t v = ((uint32_t)m[0] << 24) | ((uint32_t)m[1] << 16) | ((uint32_t)m[2] << 8) | m[3];
    uint32_t inv = ~v;
    return v != 0 && (inv & (inv + 1)) == 0;
}

static const char *reason(int status) {
    switch (status) {
    case 200: return "OK";
    case 204: return "No Content";
    case 400: return "Bad Request";
    case 401: return "Unauthorized";
    case 403: return "Forbidden";
    case 404: return "Not Found";
    case 405: return "Method Not Allowed";
    case 409: return "Conflict";
    case 413: return "Payload Too Large";
    case 414: return "URI Too Long";
    case 421: return "Misdirected Request";
    case 429: return "Too Many Requests";
    case 431: return "Request Header Fields Too Large";
    case 500: return "Internal Server Error";
    case 501: return "Not Implemented";
    default: return "Error";
    }
}

static void respond(web_conn_t *c, int status, const char *type, const uint8_t *body, size_t len,
                    bool gzip, const char *extra) {
    c->head_len = (size_t)snprintf(
        c->head, sizeof(c->head),
        "HTTP/1.1 %d %s\r\n"
        "Content-Type: %s\r\n"
        "Content-Length: %u\r\n"
        "%s"
        "Cache-Control: no-store\r\n"
        "X-Content-Type-Options: nosniff\r\n"
        "X-Frame-Options: DENY\r\n"
        "Referrer-Policy: no-referrer\r\n"
        // connect-src: the System page asks GitHub for the newest release.
        "Content-Security-Policy: default-src 'self'; connect-src 'self' https://api.github.com; "
        "frame-ancestors 'none'\r\n"
        "Connection: close\r\n"
        "%s"
        "\r\n",
        status, reason(status), type, (unsigned)len, gzip ? "Content-Encoding: gzip\r\n" : "",
        extra ? extra : "");
    if (c->head_len >= sizeof(c->head)) c->head_len = sizeof(c->head) - 1;
    c->body = body;
    c->body_len = len;
    c->sent = 0;
    c->responding = true;
}

static void respond_json(web_conn_t *c, int status, json_writer_t *w, const char *extra) {
    size_t n = jw_end(w);
    if (n == 0) {
        static const char err[] = "{\"error\":\"Response too large\"}";
        respond(c, 500, "application/json", (const uint8_t *)err, sizeof(err) - 1, false, NULL);
        return;
    }
    respond(c, status, "application/json; charset=utf-8", (const uint8_t *)c->dyn, n, false, extra);
}

static void respond_error(web_conn_t *c, int status, const char *msg) {
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_str(&w, "error", msg);
    respond_json(c, status, &w, NULL);
}

static void respond_ok(web_conn_t *c) {
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_bool(&w, "ok", true);
    respond_json(c, 200, &w, NULL);
}

// ---------------------------------------------------------------- sessions

static uint64_t now(web_app_t *app) {
    return app->be->now_us(app->be->ctx);
}

static int session_find(web_app_t *app, const char *token) {
    if (strlen(token) != 32) return -1;
    uint64_t t = now(app);
    for (int i = 0; i < WEB_SESSIONS; i++) {
        if (!app->sessions[i].used) continue;
        if (t - app->sessions[i].last_us > WEB_SESSION_IDLE_US) {
            app->sessions[i].used = false;
            continue;
        }
        // Constant-time comparison of the 32 hex characters.
        uint8_t diff = 0;
        for (int k = 0; k < 32; k++) diff |= (uint8_t)(app->sessions[i].token[k] ^ token[k]);
        if (diff == 0) {
            app->sessions[i].last_us = t;
            return i;
        }
    }
    return -1;
}

static void session_create(web_app_t *app, char out_token[33]) {
    int slot = 0;
    for (int i = 0; i < WEB_SESSIONS; i++) {
        if (!app->sessions[i].used) {
            slot = i;
            break;
        }
        if (app->sessions[i].last_us < app->sessions[slot].last_us) slot = i;
    }
    uint8_t raw[16];
    app->be->random(app->be->ctx, raw, sizeof(raw));
    hex_encode(raw, sizeof(raw), app->sessions[slot].token);
    app->sessions[slot].used = true;
    app->sessions[slot].last_us = now(app);
    memcpy(out_token, app->sessions[slot].token, 33);
}

static void sessions_clear(web_app_t *app) {
    memset(app->sessions, 0, sizeof(app->sessions));
}

// ---------------------------------------------------------------- handlers

// Store `next` and only then make it the current settings: after a failed
// save, what runs and what is in flash still agree.
// The board's name as shown: the one the user gave it, else the default.
static const char *shown_name(const web_app_t *app) {
    const char *n = settings_board_name(app->settings);
    return n[0] ? n : app->hostname;
}

// The board's network name (DHCP, Host): derived from its name, else the
// default.
static void network_name(const web_app_t *app, char out[PSHB_NAME_SIZE]) {
    pshb_hostname_from_name(settings_board_name(app->settings), app->hostname, out, PSHB_NAME_SIZE);
}

static bool save_apply(web_app_t *app, web_conn_t *c, const pshb_settings_t *next) {
    if (!app->be->save(app->be->ctx, next)) {
        respond_error(c, 500, "Saving to flash failed");
        return false;
    }
    *app->settings = *next;
    app->be->apply(app->be->ctx, app->settings);
    return true;
}

static void h_state(web_app_t *app, web_conn_t *c, bool logged_in) {
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_bool(&w, "logged_in", logged_in);
    jw_bool(&w, "must_change_password", !app->settings->pw_set);
    respond_json(c, 200, &w, NULL);
}

// Failed password attempts are counted per sender address: one address
// guessing cannot lock the owner out at another. They are forgotten
// WEB_LOCKOUT_FORGET_US after the last one. While every entry is taken,
// the addresses without one share one more entry instead of pushing others
// out: guessing from more addresses than the table holds would otherwise
// reset each count before it reaches the lockout.
static web_lockout_t *lockout_of(web_app_t *app, uint32_t ip, uint64_t t) {
    web_lockout_t *found = NULL;
    bool full = true;
    for (int i = 0; i < WEB_LOCKOUTS; i++) {
        web_lockout_t *l = &app->lockouts[i];
        if (l->failed && t - l->last_us >= WEB_LOCKOUT_FORGET_US) l->failed = 0;
        if (l->failed && l->ip == ip) found = l;
        if (!l->failed) full = false;
    }
    web_lockout_t *s = &app->lockout_shared;
    if (s->failed && t - s->last_us >= WEB_LOCKOUT_FORGET_US) s->failed = 0;
    if (!found && full && s->failed) found = s;
    return found;
}

// After failed password checks (login or password change): answer 429 and
// return true while the sender's lockout lasts.
static bool locked_out(web_app_t *app, web_conn_t *c, uint64_t t) {
    const web_lockout_t *l = lockout_of(app, c->peer_ip, t);
    if (!l || t >= l->until_us) return false;
    char msg[80];
    snprintf(msg, sizeof(msg), "Too many failed attempts. Please wait %u s.",
             (unsigned)((l->until_us - t) / 1000000 + 1));
    respond_error(c, 429, msg);
    return true;
}

static void password_failed(web_app_t *app, web_conn_t *c, uint64_t t) {
    web_lockout_t *l = lockout_of(app, c->peer_ip, t);
    if (!l) {
        // A free entry; none free: the shared one (see lockout_of).
        l = &app->lockout_shared;
        for (int i = 0; i < WEB_LOCKOUTS; i++) {
            if (!app->lockouts[i].failed) {
                l = &app->lockouts[i];
                break;
            }
        }
        *l = (web_lockout_t){.ip = c->peer_ip};
    }
    l->failed++;
    l->last_us = t;
    if (l->failed >= 3) {
        uint32_t shift = l->failed - 3 > 6 ? 6 : l->failed - 3;
        l->until_us = t + (1000000ull << shift);  // 1 s ... 64 s
    }
}

static void password_ok(web_app_t *app, web_conn_t *c, uint64_t t) {
    web_lockout_t *l = lockout_of(app, c->peer_ip, t);
    if (l) l->failed = 0;
}

static void h_login(web_app_t *app, web_conn_t *c) {
    const char *b = c->req.body;
    size_t bl = c->req.body_received;
    uint64_t t = now(app);
    if (locked_out(app, c, t)) return;
    char pw[PSHB_PASSWORD_MAX_BYTES + 1];
    if (!jr_str(b, bl, "password", pw, sizeof(pw))) {
        respond_error(c, 400, "Password missing");
        return;
    }
    bool ok = password_verify(app->settings, pw);
    crypto_wipe(pw, sizeof(pw));
    if (!ok) {
        password_failed(app, c, t);
        respond_error(c, 401, "Wrong password");
        return;
    }
    password_ok(app, c, t);
    char token[33];
    session_create(app, token);
    char cookie[128];
    snprintf(cookie, sizeof(cookie),
             "Set-Cookie: " COOKIE_NAME "=%s; Path=/; HttpOnly; SameSite=Strict\r\n", token);
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_bool(&w, "ok", true);
    jw_bool(&w, "must_change_password", !app->settings->pw_set);
    respond_json(c, 200, &w, cookie);
}

static void h_logout(web_app_t *app, web_conn_t *c, int session) {
    if (session >= 0) app->sessions[session].used = false;
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_bool(&w, "ok", true);
    respond_json(c, 200, &w, "Set-Cookie: " COOKIE_NAME "=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict\r\n");
}

// Change the password. Every other session ends: whoever logged in with the
// old (e.g. the factory) password must not keep access.
static void h_password(web_app_t *app, web_conn_t *c, int session) {
    const char *b = c->req.body;
    size_t bl = c->req.body_received;
    uint64_t t = now(app);
    if (locked_out(app, c, t)) return;
    char old_pw[PSHB_PASSWORD_MAX_BYTES + 1], new_pw[PSHB_PASSWORD_MAX_BYTES + 2];
    if (!jr_str(b, bl, "old", old_pw, sizeof(old_pw)) || !jr_str(b, bl, "new", new_pw, sizeof(new_pw))) {
        respond_error(c, 400, "Enter the current and the new password");
        return;
    }
    size_t n = strlen(new_pw) > PSHB_PASSWORD_MAX_BYTES ? PSHB_PASSWORD_MAX_LEN + 1 : password_chars(new_pw);
    bool old_ok = password_verify(app->settings, old_pw);
    crypto_wipe(old_pw, sizeof(old_pw));
    if (!old_ok) {
        password_failed(app, c, t);
        respond_error(c, 403, "Current password is wrong");
    } else if (n < PSHB_PASSWORD_MIN_LEN || n > PSHB_PASSWORD_MAX_LEN) {
        respond_error(c, 400, "The new password must be 8 to 64 characters long");
    } else if (strcmp(new_pw, PSHB_DEFAULT_PASSWORD) == 0) {
        respond_error(c, 400, "Please do not use the factory password");
    } else {
        uint8_t salt[16];
        app->be->random(app->be->ctx, salt, sizeof(salt));
        pshb_settings_t next = *app->settings;
        password_set(&next, new_pw, salt);
        if (save_apply(app, c, &next)) {
            password_ok(app, c, t);
            for (int i = 0; i < WEB_SESSIONS; i++) {
                if (i != session) app->sessions[i].used = false;
            }
            respond_ok(c);
        }
        crypto_wipe(&next, sizeof(next));
    }
    crypto_wipe(new_pw, sizeof(new_pw));
}

static void h_status(web_app_t *app, web_conn_t *c) {
    web_status_t st;
    memset(&st, 0, sizeof(st));
    app->be->status(app->be->ctx, &st);
    const pshb_settings_t *s = app->settings;
    char ip[16], mask[16], gw[16], dns[16], allowed[WEB_CIDR_STR_LEN + 1], mac[18], fw[16];
    ip_str(st.ip, ip);
    ip_str(st.mask, mask);
    ip_str(st.gw, gw);
    ip_str(st.dns, dns);
    sender_filter_str(s, allowed);
    snprintf(mac, sizeof(mac), "%02x:%02x:%02x:%02x:%02x:%02x", st.mac[0], st.mac[1], st.mac[2], st.mac[3],
             st.mac[4], st.mac[5]);
    snprintf(fw, sizeof(fw), "%u.%u.%u", (unsigned)(st.fw_version >> 24) & 0xFF,
             (unsigned)(st.fw_version >> 16) & 0xFF, (unsigned)st.fw_version & 0xFFFF);

    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    char host[PSHB_NAME_SIZE];
    network_name(app, host);
    jw_str(&w, "name", shown_name(app));
    jw_bool(&w, "name_set", settings_board_name(s)[0] != '\0');
    jw_str(&w, "default_name", app->hostname);
    jw_str(&w, "hostname", host);
    jw_str(&w, "firmware", fw);
    jw_str(&w, "partition", st.partition ? st.partition : "-");
    jw_bool(&w, "watchdog_reset", st.watchdog_reset);
    jw_int(&w, "uptime_ms", st.uptime_ms);
    jw_str(&w, "mac", mac);
    jw_bool(&w, "link_up", st.link_up);
    jw_str(&w, "net_state", st.net_state ? st.net_state : "");
    jw_str(&w, "ip", ip);
    jw_str(&w, "mask", mask);
    jw_str(&w, "gateway", gw);
    jw_str(&w, "dns", dns);
    jw_str(&w, "ip_mode", s->ip_mode == PSHB_IP_STATIC ? "static" : "dhcp");
    char cfg_ip[16], cfg_mask[16], cfg_gw[16], cfg_dns[16];
    ip_str(s->ip, cfg_ip);
    ip_str(s->mask, cfg_mask);
    ip_str(s->gw, cfg_gw);
    ip_str(s->dns, cfg_dns);
    jw_str(&w, "static_ip", cfg_ip);
    jw_str(&w, "static_mask", cfg_mask);
    jw_str(&w, "static_gateway", cfg_gw);
    jw_str(&w, "static_dns", cfg_dns);
    jw_int(&w, "udp_port", s->udp_port);
    jw_bool(&w, "psk_set", s->psk_set);
    jw_str(&w, "allowed_sender", allowed);
    jw_bool(&w, "usb_mounted", st.usb_status & PSHB_STATUS_USB_MOUNTED);
    jw_bool(&w, "usb_suspended", st.usb_status & PSHB_STATUS_USB_SUSPENDED);
    jw_bool(&w, "input_active", !st.failsafe);
    jw_int(&w, "rx_ok", st.rx_ok);
    jw_int(&w, "rx_auth_fail", st.rx_auth_fail);
    jw_int(&w, "rx_replay", st.rx_replay);
    jw_int(&w, "rx_filtered", st.rx_filtered);
    jw_int(&w, "rx_malformed", st.rx_malformed);
    jw_int(&w, "lat_usb_count", st.lat_usb_count);
    jw_int(&w, "lat_usb_p50_us", st.lat_usb_p50_us);
    jw_int(&w, "lat_usb_p95_us", st.lat_usb_p95_us);
    jw_int(&w, "lat_usb_max_us", st.lat_usb_max_us);
    jw_int(&w, "lat_rx_p50_us", st.lat_rx_p50_us);
    jw_int(&w, "lat_rx_max_us", st.lat_rx_max_us);
    jw_int(&w, "mouse_inputs", st.mouse_inputs);
    jw_int(&w, "mouse_waits", st.mouse_waits);
    jw_int(&w, "mouse_misses", st.mouse_misses);
    jw_int(&w, "mouse_lead_us", st.mouse_lead_us);
    respond_json(c, 200, &w, NULL);
}

static void jw_lat(json_writer_t *w, const char *name, const web_lat_t *l) {
    static const char *const part[] = {"n", "min", "p50", "p99", "max"};
    const uint32_t v[] = {l->n, l->min, l->p50, l->p99, l->max};
    for (int i = 0; i < 5; i++) {
        char key[24];
        snprintf(key, sizeof(key), "%s_%s", name, part[i]);
        jw_int(w, key, v[i]);
    }
}

static void h_diag(web_app_t *app, web_conn_t *c) {
    web_diag_t d;
    memset(&d, 0, sizeof(d));
    app->be->diag(app->be->ctx, &d);
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_lat(&w, "ready", &d.ready);
    jw_lat(&w, "ready1", &d.ready1);
    jw_lat(&w, "detect", &d.detect);
    jw_lat(&w, "bus", &d.bus);
    jw_lat(&w, "spi", &d.spi);
    jw_lat(&w, "handle", &d.handle);
    jw_lat(&w, "usb", &d.usb);
    jw_lat(&w, "gap", &d.gap);
    jw_lat(&w, "armed", &d.armed);
    jw_int(&w, "rounds", d.rounds);
    jw_int(&w, "datagrams", d.datagrams);
    jw_int(&w, "core0_spi", d.core0_spi);
    jw_int(&w, "cr_busy", d.cr_busy);
    jw_int(&w, "mouse_inputs", d.mouse_inputs);
    jw_int(&w, "mouse_waits", d.mouse_waits);
    jw_int(&w, "mouse_fetches", d.mouse_fetches);
    jw_int(&w, "mouse_misses", d.mouse_misses);
    jw_int(&w, "mouse_lead_us", d.mouse_lead_us);
    jw_int(&w, "mouse_host_skips", d.mouse_host_skips);
    jw_int(&w, "mouse_floor_us", d.mouse_floor_us);
    jw_int(&w, "crypto_min_ns", d.crypto_min_ns);
    jw_int(&w, "crypto_avg_ns", d.crypto_avg_ns);
    jw_int(&w, "crypto_max_ns", d.crypto_max_ns);
    jw_bool(&w, "crypto_fast", d.crypto_fast);
    jw_int(&w, "crypto_helped", d.crypto_helped);
    jw_int(&w, "crypto_alone", d.crypto_alone);
    jw_int(&w, "spi_hz", d.spi_hz);
    jw_int(&w, "bus_timeouts", d.bus_timeouts);
    respond_json(c, 200, &w, NULL);
}

static void h_clock_get(web_app_t *app, web_conn_t *c) {
    web_clock_t now;
    memset(&now, 0, sizeof(now));
    if (app->be->clock) app->be->clock(app->be->ctx, &now);
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_int(&w, "cpu_mhz", app->settings->cpu_200mhz ? 200 : 150);
    jw_bool(&w, "spi_fast", app->settings->spi_fast);
    jw_int(&w, "cpu_hz", now.cpu_hz);
    jw_int(&w, "spi_hz", now.spi_hz);
    jw_bool(&w, "cpu_fell_back", now.cpu_fell_back);
    jw_bool(&w, "spi_fast_failed", now.spi_fast_failed);
    respond_json(c, 200, &w, NULL);
}

// {"cpu_mhz": 150 | 200, "spi_fast": bool}; both optional. A change is
// stored and the board restarts (the clocks are set at start).
static void h_clock_set(web_app_t *app, web_conn_t *c) {
    const char *b = c->req.body;
    size_t bl = c->req.body_received;
    pshb_settings_t next = *app->settings;
    long long mhz;
    bool fast;
    if (jr_int(b, bl, "cpu_mhz", &mhz)) {
        if (mhz != 150 && mhz != 200) {
            respond_error(c, 400, "CPU clock must be 150 or 200 MHz");
            return;
        }
        next.cpu_200mhz = mhz == 200;
    }
    if (jr_bool(b, bl, "spi_fast", &fast)) next.spi_fast = fast;
    bool changed = next.cpu_200mhz != app->settings->cpu_200mhz || next.spi_fast != app->settings->spi_fast;
    if (changed && !save_apply(app, c, &next)) return;
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_bool(&w, "ok", true);
    jw_bool(&w, "reboot", changed);
    respond_json(c, 200, &w, NULL);
    if (changed) c->after = WEB_AFTER_REBOOT;
}

// The challenge a signed request or upload names in X-PSHB-Nonce, or,
// without that header (older apps), the newest one. -1 (answered with 403)
// if there is no such challenge that is still fresh. It is used up only by
// a request whose signature is right (use_challenge): a forged request
// cannot take the app's.
static int find_challenge(web_app_t *app, web_conn_t *c) {
    const http_req_t *r = &c->req;
    uint64_t t = now(app);
    int pick = -1;
    for (int i = 0; i < WEB_CHALLENGES; i++) {
        uint64_t until = app->challenges[i].until_us;
        if (until == 0) continue;
        if (t >= until) {
            app->challenges[i].until_us = 0;
            continue;
        }
        if (r->has_nonce ? memcmp(app->challenges[i].nonce, r->nonce, 16) == 0
                         : pick < 0 || until > app->challenges[pick].until_us) {
            pick = i;
        }
    }
    if (pick < 0) respond_error(c, 403, "Challenge missing or expired (GET /api/ota/challenge first)");
    return pick;
}

static void use_challenge(web_app_t *app, int i) {
    app->challenges[i].until_us = 0;
    crypto_wipe(app->challenges[i].nonce, sizeof(app->challenges[i].nonce));
}

// A request signed with the key (the app): a fresh challenge (used up
// here if the signature is right), and BLAKE2b-256(key = K_ota, "pshb-req-v1" || nonce ||
// method || " " || path || "\n" || body). Returns 1 if it is valid, 0 if
// the request is not signed, -1 if it is signed wrongly (answered).
static int check_signature(web_app_t *app, web_conn_t *c) {
    const http_req_t *r = &c->req;
    if (!r->has_auth) return 0;
    if (!app->settings->psk_set) {
        respond_error(c, 403, "No key set on the board");
        return -1;
    }
    int ch = find_challenge(app, c);
    if (ch < 0) return -1;
    uint8_t key[32], mac[32];
    ota_key(app->settings->psk, key);
    crypto_blake2b_ctx h;
    crypto_blake2b_keyed_init(&h, 32, key, sizeof(key));
    crypto_wipe(key, sizeof(key));
    const char *method = r->method == HTTP_POST ? "POST" : "GET";
    crypto_blake2b_update(&h, (const uint8_t *)REQ_MAC_LABEL, strlen(REQ_MAC_LABEL));
    crypto_blake2b_update(&h, app->challenges[ch].nonce, sizeof(app->challenges[ch].nonce));
    crypto_blake2b_update(&h, (const uint8_t *)method, strlen(method));
    crypto_blake2b_update(&h, (const uint8_t *)" ", 1);
    crypto_blake2b_update(&h, (const uint8_t *)r->path, strlen(r->path));
    crypto_blake2b_update(&h, (const uint8_t *)"\n", 1);
    crypto_blake2b_update(&h, (const uint8_t *)r->body, r->body_received);
    crypto_blake2b_final(&h, mac);
    if (crypto_verify32(mac, r->auth) != 0) {
        respond_error(c, 403, "Signature does not match (wrong key?)");
        return -1;
    }
    use_challenge(app, ch);
    return 1;
}

// The board's name: shown in the app, the search and here, and as network
// name (derived) sent to the router. Empty = the default name.
static void h_name(web_app_t *app, web_conn_t *c) {
    char raw[96];
    if (!jr_str(c->req.body, c->req.body_received, "name", raw, sizeof(raw))) {
        respond_error(c, 400, "Name missing or too long (at most 31 bytes)");
        return;
    }
    char *s = raw;
    while (*s == ' ') s++;
    size_t n = strlen(s);
    while (n && s[n - 1] == ' ') s[--n] = '\0';
    if (n >= PSHB_NAME_SIZE) {
        respond_error(c, 400, "The name may have at most 31 bytes (an umlaut counts as two)");
        return;
    }
    if (!pshb_name_valid(s)) {
        respond_error(c, 400, "The name must not contain control characters");
        return;
    }
    pshb_settings_t next = *app->settings;
    memset(next.name, 0, sizeof(next.name));
    memcpy(next.name, s, n);
    if (!save_apply(app, c, &next)) return;
    char host[PSHB_NAME_SIZE];
    network_name(app, host);
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_bool(&w, "ok", true);
    jw_str(&w, "name", shown_name(app));
    jw_str(&w, "hostname", host);
    respond_json(c, 200, &w, NULL);
}

// What a signed request may do: read status and diagnostics, start the
// diagnostics over, set the clocks, name the board. Password, key, network
// settings and firmware stay with their own ways in.
static void handle_signed(web_app_t *app, web_conn_t *c) {
    const http_req_t *r = &c->req;
    const char *p = r->path;
    if (r->method == HTTP_GET && strcmp(p, "/api/status") == 0) {
        h_status(app, c);
    } else if (r->method == HTTP_GET && strcmp(p, "/api/diag") == 0 && app->be->diag) {
        h_diag(app, c);
    } else if (r->method == HTTP_POST && strcmp(p, "/api/diag/reset") == 0 && app->be->diag_reset) {
        app->be->diag_reset(app->be->ctx);
        respond_ok(c);
    } else if (r->method == HTTP_POST && strcmp(p, "/api/clock") == 0) {
        h_clock_set(app, c);
    } else if (r->method == HTTP_POST && strcmp(p, "/api/name") == 0) {
        h_name(app, c);
    } else {
        respond_error(c, 403, "Not allowed with a signature");
    }
}

static void h_network(web_app_t *app, web_conn_t *c) {
    const char *b = c->req.body;
    size_t bl = c->req.body_received;
    char mode[8], ip[16], mask[16], gw[16], dns[16];
    long long port;
    pshb_settings_t s = *app->settings;
    if (!jr_str(b, bl, "mode", mode, sizeof(mode)) || !jr_int(b, bl, "udp_port", &port)) {
        respond_error(c, 400, "Mode and UDP port are required");
        return;
    }
    // 67/68: DHCP (the board's DHCP client listens on 68 and would get the
    // input); the discovery port has its own socket.
    if (port < 1 || port > 65535 || port == 67 || port == 68 || port == PSHB_DISCOVERY_PORT) {
        respond_error(c, 400,
                      "UDP port must be between 1 and 65535 (not 67 or 68, used by DHCP, nor 45046, used by the app's search)");
        return;
    }
    s.udp_port = (uint16_t)port;
    if (strcmp(mode, "dhcp") == 0) {
        s.ip_mode = PSHB_IP_DHCP;
    } else if (strcmp(mode, "static") == 0) {
        s.ip_mode = PSHB_IP_STATIC;
        if (!jr_str(b, bl, "ip", ip, sizeof(ip)) || !web_parse_ip(ip, s.ip) || ip_zero(s.ip)) {
            respond_error(c, 400, "Invalid IP address");
            return;
        }
        if (!jr_str(b, bl, "mask", mask, sizeof(mask)) || !web_parse_ip(mask, s.mask) || !mask_valid(s.mask)) {
            respond_error(c, 400, "Invalid subnet mask");
            return;
        }
        // An address nobody could reach would need a factory reset.
        if (!web_host_ip_valid(s.ip, s.mask)) {
            respond_error(c, 400, "This IP address cannot be used by a device in this subnet");
            return;
        }
        if (!jr_str(b, bl, "gateway", gw, sizeof(gw)) || !(gw[0] == '\0' || web_parse_ip(gw, s.gw))) {
            respond_error(c, 400, "Invalid gateway");
            return;
        }
        if (gw[0] == '\0') memset(s.gw, 0, 4);
        uint32_t m = ip_u32(s.mask);
        if (!ip_zero(s.gw) && (!web_host_ip_valid(s.gw, s.mask) || (ip_u32(s.gw) & m) != (ip_u32(s.ip) & m) ||
                               ip_u32(s.gw) == ip_u32(s.ip))) {
            respond_error(c, 400, "The gateway must be another address in the same subnet");
            return;
        }
        if (!jr_str(b, bl, "dns", dns, sizeof(dns)) || !(dns[0] == '\0' || web_parse_ip(dns, s.dns))) {
            respond_error(c, 400, "Invalid DNS server");
            return;
        }
        if (dns[0] == '\0') memset(s.dns, 0, 4);
    } else {
        respond_error(c, 400, "Mode must be dhcp or static");
        return;
    }
    if (!save_apply(app, c, &s)) return;
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_bool(&w, "ok", true);
    jw_bool(&w, "reboot", true);
    respond_json(c, 200, &w, NULL);
    c->after = WEB_AFTER_REBOOT;  // new addresses take effect after a restart
}

static void h_psk_generate(web_app_t *app, web_conn_t *c) {
    pshb_settings_t next = *app->settings;
    app->be->random(app->be->ctx, next.psk, sizeof(next.psk));
    next.psk_set = 1;
    bool saved = save_apply(app, c, &next);
    crypto_wipe(&next, sizeof(next));
    if (!saved) return;
    char hex[65];
    hex_encode(app->settings->psk, 32, hex);
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_str(&w, "psk", hex);
    respond_json(c, 200, &w, NULL);
    crypto_wipe(hex, sizeof(hex));
}

static void h_psk_set(web_app_t *app, web_conn_t *c) {
    char hex[80];
    uint8_t key[32];
    if (!jr_str(c->req.body, c->req.body_received, "psk", hex, sizeof(hex)) || !hex_decode(hex, key, 32)) {
        respond_error(c, 400, "The key must consist of 64 hex digits");
        return;
    }
    pshb_settings_t next = *app->settings;
    memcpy(next.psk, key, 32);
    next.psk_set = 1;
    crypto_wipe(key, sizeof(key));
    crypto_wipe(hex, sizeof(hex));
    if (save_apply(app, c, &next)) respond_ok(c);
    crypto_wipe(&next, sizeof(next));
}

static void h_allowed_sender(web_app_t *app, web_conn_t *c) {
    char text[WEB_CIDR_STR_LEN + 1];
    uint8_t v[4] = {0};
    uint8_t prefix = 0;  // empty = any sender
    if (!jr_str(c->req.body, c->req.body_received, "ip", text, sizeof(text)) ||
        !(text[0] == '\0' || web_parse_cidr(text, v, &prefix))) {
        respond_error(c, 400, "Invalid IP address or subnet (e.g. 192.168.1.20 or 192.168.1.0/24; empty = any sender)");
        return;
    }
    if (prefix > 0 && ip_zero(v)) {
        // e.g. "0.0.0.0/8": would be stored as "any sender", which is not what was asked for.
        respond_error(c, 400, "Invalid subnet: the network address must not be 0.0.0.0 (empty = any sender)");
        return;
    }
    if (prefix == 0) memset(v, 0, sizeof(v));  // "/0" = any sender
    pshb_settings_t next = *app->settings;
    memcpy(next.allowed_ip, v, 4);
    next.allowed_host_bits = prefix ? (uint8_t)(32 - prefix) : 0;
    if (save_apply(app, c, &next)) respond_ok(c);
}

// Key for signed firmware uploads, derived from the PSK (which itself is
// only used for the packet encryption).
static void ota_key(const uint8_t psk[32], uint8_t out[32]) {
    crypto_blake2b_keyed(out, 32, psk, 32, (const uint8_t *)OTA_KEY_LABEL, strlen(OTA_KEY_LABEL));
}

// A fresh random challenge for the next signed request or upload. It takes
// a free (or expired) place; else the oldest of the asker's own, so asking
// again and again cannot push out the challenge another program waits to
// use; else the oldest one.
static void h_ota_challenge(web_app_t *app, web_conn_t *c) {
    if (!app->settings->psk_set) {
        respond_error(c, 403, "No key set on the board");
        return;
    }
    uint64_t t = now(app);
    int slot = -1, own = -1, oldest = 0;
    for (int i = 0; i < WEB_CHALLENGES; i++) {
        uint64_t until = app->challenges[i].until_us;
        if (until == 0 || t >= until) {
            slot = i;
            break;
        }
        if (app->challenges[i].ip == c->peer_ip && (own < 0 || until < app->challenges[own].until_us)) own = i;
        if (until < app->challenges[oldest].until_us) oldest = i;
    }
    if (slot < 0) slot = own >= 0 ? own : oldest;
    uint8_t *nonce = app->challenges[slot].nonce;
    app->be->random(app->be->ctx, nonce, 16);
    app->challenges[slot].until_us = t + OTA_CHALLENGE_US;
    app->challenges[slot].ip = c->peer_ip;
    char hex[2 * 16 + 1];
    hex_encode(nonce, 16, hex);
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_str(&w, "nonce", hex);
    respond_json(c, 200, &w, NULL);
}

static const web_asset_t *find_asset(const char *path) {
    if (strcmp(path, "/") == 0) path = "/index.html";
    for (size_t i = 0; i < web_assets_count; i++) {
        if (strcmp(web_assets[i].path, path) == 0) return &web_assets[i];
    }
    return NULL;
}

static bool is_ipv4(const char *s, size_t n) {
    int dots = 0, digits = 0;
    for (size_t i = 0; i < n; i++) {
        if (s[i] == '.') {
            if (!digits) return false;
            dots++;
            digits = 0;
        } else if (s[i] >= '0' && s[i] <= '9') {
            if (++digits > 3) return false;
        } else {
            return false;
        }
    }
    return dots == 3 && digits;
}

// Against DNS rebinding: a web page whose name an attacker points at the
// board's address would be the board's origin in the browser (with the
// factory password it could even log in). Allowed: an address, the
// board's default name and its network name (also with a domain:
// "ps-hid-bridge-a1b2c3.fritz.box"), localhost (the simulator), and no
// Host at all (HTTP/1.0 tools).
static bool host_allowed(const web_app_t *app, const http_req_t *r) {
    if (!r->has_host) return true;
    if (r->host_bad) return false;
    const char *h = r->host;
    if (h[0] == '[') return strchr(h, ']') != NULL;  // IPv6 address (names have no brackets)
    size_t n = strcspn(h, ":");                      // without the port
    if (is_ipv4(h, n)) return true;
    if (n == 9 && strncasecmp(h, "localhost", 9) == 0) return true;
    size_t label = strcspn(h, ".:");
    size_t hn = strlen(app->hostname);
    if (hn && label == hn && strncasecmp(h, app->hostname, hn) == 0) return true;
    char net[PSHB_NAME_SIZE];
    network_name(app, net);
    size_t nn = strlen(net);
    return nn && label == nn && strncasecmp(h, net, nn) == 0;
}

// False (answered with 421) if the Host is not allowed.
static bool check_host(web_app_t *app, web_conn_t *c) {
    if (host_allowed(app, &c->req)) return true;
    char msg[128], net[PSHB_NAME_SIZE];
    network_name(app, net);
    snprintf(msg, sizeof(msg), "Open the board by its address%s%s instead of another name",
             net[0] ? " or its name " : "", net);
    respond_error(c, 421, msg);
    return false;
}

static void handle(web_app_t *app, web_conn_t *c) {
    http_req_t *r = &c->req;
    const char *p = r->path;

    if (!check_host(app, c)) return;

    if (strncmp(p, "/api/", 5) != 0) {
        if (r->method != HTTP_GET) {
            respond_error(c, 405, "Method not allowed");
            return;
        }
        const web_asset_t *a = find_asset(p);
        if (!a) {
            respond_error(c, 404, "Not found");
            return;
        }
        respond(c, 200, a->type, a->data, a->len, true, NULL);
        return;
    }

    if (r->method != HTTP_GET && r->method != HTTP_POST) {
        respond_error(c, 405, "Method not allowed");
        return;
    }
    int session = r->session[0] ? session_find(app, r->session) : -1;
    bool logged_in = session >= 0;

    if (r->method == HTTP_POST && !r->csrf_header) {
        respond_error(c, 403, "Request rejected (X-PSHB header missing)");
        return;
    }
    if (strcmp(p, "/api/state") == 0 && r->method == HTTP_GET) {
        h_state(app, c, logged_in);
        return;
    }
    if (strcmp(p, "/api/login") == 0 && r->method == HTTP_POST) {
        h_login(app, c);
        return;
    }
    if (strcmp(p, "/api/ota/challenge") == 0 && r->method == HTTP_GET) {
        h_ota_challenge(app, c);  // no login: the signed request itself proves the key
        return;
    }
    if (strcmp(p, "/api/clock") == 0 && r->method == HTTP_GET) {
        h_clock_get(app, c);  // no login: clock speeds are no secret
        return;
    }
    int sig = check_signature(app, c);
    if (sig < 0) return;
    if (sig > 0) {
        handle_signed(app, c);
        return;
    }
    if (!logged_in) {
        respond_error(c, 401, "Please log in");
        return;
    }
    if (strcmp(p, "/api/logout") == 0 && r->method == HTTP_POST) {
        h_logout(app, c, session);
        return;
    }
    if (strcmp(p, "/api/password") == 0 && r->method == HTTP_POST) {
        h_password(app, c, session);
        return;
    }
    if (!app->settings->pw_set) {
        respond_error(c, 403, "Please set your own password first");
        return;
    }
    if (r->method == HTTP_GET) {
        if (strcmp(p, "/api/status") == 0) {
            h_status(app, c);
            return;
        }
        if (strcmp(p, "/api/diag") == 0 && app->be->diag) {
            h_diag(app, c);
            return;
        }
    } else if (r->method == HTTP_POST) {
        if (strcmp(p, "/api/diag/reset") == 0 && app->be->diag_reset) {
            app->be->diag_reset(app->be->ctx);
            respond_ok(c);
            return;
        }
        if (strcmp(p, "/api/network") == 0) {
            h_network(app, c);
            return;
        }
        if (strcmp(p, "/api/psk/generate") == 0) {
            h_psk_generate(app, c);
            return;
        }
        if (strcmp(p, "/api/psk") == 0) {
            h_psk_set(app, c);
            return;
        }
        if (strcmp(p, "/api/allowed-sender") == 0) {
            h_allowed_sender(app, c);
            return;
        }
        if (strcmp(p, "/api/name") == 0) {
            h_name(app, c);
            return;
        }
        if (strcmp(p, "/api/clock") == 0) {
            h_clock_set(app, c);
            return;
        }
        if (strcmp(p, "/api/reboot") == 0) {
            respond_ok(c);
            c->after = WEB_AFTER_REBOOT;
            return;
        }
        if (strcmp(p, "/api/factory-reset") == 0) {
            respond_ok(c);
            c->after = WEB_AFTER_FACTORY_RESET;
            sessions_clear(app);
            return;
        }
    }
    respond_error(c, 404, "Not found");
}

// ---------------------------------------------------------------- connection

void web_app_init(web_app_t *app, const web_backend_t *be, pshb_settings_t *settings) {
    memset(app, 0, sizeof(*app));
    app->be = be;
    app->settings = settings;
}

void web_app_set_hostname(web_app_t *app, const char *name) {
    snprintf(app->hostname, sizeof(app->hostname), "%s", name);
}

void web_conn_reset(web_conn_t *c, uint64_t now_us) {
    http_req_init(&c->req);
    c->head_len = 0;
    c->body = NULL;
    c->body_len = 0;
    c->sent = 0;
    c->responding = false;
    c->after = WEB_AFTER_NONE;
    c->started_us = now_us;
    c->ota = false;
    c->ota_failed = false;
    c->sink = false;
    c->sink_bytes = 0;
    c->discard_left = 0;
    c->peer_ip = 0;
}

typedef struct {
    web_app_t *app;
    web_conn_t *conn;
} body_ctx_t;

static void ota_body(void *ctx, const uint8_t *data, size_t len) {
    body_ctx_t *b = ctx;
    if (b->conn->sink) {
        b->conn->sink_bytes += (uint32_t)len;  // bench sink: counted and dropped
        return;
    }
    if (b->app->ota_signed && b->app->ota_owner == b->conn) crypto_blake2b_update(&b->app->ota_hash, data, len);
    if (!b->conn->ota_failed && !b->app->be->ota_write(b->app->be->ctx, data, len)) b->conn->ota_failed = true;
}

// Signed upload: the key must be set, the challenge fresh (used up here
// once the signature is right, so a recorded upload cannot be sent again)
// and the signature right,
// all before a single byte reaches the flash. The signature covers the hash
// of the body; the body itself is hashed while it streams in and compared
// at the end (ota_done).
static bool ota_signed_start(web_app_t *app, web_conn_t *c) {
    const http_req_t *r = &c->req;
    if (!app->settings->psk_set) {
        respond_error(c, 403, "No key set on the board");
        return false;
    }
    if (!r->has_ota_auth2 || !r->has_ota_hash) {
        // The old signature (X-PSHB-OTA-Auth alone) is checked only after
        // the whole image: by then it would be in the flash already.
        respond_error(c, 403, "Please update the app: this board needs a newer signature for firmware updates");
        return false;
    }
    int ch = find_challenge(app, c);
    if (ch < 0) return false;
    uint8_t key[32], mac[32];
    ota_key(app->settings->psk, key);
    crypto_blake2b_ctx h;
    crypto_blake2b_keyed_init(&h, 32, key, sizeof(key));
    crypto_wipe(key, sizeof(key));
    uint32_t n = r->content_length;
    const uint8_t len[4] = {(uint8_t)n, (uint8_t)(n >> 8), (uint8_t)(n >> 16), (uint8_t)(n >> 24)};
    crypto_blake2b_update(&h, (const uint8_t *)OTA_MAC_LABEL, strlen(OTA_MAC_LABEL));
    crypto_blake2b_update(&h, app->challenges[ch].nonce, sizeof(app->challenges[ch].nonce));
    crypto_blake2b_update(&h, len, sizeof(len));
    crypto_blake2b_update(&h, r->ota_hash, sizeof(r->ota_hash));
    crypto_blake2b_final(&h, mac);
    if (crypto_verify32(mac, r->ota_auth2) != 0) {
        respond_error(c, 403, "Signature does not match (wrong key?)");
        return false;
    }
    use_challenge(app, ch);
    memcpy(app->ota_signed_hash, r->ota_hash, sizeof(app->ota_signed_hash));
    crypto_blake2b_init(&app->ota_hash, 32);
    return true;
}

// Same rules as the web interface's firmware upload (logged in, own
// password set, X-PSHB header): only then may a body be streamed.
static bool upload_allowed(web_app_t *app, web_conn_t *c) {
    http_req_t *r = &c->req;
    if (!check_host(app, c)) return false;
    if (!r->csrf_header) {
        respond_error(c, 403, "Request rejected (X-PSHB header missing)");
        return false;
    }
    if (!r->session[0] || session_find(app, r->session) < 0) {
        respond_error(c, 401, "Please log in");
        return false;
    }
    if (!app->settings->pw_set) {
        respond_error(c, 403, "Please set your own password first");
        return false;
    }
    return true;
}

static void sink_done(web_conn_t *c) {
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_bool(&w, "ok", true);
    jw_int(&w, "bytes", c->sink_bytes);
    respond_json(c, 200, &w, NULL);
}

// Headers of POST /api/ota are complete: authorise, then stream the body.
// Returns false if a response was produced already.
// Either logged in (web interface) or signed with the key (app, CLI).
static bool ota_start(web_app_t *app, web_conn_t *c) {
    http_req_t *r = &c->req;
    bool signed_upload = r->has_ota_auth || r->has_ota_auth2 || r->has_ota_hash;
    if (!check_host(app, c)) return false;
    if (!signed_upload && !upload_allowed(app, c)) return false;
    if (!r->has_crc) {
        respond_error(c, 400, "Checksum (X-PSHB-CRC32) missing");
        return false;
    }
    if (app->ota_owner && app->ota_owner != c) {
        respond_error(c, 409, "Another firmware update is in progress");
        return false;
    }
    if (signed_upload && !ota_signed_start(app, c)) return false;
    char err[96] = "Update not possible";
    if (!app->be->ota_begin || !app->be->ota_begin(app->be->ctx, r->content_length, err, sizeof(err))) {
        respond_error(c, 400, err);
        return false;
    }
    c->ota = true;
    app->ota_owner = c;  // until this connection is closed
    app->ota_signed = signed_upload;
    return true;
}

static void ota_done(web_app_t *app, web_conn_t *c) {
    if (app->ota_signed) {
        uint8_t hash[32];
        crypto_blake2b_final(&app->ota_hash, hash);
        app->ota_signed = false;
        if (crypto_verify32(hash, app->ota_signed_hash) != 0) {
            app->be->ota_abort(app->be->ctx);
            respond_error(c, 403, "The file does not match its signature");
            return;
        }
    }
    // finish() also reports why an earlier write failed (bad file, flash error).
    char err[96] = "Writing to flash failed";
    bool ok = app->be->ota_finish(app->be->ctx, c->req.crc, err, sizeof(err));
    if (!ok || c->ota_failed) {
        app->be->ota_abort(app->be->ctx);
        respond_error(c, 400, err);
        return;
    }
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_bool(&w, "ok", true);
    jw_bool(&w, "reboot", true);
    respond_json(c, 200, &w, NULL);
    c->after = WEB_AFTER_OTA;
}

void web_conn_input(web_app_t *app, web_conn_t *c, const uint8_t *data, size_t len) {
    if (c->responding) {
        // One request per connection; after an early answer the rest of its
        // body is read and thrown away.
        c->discard_left -= len < c->discard_left ? (uint32_t)len : c->discard_left;
        return;
    }
    body_ctx_t bctx = {app, c};
    while (len) {
        size_t n = http_req_feed(&c->req, data, len, ota_body, &bctx);
        data += n;
        len -= n;
        if (c->req.state == HTTP_PARSE_BODY && !c->req.body_decided) {
            // The body beyond this piece, should the answer come first.
            uint32_t rest = c->req.content_length > len ? c->req.content_length - (uint32_t)len : 0;
            if (c->req.method == HTTP_POST && strcmp(c->req.path, "/api/ota") == 0) {
                if (!ota_start(app, c)) {
                    c->discard_left = rest;
                    return;
                }
                http_req_decide(&c->req, true);
            } else if (c->req.method == HTTP_POST && app->bench_sink &&
                       strcmp(c->req.path, "/api/bench/sink") == 0) {
                if (!upload_allowed(app, c)) {
                    c->discard_left = rest;
                    return;
                }
                c->sink = true;
                http_req_decide(&c->req, true);
            } else {
                http_req_decide(&c->req, false);  // all other routes take a small JSON body
                if (c->req.state == HTTP_PARSE_ERROR) c->discard_left = rest;  // too large
            }
            continue;
        }
        if (n == 0) break;
        if (c->req.state != HTTP_PARSE_HEADERS && c->req.state != HTTP_PARSE_BODY) break;
    }
    if (c->req.state == HTTP_PARSE_ERROR) {
        if (c->ota) app->be->ota_abort(app->be->ctx);
        respond_error(c, c->req.error_status ? c->req.error_status : 400, "Invalid request");
    } else if (c->req.state == HTTP_PARSE_DONE) {
        if (c->ota) {
            ota_done(app, c);
        } else if (c->sink) {
            sink_done(c);
        } else {
            handle(app, c);
        }
    }
}

size_t web_conn_output(const web_conn_t *c, const uint8_t **data) {
    if (!c->responding) return 0;
    if (c->sent < c->head_len) {
        *data = (const uint8_t *)c->head + c->sent;
        return c->head_len - c->sent;
    }
    size_t body_sent = c->sent - c->head_len;
    if (body_sent < c->body_len) {
        *data = c->body + body_sent;
        return c->body_len - body_sent;
    }
    return 0;
}

void web_conn_sent(web_app_t *app, web_conn_t *c, size_t n) {
    c->sent += n;
    if (c->responding && c->sent >= c->head_len + c->body_len) {
        web_after_t after = c->after;
        c->after = WEB_AFTER_NONE;
        if (after == WEB_AFTER_REBOOT) app->be->reboot(app->be->ctx);
        if (after == WEB_AFTER_FACTORY_RESET) app->be->factory_reset(app->be->ctx);
        if (after == WEB_AFTER_OTA) app->be->ota_activate(app->be->ctx);
    }
}

void web_conn_close(web_app_t *app, web_conn_t *c) {
    if (c->ota && !c->responding) app->be->ota_abort(app->be->ctx);
    c->ota = false;
    if (app->ota_owner == c) {
        app->ota_owner = NULL;
        app->ota_signed = false;
    }
}

bool web_conn_done(const web_conn_t *c) {
    return c->responding && c->sent >= c->head_len + c->body_len && c->discard_left == 0;
}

void web_conn_input_end(web_conn_t *c) {
    c->discard_left = 0;
}
