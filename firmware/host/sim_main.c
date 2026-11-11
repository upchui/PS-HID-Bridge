// Board simulator: runs the portable firmware core (bridge, input state,
// settings, web interface) on Linux and prints the HID reports the board
// would send to the PlayStation. Used by the integration tests and for manual
// testing without hardware.
//
// Output lines (stdout, line-buffered):
//   READY <udp port> HTTP <http port or 0>
//   KBD mods=02 keys=04 1d 00 00 00 00
//   MOUSE buttons=01 dx=15 dy=-3 wheel=0 pan=0
//   FAILSAFE
//   REBOOT
#define _POSIX_C_SOURCE 200809L
#define _DEFAULT_SOURCE

#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <getopt.h>
#include <netinet/in.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <time.h>
#include <unistd.h>

#include "bridge.h"
#include "hexutil.h"
#include "input_state.h"
#include "protocol.h"
#include "settings.h"
#include "uf2_writer.h"
#include "web.h"

#define FLASH_SIZE (2 * PSHB_SETTINGS_SECTOR)
#define MAX_CONNS 2
#define CONN_TIMEOUT_US 10000000ull

static volatile sig_atomic_t s_stop;

static void on_signal(int sig) {
    (void)sig;
    s_stop = 1;
}

static uint64_t now_us(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000u + (uint64_t)ts.tv_nsec / 1000u;
}

static void random_bytes(uint8_t *buf, size_t n) {
    FILE *f = fopen("/dev/urandom", "rb");
    if (!f || fread(buf, 1, n, f) != n) {
        fprintf(stderr, "sim: cannot read /dev/urandom\n");
        exit(1);
    }
    fclose(f);
}

static uint64_t random_u64(void) {
    uint64_t v;
    random_bytes((uint8_t *)&v, sizeof(v));
    return v;
}

// ---------------------------------------------------------------- simulated board

typedef struct {
    // options
    bool plaintext, usb_mounted, have_session, have_key_override;
    uint64_t session;
    uint8_t key_override[32];
    bool have_allowed_override;
    uint32_t allowed_net, allowed_mask;
    unsigned failsafe_ms;
    const char *settings_file;

    // state
    uint8_t flash[FLASH_SIZE];
    pshb_settings_t settings;
    input_state_t input;
    bridge_t bridge;
    uint64_t boot_us;
    bool reboot_requested;
} sim_t;

static sim_t S;

static void flash_read(void *ctx, uint32_t off, void *buf, size_t len) {
    (void)ctx;
    memcpy(buf, S.flash + off, len);
}

static void flash_persist(void) {
    if (!S.settings_file) return;
    FILE *f = fopen(S.settings_file, "wb");
    if (f) {
        fwrite(S.flash, 1, sizeof(S.flash), f);
        fclose(f);
    }
}

static bool flash_erase(void *ctx, uint32_t off) {
    (void)ctx;
    memset(S.flash + off, 0xFF, PSHB_SETTINGS_SECTOR);
    flash_persist();
    return true;
}

static bool flash_program(void *ctx, uint32_t off, const void *data, size_t len) {
    (void)ctx;
    // NOR flash semantics: programming can only clear bits.
    const uint8_t *d = data;
    for (size_t i = 0; i < len; i++) S.flash[off + i] &= d[i];
    flash_persist();
    return true;
}

static const pshb_flash_t k_flash = {NULL, flash_read, flash_erase, flash_program};

static void apply_settings(const pshb_settings_t *s) {
    if (S.have_key_override) {
        bridge_set_key(&S.bridge, S.key_override);
    } else {
        bridge_set_key(&S.bridge, s->psk_set ? s->psk : NULL);
    }
    if (S.have_allowed_override) {
        S.bridge.allowed_net = S.allowed_net;
        S.bridge.allowed_mask = S.allowed_mask;
    } else {
        settings_sender_filter(s, &S.bridge.allowed_net, &S.bridge.allowed_mask);
    }
}

