#include "w5500_port.h"

#include <string.h>

#include "hardware/gpio.h"
#include "hardware/spi.h"
#include "pico/mutex.h"
#include "pico/time.h"
#include "pico/unique_id.h"

#include "pshb_config.h"
#include "wizchip_conf.h"

#define W5500_SPI spi0

// Receive with a few burst accesses instead of the ioLibrary sequence
// (about 7 instead of about 20 SPI transactions per datagram). 0 = use the
// ioLibrary functions.
#ifndef PSHB_W5500_FAST_RX
#define PSHB_W5500_FAST_RX 1
#endif

// Typical datagram from the app (INPUT/HEARTBEAT): read this much at once.
#define FAST_RX_FIRST (8 + 49)

static recursive_mutex_t s_mutex;
static uint8_t s_mac[6];
static bool s_tx_busy[8];

// ---------------------------------------------------------------- ioLibrary callbacks

static void cris_enter(void) { recursive_mutex_enter_blocking(&s_mutex); }
static void cris_exit(void) { recursive_mutex_exit(&s_mutex); }
static void cs_select(void) { gpio_put(W5500_PIN_CS, 0); }
static void cs_deselect(void) { gpio_put(W5500_PIN_CS, 1); }

static uint8_t spi_read_byte(void) {
    uint8_t b;
    spi_read_blocking(W5500_SPI, 0x00, &b, 1);
    return b;
}

static void spi_write_byte(uint8_t b) {
    spi_write_blocking(W5500_SPI, &b, 1);
}

static void spi_read_burst(uint8_t *buf, uint16_t len) {
    spi_read_blocking(W5500_SPI, 0x00, buf, len);
}

static void spi_write_burst(uint8_t *buf, uint16_t len) {
    spi_write_blocking(W5500_SPI, buf, len);
}

// ---------------------------------------------------------------- init

static void make_mac(uint8_t mac[6]) {
    pico_unique_board_id_t id;
    pico_get_unique_board_id(&id);
    mac[0] = 0x02;  // locally administered, unicast
    memcpy(&mac[1], &id.id[3], 5);
}

bool w5500_port_init(void) {
    recursive_mutex_init(&s_mutex);

    spi_init(W5500_SPI, PSHB_W5500_SPI_HZ);
    gpio_set_function(W5500_PIN_SCK, GPIO_FUNC_SPI);
    gpio_set_function(W5500_PIN_MOSI, GPIO_FUNC_SPI);
    gpio_set_function(W5500_PIN_MISO, GPIO_FUNC_SPI);

    gpio_init(W5500_PIN_CS);
    gpio_set_dir(W5500_PIN_CS, GPIO_OUT);
    gpio_put(W5500_PIN_CS, 1);

    gpio_init(W5500_PIN_INT);
    gpio_set_dir(W5500_PIN_INT, GPIO_IN);
    gpio_pull_up(W5500_PIN_INT);

    // Hardware reset: >= 500 us low, then wait for the PLL to lock.
    gpio_init(W5500_PIN_RST);
    gpio_set_dir(W5500_PIN_RST, GPIO_OUT);
    gpio_put(W5500_PIN_RST, 0);
    sleep_ms(2);
    gpio_put(W5500_PIN_RST, 1);

    reg_wizchip_cris_cbfunc(cris_enter, cris_exit);
    reg_wizchip_cs_cbfunc(cs_select, cs_deselect);
    reg_wizchip_spi_cbfunc(spi_read_byte, spi_write_byte);
    reg_wizchip_spiburst_cbfunc(spi_read_burst, spi_write_burst);

    // VERSIONR reads 0x04 once the chip is up.
    absolute_time_t deadline = make_timeout_time_ms(200);
    while (getVERSIONR() != 0x04) {
        if (time_reached(deadline)) return false;
        sleep_ms(5);
    }

    // Socket buffers in KB (16 KB each direction in total):
    // 0 DHCP, 1 input UDP, 2-3 HTTP, 4-7 unused.
    uint8_t tx[8] = {2, 2, 4, 4, 1, 1, 1, 1};
    uint8_t rx[8] = {2, 4, 4, 4, 1, 1, 0, 0};
    if (wizchip_init(tx, rx) != 0) return false;

    make_mac(s_mac);
    setSHAR(s_mac);

    // Retransmission: 100 ms x 3 tries instead of 200 ms x 8, so a missing
    // ARP peer costs at most ~0.4 s on core 1 (core 0 never waits on it).
    setRTR(1000);
    setRCR(2);

    setSIMR(0);
    return true;
}

void w5500_get_mac(uint8_t mac[6]) {
    memcpy(mac, s_mac, 6);
}

void w5500_lock(void) {
    recursive_mutex_enter_blocking(&s_mutex);
}

void w5500_unlock(void) {
    recursive_mutex_exit(&s_mutex);
}

bool w5500_int_asserted(void) {
    return !gpio_get(W5500_PIN_INT);
}

// ---------------------------------------------------------------- UDP fast path

static void exec_cmd(uint8_t sn, uint8_t cmd) {
    setSn_CR(sn, cmd);
    while (getSn_CR(sn)) {
        // The W5500 clears the command register within a few cycles.
    }
}

