#include "w5500_port.h"

#include <stdio.h>
#include <string.h>

#include "hardware/gpio.h"
#include "hardware/spi.h"
#include "hardware/sync.h"
#include "pico/mutex.h"
#include "pico/platform.h"
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
static uint32_t s_spi_hz;
// Diagnostics: SPI transactions started by core 0, and when INTn last fell
// (0 = not since it was taken).
static volatile uint32_t s_core0_spi;
static volatile uint64_t s_int_us;
static uint32_t s_cr_busy;  // Sn_CR still busy before a command (timing image)

// ---------------------------------------------------------------- ioLibrary callbacks

// Bus priority for core 0 (the input path). Core 1 moves long transfers in
// pieces (cmake/iolibrary.cmake) and, before each one, gives way while core
// 0 waits for the bus: core 0 then waits at most for the piece in progress.
// Only before its outermost access: core 1 must not wait while it already
// holds the bus (core 0 would wait for it in turn).
static volatile bool s_core0_wants;
static int s_core1_depth;

static void cris_enter(void) {
    if (get_core_num() != 0) {
        if (s_core1_depth == 0) {
            while (s_core0_wants) {
                tight_loop_contents();
            }
        }
        recursive_mutex_enter_blocking(&s_mutex);
        s_core1_depth++;
        return;
    }
    recursive_mutex_enter_blocking(&s_mutex);
}

static void cris_exit(void) {
    if (get_core_num() != 0) s_core1_depth--;
    recursive_mutex_exit(&s_mutex);
}
static void cs_select(void) {
    if (get_core_num() == 0) s_core0_spi++;
    gpio_put(W5500_PIN_CS, 0);
}
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

static void int_edge(uint gpio, uint32_t events) {
    (void)gpio;
    (void)events;
    if (!s_int_us) s_int_us = time_us_64();
}

#if PSHB_W5500_SPI_FAST
// Faster SPI clock (PSHB_W5500_SPI_FAST): above the 33.3 MHz the datasheet
// guarantees, so it has to prove itself first. Patterns through the TX
// buffer of the unused socket 7 (1 KiB) and one of its registers, read back
// at the new clock; any difference means back to the safe clock.
static bool spi_selftest(void) {
    for (int i = 0; i < 4; i++) {
        if (getVERSIONR() != 0x04) return false;
    }
    uint8_t out[256], in[256];
    uint32_t x = 0x9E3779B9u;
    for (int round = 0; round < 16; round++) {
        for (size_t i = 0; i < sizeof(out); i++) {
            x = x * 1664525u + 1013904223u;  // any pattern; also 0x00/0xFF runs below
            out[i] = (uint8_t)(round == 0 ? 0x00 : round == 1 ? 0xFF : round == 2 ? (i & 1 ? 0x55 : 0xAA)
                                                                                   : (x >> 24));
        }
        uint32_t addr = ((uint32_t)((round * 256) & 0x3FF) << 8) + (WIZCHIP_TXBUF_BLOCK(7) << 3);
        WIZCHIP_WRITE_BUF(addr, out, sizeof(out));
        memset(in, 0, sizeof(in));
        WIZCHIP_READ_BUF(addr, in, sizeof(in));
        if (memcmp(in, out, sizeof(in)) != 0) return false;
    }
    for (uint16_t v = 1; v; v = (uint16_t)(v << 1)) {
        setSn_PORT(7, v);
        if (getSn_PORT(7) != v) return false;
    }
    setSn_PORT(7, 0);
    return true;
}
#endif

static void make_mac(uint8_t mac[6]) {
    pico_unique_board_id_t id;
    pico_get_unique_board_id(&id);
    mac[0] = 0x02;  // locally administered, unicast
    memcpy(&mac[1], &id.id[3], 5);
}