// Power-on: load settings, new session, empty input state.
static void boot(void) {
    settings_load(&k_flash, &S.settings);
    if (S.have_key_override) {
        // As if this key had been set in the web interface (not saved),
        // so key-based features like signed firmware updates work too.
        memcpy(S.settings.psk, S.key_override, sizeof(S.settings.psk));
        S.settings.psk_set = 1;
    }
    S.boot_us = now_us();
    input_state_init(&S.input);
    uint8_t kbd[PSHB_KBD_REPORT_LEN], mouse[PSHB_MOUSE_REPORT_LEN];
    input_state_take_kbd_report(&S.input, kbd);  // host already saw the empty state
    input_state_take_mouse_report(&S.input, mouse, false);
    bridge_init(&S.bridge, &S.input, S.have_session ? S.session : random_u64(), random_u64(), S.boot_us);
    S.bridge.allow_plaintext = S.plaintext;
    S.bridge.failsafe_us = S.failsafe_ms * 1000u;
    S.bridge.usb_status = S.usb_mounted ? PSHB_STATUS_USB_MOUNTED : 0;
    apply_settings(&S.settings);
}

// ---------------------------------------------------------------- web backend

static void be_status(void *ctx, web_status_t *out) {
    (void)ctx;
    out->fw_version = S.bridge.fw_version;
    out->uptime_ms = (uint32_t)((now_us() - S.boot_us) / 1000);
    static const uint8_t mac[6] = {0x02, 0x00, 0x00, 0x51, 0x4D, 0x00};
    memcpy(out->mac, mac, 6);
    static const uint8_t ip[4] = {127, 0, 0, 1}, mask[4] = {255, 0, 0, 0};
    memcpy(out->ip, S.settings.ip_mode == PSHB_IP_STATIC ? S.settings.ip : ip, 4);
    memcpy(out->mask, S.settings.ip_mode == PSHB_IP_STATIC ? S.settings.mask : mask, 4);
    memcpy(out->gw, S.settings.gw, 4);
    memcpy(out->dns, S.settings.dns, 4);
    out->link_up = true;
    out->net_state = "Simulator";
    out->usb_status = S.bridge.usb_status;
    out->failsafe = S.bridge.failsafe_active;
    out->rx_ok = S.bridge.stats.rx_ok;
    out->rx_auth_fail = S.bridge.stats.rx_auth_fail;
    out->rx_replay = S.bridge.stats.rx_replay;
    out->rx_filtered = S.bridge.stats.rx_filtered;
    out->rx_malformed = S.bridge.stats.rx_malformed;
    out->partition = "-";
}

static bool be_save(void *ctx, const pshb_settings_t *s) {
    (void)ctx;
    pshb_settings_t copy = *s;
    return settings_save(&k_flash, &copy) && (S.settings = copy, true);
}

static void be_apply(void *ctx, const pshb_settings_t *s) {
    (void)ctx;
    apply_settings(s);
}

static void be_reboot(void *ctx) {
    (void)ctx;
    S.reboot_requested = true;
}

static void be_factory_reset(void *ctx) {
    (void)ctx;
    settings_erase(&k_flash);
    S.reboot_requested = true;
}

static void be_random(void *ctx, uint8_t *buf, size_t n) {
    (void)ctx;
    random_bytes(buf, n);
}

static uint64_t be_now(void *ctx) {
    (void)ctx;
    return now_us();
}

// Firmware update into a RAM "slot" (same writer as on the board).
#define SIM_SLOT_SIZE (1000u * 1024u)
static uint8_t s_slot[SIM_SLOT_SIZE];
static uf2_writer_t s_uf2;
static bool s_ota_running;

static bool slot_erase(void *ctx, uint32_t off) {
    (void)ctx;
    memset(s_slot + off, 0xFF, UF2_SECTOR_SIZE);
    return true;
}

static bool slot_program(void *ctx, uint32_t off, const uint8_t data[UF2_PAYLOAD_SIZE]) {
    (void)ctx;
    memcpy(s_slot + off, data, UF2_PAYLOAD_SIZE);
    return true;
}

static const uf2_flash_t k_slot = {NULL, slot_erase, slot_program};