#if PSHB_W5500_FAST_RX

static uint16_t read16(uint32_t addr) {
    uint8_t b[2];
    WIZCHIP_READ_BUF(addr, b, 2);
    return (uint16_t)((b[0] << 8) | b[1]);
}

static void write16(uint32_t addr, uint16_t v) {
    uint8_t b[2] = {(uint8_t)(v >> 8), (uint8_t)v};
    WIZCHIP_WRITE_BUF(addr, b, 2);
}

// The chip may update the received-size register while we read it: read
// until two reads agree (as the datasheet asks).
static uint16_t rx_size(uint8_t sn) {
    uint16_t a, b = read16(Sn_RX_RSR(sn));
    do {
        a = b;
        b = read16(Sn_RX_RSR(sn));
    } while (a != b);
    return a;
}

static void rx_read(uint8_t sn, uint16_t ptr, uint8_t *buf, uint16_t len) {
    // The chip wraps reads at the end of the socket's buffer by itself.
    WIZCHIP_READ_BUF(((uint32_t)ptr << 8) + (WIZCHIP_RXBUF_BLOCK(sn) << 3), buf, len);
}

int w5500_udp_recv(uint8_t sn, uint8_t *buf, uint16_t cap, uint8_t ip[4], uint16_t *port, uint16_t *left) {
    uint16_t avail = *left ? *left : rx_size(sn);
    *left = 0;
    if (avail < 8) return W5500_RX_EMPTY;

    // In UDP mode every datagram is preceded by an 8-byte header:
    // source IP (4), source port (2), payload length (2).
    // Read header and a typical payload in one go.
    uint8_t first[FAST_RX_FIRST];
    uint16_t ptr = read16(Sn_RX_RD(sn));
    uint16_t n_first = avail < sizeof(first) ? avail : (uint16_t)sizeof(first);
    rx_read(sn, ptr, first, n_first);
    memcpy(ip, first, 4);
    *port = (uint16_t)((first[4] << 8) | first[5]);
    uint16_t len = (uint16_t)((first[6] << 8) | first[7]);
    uint32_t total = 8u + len;
    if (total > avail) {
        // Inconsistent with what the chip reported: drop everything.
        write16(Sn_RX_RD(sn), (uint16_t)(ptr + avail));
        exec_cmd(sn, Sn_CR_RECV);
        return W5500_RX_OVERSIZE;
    }
    int result = W5500_RX_OVERSIZE;
    if (len <= cap) {
        uint16_t have = (uint16_t)(n_first - 8 < len ? n_first - 8 : len);
        memcpy(buf, first + 8, have);
        if (have < len) rx_read(sn, (uint16_t)(ptr + 8 + have), buf + have, (uint16_t)(len - have));
        result = len;
    }
    write16(Sn_RX_RD(sn), (uint16_t)(ptr + total));
    exec_cmd(sn, Sn_CR_RECV);
    *left = (uint16_t)(avail - total);
    return result;
}

#else

int w5500_udp_recv(uint8_t sn, uint8_t *buf, uint16_t cap, uint8_t ip[4], uint16_t *port, uint16_t *left) {
    uint16_t avail = getSn_RX_RSR(sn);
    *left = 0;
    if (avail < 8) return W5500_RX_EMPTY;

    // In UDP mode every datagram is preceded by an 8-byte header:
    // source IP (4), source port (2), payload length (2).
    uint8_t head[8];
    wiz_recv_data(sn, head, sizeof(head));
    memcpy(ip, head, 4);
    *port = (uint16_t)((head[4] << 8) | head[5]);
    uint16_t len = (uint16_t)((head[6] << 8) | head[7]);

    uint16_t n = len <= cap ? len : 0;
    if (n) wiz_recv_data(sn, buf, n);
    if (len > n) wiz_recv_ignore(sn, (uint16_t)(len - n));
    exec_cmd(sn, Sn_CR_RECV);
    *left = avail > 8u + len ? (uint16_t)(avail - 8u - len) : 0;
    return len <= cap ? (int)len : W5500_RX_OVERSIZE;
}

#endif

void w5500_udp_tx_poll(uint8_t sn) {
    if (!s_tx_busy[sn]) return;
    uint8_t ir = getSn_IR(sn);
    if (ir & (Sn_IR_SENDOK | Sn_IR_TIMEOUT)) {
        setSn_IR(sn, ir & (Sn_IR_SENDOK | Sn_IR_TIMEOUT));
        s_tx_busy[sn] = false;
    }
}

bool w5500_udp_send_nb(uint8_t sn, const uint8_t *buf, uint16_t len, const uint8_t ip[4],
                       uint16_t port) {
    w5500_udp_tx_poll(sn);
    if (s_tx_busy[sn] || getSn_TX_FSR(sn) < len) return false;
    setSn_DIPR(sn, (uint8_t *)ip);
    setSn_DPORT(sn, port);
    wiz_send_data(sn, (uint8_t *)buf, len);
    exec_cmd(sn, Sn_CR_SEND);
    s_tx_busy[sn] = true;
    return true;
}
