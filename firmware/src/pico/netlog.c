#include "netlog.h"

#include <stdio.h>
#include <string.h>

#include "hardware/gpio.h"
#include "hardware/uart.h"
#include "pico/critical_section.h"
#include "pico/stdio/driver.h"
#include "pico/stdlib.h"

#include "dhcp.h"
#include "logbuf.h"
#include "socket.h"
#include "w5500_port.h"
#include "wizchip_conf.h"

#define LOG_SIZE        (48 * 1024)
#define KEEP_MAGIC      0x214C4F47u   // "GOL!"
#define NETLOG_SOCK     W5500_SOCK_UDP
#define SUB_TIMEOUT_US  (30u * 1000u * 1000u)
#define LINK_POLL_US    500000u
#define DHCP_POLL_US    10000u
#define DHCP_RETRY_US   (5u * 1000u * 1000u)
#define SEND_CHUNK      1024u
#define SENDS_PER_ROUND 4

// The log survives a watchdog restart (not a power cycle), so what led to a
// hang is still there afterwards.
typedef struct {
    uint32_t magic;
    logbuf_t log;
} keep_t;
static char __uninitialized_ram(s_mem)[LOG_SIZE];
static keep_t __uninitialized_ram(s_keep);

static critical_section_t s_lock;
static bool s_line_start = true;

// UART0: what was taken from the log but not yet written.
static uint64_t s_uart_pos;
static char s_upend[64];
static uint32_t s_ulen, s_uoff;

// Network.
typedef enum { N_OFF, N_NO_LINK, N_DHCP, N_DHCP_WAIT, N_READY } net_state_t;
static net_state_t s_net = N_OFF;
static pshb_settings_t s_cfg;
static uint8_t s_dhcp_buf[W5500_DHCP_RX_KB * 1024];
static volatile bool s_dhcp_assigned, s_dhcp_conflict;
static uint64_t s_next_link, s_next_tick, s_next_dhcp, s_dhcp_retry;
static bool s_sock_open;

// The subscriber (tools/boardlog.py).
static bool s_sub;
static uint8_t s_sub_ip[4];
static uint16_t s_sub_port;
static uint64_t s_sub_seen, s_net_pos;
// Where this boot's log starts: a receiver that was there before a restart
// (it only sends "ka") gets the log from here on, not everything again.
static uint64_t s_boot_pos;
static netlog_cmd_fn s_cmd;

// "12345.678901 " (seconds since boot).
static uint32_t fmt_time(char *out, uint64_t us) {
    uint32_t s = (uint32_t)(us / 1000000u), f = (uint32_t)(us % 1000000u);
    char digits[10];
    int n = 0;
    do {
        digits[n++] = (char)('0' + s % 10);
        s /= 10;
    } while (s);
    uint32_t k = 0;
    for (int i = n; i < 5; i++) out[k++] = ' ';
    while (n) out[k++] = digits[--n];
    out[k++] = '.';
    for (uint32_t d = 100000; d; d /= 10) out[k++] = (char)('0' + (f / d) % 10);
    out[k++] = ' ';
    return k;
}

static void drv_out_chars(const char *buf, int len) {
    critical_section_enter_blocking(&s_lock);
    int i = 0;
    while (i < len) {
        if (s_line_start) {
            char ts[20];
            logbuf_write(&s_keep.log, ts, fmt_time(ts, time_us_64()));
            s_line_start = false;
        }
        const char *nl = memchr(buf + i, '\n', (size_t)(len - i));
        int end = nl ? (int)(nl - buf) + 1 : len;
        logbuf_write(&s_keep.log, buf + i, (uint32_t)(end - i));
        if (nl) s_line_start = true;
        i = end;
    }
    critical_section_exit(&s_lock);
}

static stdio_driver_t s_driver = {
    .out_chars = drv_out_chars,
    .crlf_enabled = false,
};

void netlog_init(void) {
    critical_section_init(&s_lock);
    bool kept = s_keep.magic == KEEP_MAGIC && s_keep.log.mem == s_mem && s_keep.log.cap == LOG_SIZE &&
                s_keep.log.head < (1ull << 40);
    if (!kept) {
        logbuf_init(&s_keep.log, s_mem, LOG_SIZE);
        s_keep.magic = KEEP_MAGIC;
    }
    s_uart_pos = s_keep.log.head;  // the UART saw the old part already
    s_boot_pos = s_keep.log.head;
    uart_init(uart0, 115200);
    gpio_set_function(PICO_DEFAULT_UART_TX_PIN, GPIO_FUNC_UART);
    gpio_set_function(PICO_DEFAULT_UART_RX_PIN, GPIO_FUNC_UART);
    stdio_set_driver_enabled(&s_driver, true);
    if (kept) {
        static const char mark[] = "\n----- restart (the log before it is kept) -----\n";
        drv_out_chars(mark, sizeof mark - 1);
    }
}