static bool be_ota_begin(void *ctx, uint32_t size, char *err, size_t cap) {
    (void)ctx;
    if (size == 0 || size % UF2_BLOCK_SIZE != 0) {
        snprintf(err, cap, "This is not a UF2 file.");
        return false;
    }
    uf2_writer_init(&s_uf2, &k_slot, SIM_SLOT_SIZE, UF2_FAMILY_RP2350_ARM_S);
    s_ota_running = true;
    printf("OTA BEGIN %u\n", (unsigned)size);
    return true;
}

static bool be_ota_write(void *ctx, const uint8_t *data, size_t len) {
    (void)ctx;
    return uf2_writer_feed(&s_uf2, data, len);
}

static bool be_ota_finish(void *ctx, uint32_t crc, char *err, size_t cap) {
    (void)ctx;
    uf2_result_t r = uf2_writer_finish(&s_uf2, crc);
    if (r != UF2_OK) {
        snprintf(err, cap, "%s", uf2_error_text(r));
        return false;
    }
    printf("OTA OK %u\n", (unsigned)s_uf2.blocks_written);
    return true;
}

static void be_ota_abort(void *ctx) {
    (void)ctx;
    if (s_ota_running) printf("OTA ABORT\n");
    s_ota_running = false;
}

static void be_ota_activate(void *ctx) {
    (void)ctx;
    s_ota_running = false;
    S.reboot_requested = true;
}

static const web_backend_t k_backend = {
    .ctx = NULL,
    .status = be_status,
    .save = be_save,
    .apply = be_apply,
    .reboot = be_reboot,
    .factory_reset = be_factory_reset,
    .random = be_random,
    .now_us = be_now,
    .ota_begin = be_ota_begin,
    .ota_write = be_ota_write,
    .ota_finish = be_ota_finish,
    .ota_abort = be_ota_abort,
    .ota_activate = be_ota_activate,
};

// ---------------------------------------------------------------- main loop

typedef struct {
    int fd;
    web_conn_t conn;
} http_slot_t;

static void usage(const char *argv0) {
    fprintf(stderr,
            "usage: %s [options]\n"
            "  --bind ADDR       listen address (default 127.0.0.1)\n"
            "  --port N          UDP port (default %d, 0 = any free port)\n"
            "  --http-port N     web interface port (default: off, 0 = any free port)\n"
            "  --settings FILE   keep the simulated settings flash in FILE\n"
            "  --plaintext       accept unencrypted debug packets (version 0)\n"
            "  --psk HEX         pre-shared key (overrides the settings)\n"
            "  --psk-file PATH   read the pre-shared key from a file\n"
            "  --failsafe-ms N   failsafe timeout (default 250)\n"
            "  --session HEX     fixed session id (default: random per boot)\n"
            "  --allow IP[/N]    only accept packets from this sender or subnet\n"
            "  --usb-unmounted   simulate a board that is not plugged in\n",
            argv0, PSHB_UDP_PORT_DEFAULT);
}

static void usb_frame(void) {
    if (input_state_kbd_pending(&S.input)) {
        uint8_t r[PSHB_KBD_REPORT_LEN];
        input_state_take_kbd_report(&S.input, r);
        printf("KBD mods=%02x keys=%02x %02x %02x %02x %02x %02x\n", r[0], r[2], r[3], r[4], r[5], r[6], r[7]);
    }
    if (input_state_mouse_pending(&S.input)) {
        uint8_t r[PSHB_MOUSE_REPORT_LEN];
        input_state_take_mouse_report(&S.input, r, false);
        printf("MOUSE buttons=%02x dx=%d dy=%d wheel=%d pan=%d\n", r[0], (int16_t)pshb_rd16(&r[1]),
               (int16_t)pshb_rd16(&r[3]), (int8_t)r[5], (int8_t)r[6]);
    }
}

static int bind_socket(int type, const char *addr_s, int port, int *bound_port) {
    int fd = socket(AF_INET, type, 0);
    int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    struct sockaddr_in addr = {.sin_family = AF_INET, .sin_port = htons((uint16_t)port)};
    if (fd < 0 || inet_pton(AF_INET, addr_s, &addr.sin_addr) != 1 ||
        bind(fd, (struct sockaddr *)&addr, sizeof(addr)) != 0) {
        perror("sim: bind");
        exit(1);
    }
    socklen_t alen = sizeof(addr);
    getsockname(fd, (struct sockaddr *)&addr, &alen);
    *bound_port = ntohs(addr.sin_port);
    fcntl(fd, F_SETFL, O_NONBLOCK);
    return fd;
}