bool w5500_port_init(void) {
    recursive_mutex_init(&s_mutex);

    s_spi_hz = spi_init(W5500_SPI, PSHB_W5500_SPI_HZ);
    // SPI mode 3 (the W5500 takes modes 0 and 3): in mode 0 the PL022 pulls
    // its frame signal up between bytes and leaves the clock idle for about
    // 1.5 bit times after each one; with CPHA = 1 the bytes follow back to
    // back.
    spi_set_format(W5500_SPI, 8, SPI_CPOL_1, SPI_CPHA_1, SPI_MSB_FIRST);
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

#if PSHB_W5500_SPI_FAST
    s_spi_hz = spi_set_baudrate(W5500_SPI, PSHB_W5500_SPI_FAST_HZ);
    if (spi_selftest()) {
        printf("w5500: SPI at %lu Hz\n", (unsigned long)s_spi_hz);
    } else {
        s_spi_hz = spi_set_baudrate(W5500_SPI, PSHB_W5500_SPI_HZ);
        printf("w5500: SPI self-test failed at the fast clock, back to %lu Hz\n", (unsigned long)s_spi_hz);
    }
#endif

    // Socket buffers in KB (16 KB each direction in total):
    // 0 DHCP, 1 input UDP, 2-3 HTTP, 4-7 unused.
    uint8_t tx[8] = {2, 2, 4, 4, 1, 1, 1, 1};
    uint8_t rx[8] = {W5500_DHCP_RX_KB, 4, 4, 4, 1, 1, 0, 0};
    if (wizchip_init(tx, rx) != 0) return false;

    make_mac(s_mac);
    setSHAR(s_mac);

    // Retransmission: 100 ms x 3 tries instead of 200 ms x 8, so a missing
    // ARP peer costs at most ~0.4 s on core 1 (core 0 never waits on it).
    setRTR(1000);
    setRCR(2);

    setSIMR(0);
    // Only a timestamp (diagnostics): the datagram itself is read in the
    // main loop. Registered on core 0, which calls this function.
    gpio_set_irq_enabled_with_callback(W5500_PIN_INT, GPIO_IRQ_EDGE_FALL, true, int_edge);
    return true;
}

void w5500_get_mac(uint8_t mac[6]) {
    memcpy(mac, s_mac, 6);
}

void w5500_lock(void) {
    s_core0_wants = true;  // core 1 starts no new piece meanwhile
    recursive_mutex_enter_blocking(&s_mutex);
    s_core0_wants = false;
}

void w5500_unlock(void) {
    recursive_mutex_exit(&s_mutex);
}

bool w5500_int_asserted(void) {
    return !gpio_get(W5500_PIN_INT);
}

uint64_t w5500_int_time_take(void) {
    uint32_t irq = save_and_disable_interrupts();  // 64-bit value, written by the IRQ
    uint64_t t = s_int_us;
    s_int_us = 0;
    restore_interrupts(irq);
    return t;
}

uint32_t w5500_core0_spi(void) {
    return s_core0_spi;
}

void w5500_core0_spi_reset(void) {
    s_core0_spi = 0;
    s_cr_busy = 0;
}

uint32_t w5500_spi_hz(void) {
    return s_spi_hz;
}

// ---------------------------------------------------------------- UDP fast path

// Commands to a socket. The W5500 takes a command within a few of its own
// clock cycles and then clears Sn_CR; datasheet code waits for that after
// every command (one or more SPI transactions). Here the wait happens only
// if the previous command to this socket is very recent; the next command
// of the input path comes several SPI transactions (> 5 µs) later anyway.
// The timing image checks every 16th command and counts how often Sn_CR
// was still busy (expected: never).
#define CMD_SETTLE_US 5
static uint32_t s_cmd_us[_WIZCHIP_SOCK_NUM_];
#if PSHB_TIMING_GPIO
static uint32_t s_cr_checks;
#endif

static void command(uint8_t sn, uint8_t cmd) {
#if PSHB_TIMING_GPIO
    if ((++s_cr_checks & 15) == 0 && getSn_CR(sn)) s_cr_busy++;
#endif
    if (time_us_32() - s_cmd_us[sn] < CMD_SETTLE_US) {
        while (getSn_CR(sn)) {
        }
    }
    setSn_CR(sn, cmd);
    s_cmd_us[sn] = time_us_32();
}