void netlog_on_command(netlog_cmd_fn fn) {
    s_cmd = fn;
}

void netlog_hex(const char *what, const void *data, uint32_t len) {
    const uint8_t *p = data;
    if (len == 0) {
        printf("%s: (empty)\n", what);
        return;
    }
    for (uint32_t off = 0; off < len; off += 32) {
        char line[32 * 3 + 1];
        uint32_t n = len - off < 32 ? len - off : 32;
        for (uint32_t i = 0; i < n; i++) {
            static const char hex[] = "0123456789abcdef";
            line[3 * i] = hex[p[off + i] >> 4];
            line[3 * i + 1] = hex[p[off + i] & 15];
            line[3 * i + 2] = ' ';
        }
        line[3 * n - 1] = '\0';
        printf("%s +%03lu: %s\n", what, (unsigned long)off, line);
    }
}

// ------------------------------------------------------------------ UART

static void uart_service(void) {
    if (s_uoff == s_ulen) {
        critical_section_enter_blocking(&s_lock);
        s_ulen = logbuf_read(&s_keep.log, &s_uart_pos, s_upend, sizeof s_upend, 0, NULL);
        critical_section_exit(&s_lock);
        s_uoff = 0;
    }
    while (s_uoff < s_ulen && uart_is_writable(uart0)) uart_putc_raw(uart0, s_upend[s_uoff++]);
}

// --------------------------------------------------------------- network

// Called by our copy of dhcp.c after it declined an address (see
// cmake/iolibrary.cmake).
void pshb_dhcp_decline_wait(void) {
    sleep_ms(1000);
}

static void on_dhcp_assign(void) {
    wiz_NetInfo ni;
    wizchip_getnetinfo(&ni);
    getIPfromDHCP(ni.ip);
    getGWfromDHCP(ni.gw);
    getSNfromDHCP(ni.sn);
    getDNSfromDHCP(ni.dns);
    ni.dhcp = NETINFO_DHCP;
    wizchip_setnetinfo(&ni);
    s_dhcp_assigned = true;
}

static void on_dhcp_conflict(void) {
    s_dhcp_conflict = true;
}

static void open_socket(void) {
    if (s_sock_open) return;
    if (socket(NETLOG_SOCK, Sn_MR_UDP, NETLOG_PORT, 0) != NETLOG_SOCK) {
        printf("net: cannot open the log socket\n");
        return;
    }
    s_sock_open = true;
    wiz_NetInfo ni;
    wizchip_getnetinfo(&ni);
    printf("net: log on udp %u.%u.%u.%u:%u (tools/boardlog.py %u.%u.%u.%u)\n", ni.ip[0], ni.ip[1], ni.ip[2],
           ni.ip[3], NETLOG_PORT, ni.ip[0], ni.ip[1], ni.ip[2], ni.ip[3]);
}

static void close_socket(void) {
    if (!s_sock_open) return;
    close(NETLOG_SOCK);
    s_sock_open = false;
}

static void start_addressing(void) {
    if (s_cfg.ip_mode == PSHB_IP_STATIC) {
        wiz_NetInfo ni;
        wizchip_getnetinfo(&ni);
        memcpy(ni.ip, s_cfg.ip, 4);
        memcpy(ni.sn, s_cfg.mask, 4);
        memcpy(ni.gw, s_cfg.gw, 4);
        memcpy(ni.dns, s_cfg.dns, 4);
        ni.dhcp = NETINFO_STATIC;
        wizchip_setnetinfo(&ni);
        printf("net: static address\n");
        open_socket();
        s_net = N_READY;
    } else {
        printf("net: starting DHCP\n");
        DHCP_init(W5500_SOCK_DHCP, s_dhcp_buf);
        s_net = N_DHCP;
    }
}

void netlog_start_network(const pshb_settings_t *s, bool chip_ok) {
    if (!chip_ok) {
        printf("net: the W5500 does not answer -- log on UART0 only\n");
        return;
    }
    s_cfg = *s;
    reg_dhcp_cbfunc(on_dhcp_assign, on_dhcp_assign, on_dhcp_conflict);
    s_net = N_NO_LINK;
}