static void http_service(web_app_t *app, http_slot_t *slots, int listen_fd, uint64_t t) {
    for (int i = 0; i < MAX_CONNS; i++) {
        if (slots[i].fd < 0) {
            int fd = accept(listen_fd, NULL, NULL);
            if (fd < 0) break;
            fcntl(fd, F_SETFL, O_NONBLOCK);
            slots[i].fd = fd;
            web_conn_reset(&slots[i].conn, t);
        }
    }
    for (int i = 0; i < MAX_CONNS; i++) {
        http_slot_t *s = &slots[i];
        if (s->fd < 0) continue;
        uint8_t buf[1024];
        ssize_t n = recv(s->fd, buf, sizeof(buf), 0);
        if (n > 0) {
            web_conn_input(app, &s->conn, buf, (size_t)n);
        } else if (n == 0 && !s->conn.responding) {
            web_conn_close(app, &s->conn);
            close(s->fd);
            s->fd = -1;
            continue;
        }
        const uint8_t *out;
        size_t len;
        while ((len = web_conn_output(&s->conn, &out)) > 0) {
            if (len > 1024) len = 1024;  // same chunking as the board
            ssize_t w = send(s->fd, out, len, MSG_NOSIGNAL);
            if (w <= 0) break;
            web_conn_sent(app, &s->conn, (size_t)w);
        }
        if (web_conn_done(&s->conn) || t - s->conn.started_us > CONN_TIMEOUT_US) {
            web_conn_close(app, &s->conn);
            close(s->fd);
            s->fd = -1;
        }
    }
}

