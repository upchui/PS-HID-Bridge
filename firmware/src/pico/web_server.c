#include "web_server.h"

#include "pico/time.h"

#include "socket.h"
#include "w5500_port.h"
#include "wizchip_conf.h"

#define HTTP_PORT 80
#define CHUNK 1024  // bounds the time core 0 may wait for the SPI bus
#define CONN_TIMEOUT_US 60000000u  // generous: a firmware upload takes a few seconds
#define CLOSE_TIMEOUT_US 5000000u  // a close the peer does not complete

// The sockets are non-blocking: in blocking mode ioLibrary's send() waits
// for room in the transmit buffer and disconnect() for the peer to finish
// the close -- a client that stops reading or never closes would stop core 1
// (DHCP, web interface, BOOTSEL reset) for good.
static web_app_t s_app;
static web_conn_t s_conn[2];
static bool s_open[2];
static bool s_closing[2];  // disconnect() issued, the chip completes the close
static uint64_t s_closing_since[2];
static uint8_t s_rx[CHUNK];

static void end_conn(int i, uint8_t sn) {
    web_conn_close(&s_app, &s_conn[i]);
    disconnect(sn);  // returns at once; the socket goes CLOSED when done
    s_open[i] = false;
    s_closing[i] = true;
    s_closing_since[i] = time_us_64();
}

void web_server_init(const web_backend_t *be, pshb_settings_t *settings) {
    web_app_init(&s_app, be, settings);
}

static void serve(int i, uint8_t sn) {
    web_conn_t *c = &s_conn[i];
    uint64_t now = time_us_64();

    if (getSn_IR(sn) & Sn_IR_CON) {
        setSn_IR(sn, Sn_IR_CON);
        web_conn_reset(c, now);
        s_open[i] = true;
    }
    if (!s_open[i]) {
        web_conn_reset(c, now);
        s_open[i] = true;
    }

    uint16_t avail = getSn_RX_RSR(sn);
    if (avail) {
        int32_t n = recv(sn, s_rx, avail < CHUNK ? avail : CHUNK);
        if (n > 0) web_conn_input(&s_app, c, s_rx, (size_t)n);
    }

    const uint8_t *out;
    size_t len = web_conn_output(c, &out);
    if (len) {
        if (len > CHUNK) len = CHUNK;
        // SOCK_BUSY (0): no room yet or the last chunk still on its way; next round.
        int32_t n = send(sn, (uint8_t *)out, (uint16_t)len);
        if (n > 0) web_conn_sent(&s_app, c, (size_t)n);
    }

    if (web_conn_done(c) || now - c->started_us > CONN_TIMEOUT_US) end_conn(i, sn);
}

bool web_server_busy(void) {
    return s_open[0] || s_open[1];
}

void web_server_service(void) {
    for (int i = 0; i < 2; i++) {
        uint8_t sn = (uint8_t)(W5500_SOCK_HTTP0 + i);
        uint8_t sr = getSn_SR(sn);
        if (s_closing[i] && sr != SOCK_CLOSED) {
            // Closing (FIN_WAIT, LAST_ACK, ...; until the chip has sent the
            // FIN even still ESTABLISHED): serve nothing more on it, and
            // drop it if the peer never completes the close.
            if (time_us_64() - s_closing_since[i] > CLOSE_TIMEOUT_US) close(sn);
            continue;
        }
        switch (sr) {
        case SOCK_ESTABLISHED:
            serve(i, sn);
            break;
        case SOCK_CLOSE_WAIT:
            // The client finished sending; answer what we have, then close.
            if (getSn_RX_RSR(sn) > 0 || (s_open[i] && s_conn[i].responding && !web_conn_done(&s_conn[i]))) {
                serve(i, sn);
            } else {
                end_conn(i, sn);
            }
            break;
        case SOCK_INIT:
            listen(sn);
            break;
        case SOCK_CLOSED:
            // Closed by the peer (reset) while open: end the request
            // properly, e.g. abort an interrupted firmware upload.
            if (s_open[i]) web_conn_close(&s_app, &s_conn[i]);
            s_open[i] = false;
            s_closing[i] = false;
            socket(sn, Sn_MR_TCP, HTTP_PORT, SF_IO_NONBLOCK);
            break;
        default:
            break;  // SYNRECV, FIN_WAIT after the peer's close, ...: the chip times these out
        }
    }
}

void web_server_drain(uint32_t timeout_ms) {
    absolute_time_t until = make_timeout_time_ms(timeout_ms);
    while (!time_reached(until)) {
        bool busy = false;
        for (int i = 0; i < 2; i++) {
            uint8_t sr = getSn_SR((uint8_t)(W5500_SOCK_HTTP0 + i));
            if (s_closing[i] && sr != SOCK_CLOSED) busy = true;
        }
        if (!busy) return;
        sleep_ms(5);
    }
}