static void handle_datagram(const uint8_t ip[4], uint16_t port, char *text) {
    // Trim.
    char *end = text + strlen(text);
    while (end > text && (end[-1] == '\n' || end[-1] == '\r' || end[-1] == ' ')) *--end = '\0';
    while (*text == ' ') text++;
    bool new_sub = !s_sub || memcmp(ip, s_sub_ip, 4) != 0 || port != s_sub_port;
    memcpy(s_sub_ip, ip, 4);
    s_sub_port = port;
    s_sub_seen = time_us_64();
    if (strcmp(text, "sub") == 0 || new_sub) {
        critical_section_enter_blocking(&s_lock);
        uint64_t tail = logbuf_tail(&s_keep.log);
        // "sub": everything still kept; anything else from a new receiver
        // (a "ka" after a restart): this boot's log.
        s_net_pos = strcmp(text, "sub") == 0 || s_boot_pos < tail ? tail : s_boot_pos;
        critical_section_exit(&s_lock);
        if (!s_sub || new_sub)
            printf("net: log goes to %u.%u.%u.%u:%u\n", ip[0], ip[1], ip[2], ip[3], port);
        s_sub = true;
    }
    if (strcmp(text, "sub") == 0 || strcmp(text, "ka") == 0 || *text == '\0') return;
    printf("cmd: %s\n", text);
    if (s_cmd) s_cmd(text);
}

static void receive(void) {
    if (getSn_RX_RSR(NETLOG_SOCK) == 0) return;
    uint8_t buf[128], ip[4];
    uint16_t port;
    int32_t n = recvfrom(NETLOG_SOCK, buf, sizeof buf - 1, ip, &port);
    if (n <= 0) return;
    buf[n] = '\0';
    handle_datagram(ip, port, (char *)buf);
}

static void send_log(void) {
    if (!s_sub) return;
    if (time_us_64() - s_sub_seen > SUB_TIMEOUT_US) {
        s_sub = false;  // the receiver is gone
        return;
    }
    for (int i = 0; i < SENDS_PER_ROUND; i++) {
        static char chunk[SEND_CHUNK];
        uint64_t lost;
        critical_section_enter_blocking(&s_lock);
        uint32_t n = logbuf_read(&s_keep.log, &s_net_pos, chunk, sizeof chunk, 1, &lost);
        critical_section_exit(&s_lock);
        if (lost) {
            char note[64];
            int k = snprintf(note, sizeof note, "[... %llu bytes of the log lost ...]\n", (unsigned long long)lost);
            sendto(NETLOG_SOCK, (uint8_t *)note, (uint16_t)k, s_sub_ip, s_sub_port);
        }
        if (n == 0) return;
        if (sendto(NETLOG_SOCK, (uint8_t *)chunk, (uint16_t)n, s_sub_ip, s_sub_port) < 0) {
            s_sub = false;  // e.g. no ARP answer: wait for the next "ka"
            return;
        }
    }
}

static void net_service(void) {
    if (s_net == N_OFF) return;
    uint64_t now = time_us_64();
    if (now >= s_next_link) {
        s_next_link = now + LINK_POLL_US;
        bool link = wizphy_getphylink() == PHY_LINK_ON;
        if (!link && s_net != N_NO_LINK) {
            printf("net: link down\n");
            if (s_cfg.ip_mode == PSHB_IP_DHCP) DHCP_stop();
            close_socket();
            s_net = N_NO_LINK;
        } else if (link && s_net == N_NO_LINK) {
            printf("net: link up\n");
            start_addressing();
        }
    }
    if (s_cfg.ip_mode == PSHB_IP_DHCP && s_net != N_NO_LINK) {
        if (now >= s_next_tick) {
            s_next_tick = now + 1000000u;
            DHCP_time_handler();
        }
        if (s_net == N_DHCP_WAIT && now >= s_dhcp_retry) {
            DHCP_init(W5500_SOCK_DHCP, s_dhcp_buf);
            s_net = N_DHCP;
        }
        if ((s_net == N_DHCP || s_net == N_READY) && now >= s_next_dhcp) {
            s_next_dhcp = now + DHCP_POLL_US;
            uint8_t r = DHCP_run();
            if (s_dhcp_assigned) {
                s_dhcp_assigned = false;
                printf("net: DHCP lease\n");
                open_socket();
                s_net = N_READY;
            }
            if (s_dhcp_conflict || r == DHCP_FAILED) {
                printf("net: DHCP %s, retrying\n", s_dhcp_conflict ? "conflict" : "failed");
                s_dhcp_conflict = false;
                DHCP_stop();
                close_socket();
                s_dhcp_retry = now + DHCP_RETRY_US;
                s_net = N_DHCP_WAIT;
            }
        }
    }
    if (s_sock_open) {
        receive();
        send_log();
    }
}

void netlog_service(void) {
    uart_service();
    net_service();
}