int main(int argc, char **argv) {
    const char *bind_addr = "127.0.0.1";
    int port = PSHB_UDP_PORT_DEFAULT;
    int http_port = -1;
    S.usb_mounted = true;
    S.failsafe_ms = PSHB_FAILSAFE_US_DEFAULT / 1000;

    static const struct option opts[] = {
        {"bind", required_argument, 0, 'b'},
        {"port", required_argument, 0, 'p'},
        {"http-port", required_argument, 0, 'w'},
        {"settings", required_argument, 0, 'S'},
        {"plaintext", no_argument, 0, 'P'},
        {"psk", required_argument, 0, 'k'},
        {"psk-file", required_argument, 0, 'K'},
        {"failsafe-ms", required_argument, 0, 'f'},
        {"session", required_argument, 0, 's'},
        {"allow", required_argument, 0, 'a'},
        {"usb-unmounted", no_argument, 0, 'u'},
        {"help", no_argument, 0, 'h'},
        {0, 0, 0, 0},
    };
    int c;
    while ((c = getopt_long(argc, argv, "", opts, NULL)) != -1) {
        switch (c) {
        case 'b': bind_addr = optarg; break;
        case 'p': port = atoi(optarg); break;
        case 'w': http_port = atoi(optarg); break;
        case 'S': S.settings_file = optarg; break;
        case 'P': S.plaintext = true; break;
        case 'k':
            if (!hex_decode(optarg, S.key_override, 32)) {
                fprintf(stderr, "sim: --psk needs 64 hex digits\n");
                return 2;
            }
            S.have_key_override = true;
            break;
        case 'K': {
            char buf[80] = {0};
            FILE *f = fopen(optarg, "r");
            if (!f || !fgets(buf, sizeof(buf), f)) {
                fprintf(stderr, "sim: cannot read %s\n", optarg);
                return 2;
            }
            fclose(f);
            buf[strcspn(buf, "\r\n")] = '\0';
            if (!hex_decode(buf, S.key_override, 32)) {
                fprintf(stderr, "sim: %s does not contain 64 hex digits\n", optarg);
                return 2;
            }
            S.have_key_override = true;
            break;
        }
        case 'f': S.failsafe_ms = (unsigned)atoi(optarg); break;
        case 's':
            S.session = strtoull(optarg, NULL, 16);
            S.have_session = true;
            break;
        case 'a': {
            uint8_t ip[4], prefix;
            if (!web_parse_cidr(optarg, ip, &prefix)) {
                fprintf(stderr, "sim: bad IP or subnet %s\n", optarg);
                return 2;
            }
            S.allowed_mask = settings_prefix_mask(prefix);
            S.allowed_net = (((uint32_t)ip[0] << 24) | ((uint32_t)ip[1] << 16) | ((uint32_t)ip[2] << 8) | ip[3]) &
                            S.allowed_mask;
            S.have_allowed_override = true;
            break;
        }
        case 'u': S.usb_mounted = false; break;
        default: usage(argv[0]); return c == 'h' ? 0 : 2;
        }
    }

    setvbuf(stdout, NULL, _IOLBF, 0);
    signal(SIGINT, on_signal);
    signal(SIGTERM, on_signal);
    signal(SIGPIPE, SIG_IGN);

    memset(S.flash, 0xFF, sizeof(S.flash));
    if (S.settings_file) {
        FILE *f = fopen(S.settings_file, "rb");
        if (f) {
            if (fread(S.flash, 1, sizeof(S.flash), f) != sizeof(S.flash)) memset(S.flash, 0xFF, sizeof(S.flash));
            fclose(f);
        }
    }
    boot();

    int udp_port = 0, bound_http = 0;
    int udp = bind_socket(SOCK_DGRAM, bind_addr, port, &udp_port);
    int listen_fd = -1;
    if (http_port >= 0) {
        listen_fd = bind_socket(SOCK_STREAM, bind_addr, http_port, &bound_http);
        listen(listen_fd, 4);
    }

    web_app_t app;
    web_app_init(&app, &k_backend, &S.settings);
    http_slot_t slots[MAX_CONNS];
    for (int i = 0; i < MAX_CONNS; i++) slots[i].fd = -1;

    printf("READY %d HTTP %d\n", udp_port, bound_http);

    uint64_t next_frame = now_us() + 1000;
    while (!s_stop) {
        uint64_t t = now_us();
        int timeout_ms = next_frame > t ? (int)((next_frame - t + 999) / 1000) : 0;
        struct pollfd pfd[2 + MAX_CONNS];
        int nfds = 0;
        pfd[nfds++] = (struct pollfd){.fd = udp, .events = POLLIN};
        if (listen_fd >= 0) pfd[nfds++] = (struct pollfd){.fd = listen_fd, .events = POLLIN};
        for (int i = 0; i < MAX_CONNS; i++) {
            if (slots[i].fd >= 0) pfd[nfds++] = (struct pollfd){.fd = slots[i].fd, .events = POLLIN | POLLOUT};
        }
        if (poll(pfd, (nfds_t)nfds, timeout_ms) < 0 && errno != EINTR) {
            perror("sim: poll");
            break;
        }

        for (;;) {
            uint8_t pkt[2048], reply[PSHB_MAX_PACKET_LEN];
            struct sockaddr_in src;
            socklen_t slen = sizeof(src);
            ssize_t n = recvfrom(udp, pkt, sizeof(pkt), 0, (struct sockaddr *)&src, &slen);
            if (n < 0) break;
            size_t rlen = bridge_handle_packet(&S.bridge, pkt, (size_t)n, ntohl(src.sin_addr.s_addr), now_us(),
                                               reply, sizeof(reply));
            if (rlen) sendto(udp, reply, rlen, 0, (struct sockaddr *)&src, slen);
        }
        if (listen_fd >= 0) http_service(&app, slots, listen_fd, now_us());

        if (S.reboot_requested) {
            S.reboot_requested = false;
            printf("REBOOT\n");
            boot();
            web_app_init(&app, &k_backend, &S.settings);
        }

        t = now_us();
        bool was_failsafe = S.bridge.failsafe_active;
        bridge_tick(&S.bridge, t);
        if (S.bridge.failsafe_active && !was_failsafe) printf("FAILSAFE\n");
        if (t >= next_frame) {
            next_frame = t + 1000;
            if (S.usb_mounted) {
                usb_frame();
            } else {
                input_state_drop_motion(&S.input);
            }
        }
    }
    close(udp);
    if (listen_fd >= 0) close(listen_fd);
    return 0;
}