uint32_t w5500_cr_busy(void) {
    return s_cr_busy;
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

// Sn_RX_RSR read until two reads agree, as the datasheet asks: the chip may
// update it between its two bytes. Only needed when a single read does not
// fit the datagrams (see w5500_udp_recv).
static uint16_t rx_size_consistent(uint8_t sn) {
    uint16_t a = read16(Sn_RX_RSR(sn));
    for (;;) {
        uint16_t again = read16(Sn_RX_RSR(sn));
        if (again == a) return a;
        a = again;
    }
}

// Per socket: read pointer after the datagrams read so far, and whether it
// still has to be written back (w5500_udp_commit).
static uint16_t s_rx_rd[_WIZCHIP_SOCK_NUM_];
static bool s_rx_pending[_WIZCHIP_SOCK_NUM_];

static void rx_read(uint8_t sn, uint16_t ptr, uint8_t *buf, uint16_t len) {
    // The chip wraps reads at the end of the socket's buffer by itself.
    WIZCHIP_READ_BUF(((uint32_t)ptr << 8) + (WIZCHIP_RXBUF_BLOCK(sn) << 3), buf, len);
}

void w5500_udp_commit(uint8_t sn) {
    if (!s_rx_pending[sn]) return;
    write16(Sn_RX_RD(sn), s_rx_rd[sn]);
    command(sn, Sn_CR_RECV);
    s_rx_pending[sn] = false;
}

int w5500_udp_recv(uint8_t sn, uint8_t *buf, uint16_t cap, uint8_t ip[4], uint16_t *port, uint16_t *left) {
    uint16_t avail, ptr;
    bool fresh = *left == 0;
    if (!fresh) {
        avail = *left;
        ptr = s_rx_rd[sn];
    } else {
        // Asking the chip anew: what was read before must be released first
        // (Sn_RX_RSR still counts it until then).
        w5500_udp_commit(sn);
        // Size and read pointer are neighbours (Sn_RX_RSR 0x26-0x27,
        // Sn_RX_RD 0x28-0x29): one burst. The size is read once: its high
        // byte comes first, so a value torn by an update in between is never
        // larger than the real one, and every byte it counts is there. If a
        // datagram does not fit, look again (below).
        uint8_t b[4];
        WIZCHIP_READ_BUF(Sn_RX_RSR(sn), b, 4);
        avail = (uint16_t)((b[0] << 8) | b[1]);
        ptr = (uint16_t)((b[2] << 8) | b[3]);
    }
    *left = 0;
    if (avail < 8) return W5500_RX_EMPTY;

    // In UDP mode every datagram is preceded by an 8-byte header:
    // source IP (4), source port (2), payload length (2).
    // Header and a typical payload in one go.
    uint8_t first[FAST_RX_FIRST];
    uint16_t n_first = avail < sizeof(first) ? avail : (uint16_t)sizeof(first);
    rx_read(sn, ptr, first, n_first);
    memcpy(ip, first, 4);
    *port = (uint16_t)((first[4] << 8) | first[5]);
    uint16_t len = (uint16_t)((first[6] << 8) | first[7]);
    uint32_t total = 8u + len;
    if (total > avail) {
        if (!fresh) {
            // The size read at the start of the round may have been torn:
            // ask the chip again in a new round (nothing is lost meanwhile).
            return W5500_RX_AGAIN;
        }
        uint16_t again = rx_size_consistent(sn);
        if (total > again) {
            // Inconsistent with what the chip reports: drop everything.
            s_rx_rd[sn] = (uint16_t)(ptr + again);
            s_rx_pending[sn] = true;
            w5500_udp_commit(sn);
            return W5500_RX_OVERSIZE;
        }
        avail = again;
        if (n_first < sizeof(first) && n_first < total) {
            // The first read was cut short by the smaller size: read the rest.
            uint16_t more = (uint16_t)((total < sizeof(first) ? total : sizeof(first)) - n_first);
            rx_read(sn, (uint16_t)(ptr + n_first), first + n_first, more);
            n_first = (uint16_t)(n_first + more);
        }
    }
    int result = W5500_RX_OVERSIZE;
    if (len <= cap) {
        uint16_t have = (uint16_t)(n_first - 8 < len ? n_first - 8 : len);
        memcpy(buf, first + 8, have);
        if (have < len) rx_read(sn, (uint16_t)(ptr + 8 + have), buf + have, (uint16_t)(len - have));
        result = len;
    }
    // Released later, once per round (w5500_udp_commit).
    s_rx_rd[sn] = (uint16_t)(ptr + total);
    s_rx_pending[sn] = true;
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
    command(sn, Sn_CR_RECV);
    *left = avail > 8u + len ? (uint16_t)(avail - 8u - len) : 0;
    return len <= cap ? (int)len : W5500_RX_OVERSIZE;
}

void w5500_udp_commit(uint8_t sn) {
    (void)sn;  // every datagram was released right away
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
    command(sn, Sn_CR_SEND);
    s_tx_busy[sn] = true;
    return true;
}
