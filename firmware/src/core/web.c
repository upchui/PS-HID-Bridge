#include "web.h"

#include <stdio.h>
#include <string.h>

#include "hexutil.h"
#include "json.h"
#include "monocypher.h"
#include "password.h"
#include "protocol.h"
#include "web_assets.h"

#define COOKIE_NAME "pshb_session"

// ---------------------------------------------------------------- helpers

bool web_parse_ip(const char *s, uint8_t out[4]) {
    unsigned v[4];
    char tail;
    if (sscanf(s, "%u.%u.%u.%u%c", &v[0], &v[1], &v[2], &v[3], &tail) != 4) return false;
    for (int i = 0; i < 4; i++) {
        if (v[i] > 255) return false;
        out[i] = (uint8_t)v[i];
    }
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
    case 413: return "Payload Too Large";
    case 414: return "URI Too Long";
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
        "Content-Security-Policy: default-src 'self'; frame-ancestors 'none'\r\n"
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

static bool save_apply(web_app_t *app, web_conn_t *c) {
    if (!app->be->save(app->be->ctx, app->settings)) {
        respond_error(c, 500, "Saving to flash failed");
        return false;
    }
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

static void h_login(web_app_t *app, web_conn_t *c) {
    const char *b = c->req.body;
    size_t bl = c->req.body_received;
    uint64_t t = now(app);
    if (t < app->lockout_until_us) {
        char msg[80];
        snprintf(msg, sizeof(msg), "Too many failed attempts. Please wait %u s.",
                 (unsigned)((app->lockout_until_us - t) / 1000000 + 1));
        respond_error(c, 429, msg);
        return;
    }
    char pw[PSHB_PASSWORD_MAX_LEN + 1];
    if (!jr_str(b, bl, "password", pw, sizeof(pw))) {
        respond_error(c, 400, "Password missing");
        return;
    }
    bool ok = password_verify(app->settings, pw);
    crypto_wipe(pw, sizeof(pw));
    if (!ok) {
        app->failed_logins++;
        if (app->failed_logins >= 3) {
            uint32_t shift = app->failed_logins - 3 > 6 ? 6 : app->failed_logins - 3;
            app->lockout_until_us = t + (1000000ull << shift);  // 1 s ... 64 s
        }
        respond_error(c, 401, "Wrong password");
        return;
    }
    app->failed_logins = 0;
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

static void h_password(web_app_t *app, web_conn_t *c) {
    const char *b = c->req.body;
    size_t bl = c->req.body_received;
    char old_pw[PSHB_PASSWORD_MAX_LEN + 1], new_pw[PSHB_PASSWORD_MAX_LEN + 2];
    if (!jr_str(b, bl, "old", old_pw, sizeof(old_pw)) || !jr_str(b, bl, "new", new_pw, sizeof(new_pw))) {
        respond_error(c, 400, "Enter the current and the new password");
        return;
    }
    size_t n = strlen(new_pw);
    bool old_ok = password_verify(app->settings, old_pw);
    crypto_wipe(old_pw, sizeof(old_pw));
    if (!old_ok) {
        respond_error(c, 403, "Current password is wrong");
    } else if (n < PSHB_PASSWORD_MIN_LEN || n > PSHB_PASSWORD_MAX_LEN) {
        respond_error(c, 400, "The new password must be 8 to 64 characters long");
    } else if (strcmp(new_pw, PSHB_DEFAULT_PASSWORD) == 0) {
        respond_error(c, 400, "Please do not use the factory password");
    } else {
        uint8_t salt[16];
        app->be->random(app->be->ctx, salt, sizeof(salt));
        password_set(app->settings, new_pw, salt);
        if (save_apply(app, c)) respond_ok(c);
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
    snprintf(fw, sizeof(fw), "%u.%u.%u", (unsigned)(st.fw_version >> 16) & 0xFF,
             (unsigned)(st.fw_version >> 8) & 0xFF, (unsigned)st.fw_version & 0xFF);

    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_str(&w, "firmware", fw);
    jw_str(&w, "partition", st.partition ? st.partition : "-");
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
    respond_json(c, 200, &w, NULL);
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
    if (port < 1 || port > 65535 || port == 80) {
        respond_error(c, 400, "UDP port must be between 1 and 65535 (not 80)");
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
        if (!jr_str(b, bl, "gateway", gw, sizeof(gw)) || !(gw[0] == '\0' || web_parse_ip(gw, s.gw))) {
            respond_error(c, 400, "Invalid gateway");
            return;
        }
        if (gw[0] == '\0') memset(s.gw, 0, 4);
        if (!jr_str(b, bl, "dns", dns, sizeof(dns)) || !(dns[0] == '\0' || web_parse_ip(dns, s.dns))) {
            respond_error(c, 400, "Invalid DNS server");
            return;
        }
        if (dns[0] == '\0') memset(s.dns, 0, 4);
    } else {
        respond_error(c, 400, "Mode must be dhcp or static");
        return;
    }
    *app->settings = s;
    if (!save_apply(app, c)) return;
    json_writer_t w;
    jw_begin(&w, c->dyn, sizeof(c->dyn));
    jw_bool(&w, "ok", true);
    jw_bool(&w, "reboot", true);
    respond_json(c, 200, &w, NULL);
    c->after = WEB_AFTER_REBOOT;  // new addresses take effect after a restart
}

static void h_psk_generate(web_app_t *app, web_conn_t *c) {
    app->be->random(app->be->ctx, app->settings->psk, sizeof(app->settings->psk));
    app->settings->psk_set = 1;
    if (!save_apply(app, c)) return;
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
    memcpy(app->settings->psk, key, 32);
    app->settings->psk_set = 1;
    crypto_wipe(key, sizeof(key));
    crypto_wipe(hex, sizeof(hex));
    if (save_apply(app, c)) respond_ok(c);
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
    memcpy(app->settings->allowed_ip, v, 4);
    app->settings->allowed_host_bits = prefix ? (uint8_t)(32 - prefix) : 0;
    if (save_apply(app, c)) respond_ok(c);
}

static const web_asset_t *find_asset(const char *path) {
    if (strcmp(path, "/") == 0) path = "/index.html";
    for (size_t i = 0; i < web_assets_count; i++) {
        if (strcmp(web_assets[i].path, path) == 0) return &web_assets[i];
    }
    return NULL;
}

static void handle(web_app_t *app, web_conn_t *c) {
    http_req_t *r = &c->req;
    const char *p = r->path;

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
    if (!logged_in) {
        respond_error(c, 401, "Please log in");
        return;
    }
    if (strcmp(p, "/api/logout") == 0 && r->method == HTTP_POST) {
        h_logout(app, c, session);
        return;
    }
    if (strcmp(p, "/api/password") == 0 && r->method == HTTP_POST) {
        h_password(app, c);
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
    } else if (r->method == HTTP_POST) {
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
    c->close_early = false;
}

typedef struct {
    web_app_t *app;
    web_conn_t *conn;
} body_ctx_t;

static void ota_body(void *ctx, const uint8_t *data, size_t len) {
    body_ctx_t *b = ctx;
    if (!b->conn->ota_failed && !b->app->be->ota_write(b->app->be->ctx, data, len)) b->conn->ota_failed = true;
}

// Headers of POST /api/ota are complete: authorise, then stream the body.
// Returns false if a response was produced already.
static bool ota_start(web_app_t *app, web_conn_t *c) {
    http_req_t *r = &c->req;
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
    if (!r->has_crc) {
        respond_error(c, 400, "Checksum (X-PSHB-CRC32) missing");
        return false;
    }
    char err[96] = "Update not possible";
    if (!app->be->ota_begin || !app->be->ota_begin(app->be->ctx, r->content_length, err, sizeof(err))) {
        respond_error(c, 400, err);
        return false;
    }
    c->ota = true;
    return true;
}

static void ota_done(web_app_t *app, web_conn_t *c) {
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
    if (c->responding) return;  // one request per connection
    body_ctx_t bctx = {app, c};
    while (len) {
        size_t n = http_req_feed(&c->req, data, len, ota_body, &bctx);
        data += n;
        len -= n;
        if (c->req.state == HTTP_PARSE_BODY && !c->req.body_decided) {
            if (c->req.method == HTTP_POST && strcmp(c->req.path, "/api/ota") == 0) {
                if (!ota_start(app, c)) {
                    c->close_early = true;
                    return;
                }
                http_req_decide(&c->req, true);
            } else {
                http_req_decide(&c->req, false);  // all other routes take a small JSON body
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
}

bool web_conn_done(const web_conn_t *c) {
    return c->responding && c->sent >= c->head_len + c->body_len;
}
