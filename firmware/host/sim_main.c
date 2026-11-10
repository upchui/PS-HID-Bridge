// Board simulator: runs the portable firmware core (bridge, input state) on
// Linux behind a UDP socket and prints the HID reports the board would send
// to the PS5. Used by the integration tests and for manual testing without
// hardware.
//
// Output lines (stdout, line-buffered):
//   READY <port>
//   KBD mods=02 keys=04 1d 00 00 00 00
//   MOUSE buttons=01 dx=15 dy=-3 wheel=0 pan=0
#define _POSIX_C_SOURCE 200809L

#include <arpa/inet.h>
#include <errno.h>
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
#include "input_state.h"
#include "protocol.h"

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

static uint64_t random_u64(void) {
    uint64_t v = 0;
    FILE *f = fopen("/dev/urandom", "rb");
    if (!f || fread(&v, sizeof(v), 1, f) != 1) {
        fprintf(stderr, "sim: cannot read /dev/urandom\n");
        exit(1);
    }
    fclose(f);
    return v;
}

static void usage(const char *argv0) {
    fprintf(stderr,
            "usage: %s [options]\n"
            "  --bind ADDR       listen address (default 127.0.0.1)\n"
            "  --port N          UDP port (default %d, 0 = any free port)\n"
            "  --plaintext       accept unencrypted debug packets (version 0)\n"
            "  --session HEX     fixed session id (default: random)\n"
            "  --allow IP        only accept packets from this sender\n"
            "  --usb-unmounted   simulate a board that is not plugged in\n",
            argv0, PSHB_UDP_PORT_DEFAULT);
}

// One simulated USB frame: at most one report per endpoint, like the real
// 1 ms interrupt endpoints.
static void usb_frame(input_state_t *in) {
    if (input_state_kbd_pending(in)) {
        uint8_t r[PSHB_KBD_REPORT_LEN];
        input_state_take_kbd_report(in, r);
        printf("KBD mods=%02x keys=%02x %02x %02x %02x %02x %02x\n", r[0], r[2], r[3], r[4], r[5],
               r[6], r[7]);
    }
    if (input_state_mouse_pending(in)) {
        uint8_t r[PSHB_MOUSE_REPORT_LEN];
        input_state_take_mouse_report(in, r, false);
        printf("MOUSE buttons=%02x dx=%d dy=%d wheel=%d pan=%d\n", r[0],
               (int16_t)pshb_rd16(&r[1]), (int16_t)pshb_rd16(&r[3]), (int8_t)r[5], (int8_t)r[6]);
    }
}

int main(int argc, char **argv) {
    const char *bind_addr = "127.0.0.1";
    int port = PSHB_UDP_PORT_DEFAULT;
    bool plaintext = false, usb_mounted = true;
    bool have_session = false;
    uint64_t session = 0;
    uint32_t allowed_ip = 0;

    static const struct option opts[] = {
        {"bind", required_argument, 0, 'b'},
        {"port", required_argument, 0, 'p'},
        {"plaintext", no_argument, 0, 'P'},
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
        case 'P': plaintext = true; break;
        case 's':
            session = strtoull(optarg, NULL, 16);
            have_session = true;
            break;
        case 'a': {
            struct in_addr a;
            if (inet_pton(AF_INET, optarg, &a) != 1) {
                fprintf(stderr, "sim: bad IP %s\n", optarg);
                return 2;
            }
            allowed_ip = ntohl(a.s_addr);
            break;
        }
        case 'u': usb_mounted = false; break;
        default: usage(argv[0]); return c == 'h' ? 0 : 2;
        }
    }

    setvbuf(stdout, NULL, _IOLBF, 0);
    signal(SIGINT, on_signal);
    signal(SIGTERM, on_signal);

    int fd = socket(AF_INET, SOCK_DGRAM, 0);
    struct sockaddr_in addr = {.sin_family = AF_INET, .sin_port = htons((uint16_t)port)};
    if (fd < 0 || inet_pton(AF_INET, bind_addr, &addr.sin_addr) != 1 ||
        bind(fd, (struct sockaddr *)&addr, sizeof(addr)) != 0) {
        perror("sim: bind");
        return 1;
    }
    socklen_t alen = sizeof(addr);
    getsockname(fd, (struct sockaddr *)&addr, &alen);

    input_state_t input;
    bridge_t bridge;
    input_state_init(&input);
    bridge_init(&bridge, &input, have_session ? session : random_u64(), now_us());
    bridge.allow_plaintext = plaintext;
    bridge.allowed_ip = allowed_ip;
    bridge.usb_status = usb_mounted ? PSHB_STATUS_USB_MOUNTED : 0;

    // The host has just "configured" the device: drop the initial empty reports.
    {
        uint8_t kbd[PSHB_KBD_REPORT_LEN], mouse[PSHB_MOUSE_REPORT_LEN];
        input_state_take_kbd_report(&input, kbd);
        input_state_take_mouse_report(&input, mouse, false);
    }

    printf("READY %d\n", ntohs(addr.sin_port));

    uint64_t next_frame = now_us() + 1000;
    while (!s_stop) {
        uint64_t t = now_us();
        int timeout_ms = next_frame > t ? (int)((next_frame - t + 999) / 1000) : 0;
        struct pollfd pfd = {.fd = fd, .events = POLLIN};
        int pr = poll(&pfd, 1, timeout_ms);
        if (pr < 0 && errno != EINTR) {
            perror("sim: poll");
            break;
        }
        if (pr > 0 && (pfd.revents & POLLIN)) {
            uint8_t pkt[2048], reply[PSHB_MAX_PACKET_LEN];
            struct sockaddr_in src;
            socklen_t slen = sizeof(src);
            ssize_t n = recvfrom(fd, pkt, sizeof(pkt), 0, (struct sockaddr *)&src, &slen);
            if (n >= 0) {
                size_t rlen = bridge_handle_packet(&bridge, pkt, (size_t)n, ntohl(src.sin_addr.s_addr),
                                                   now_us(), reply, sizeof(reply));
                if (rlen) sendto(fd, reply, rlen, 0, (struct sockaddr *)&src, slen);
            }
        }
        t = now_us();
        if (t >= next_frame) {
            next_frame = t + 1000;
            if (usb_mounted) {
                usb_frame(&input);
            } else {
                input_state_drop_motion(&input);
            }
        }
    }
    close(fd);
    return 0;
}
