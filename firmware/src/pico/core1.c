#include "core1.h"

#include <stdio.h>
#include <string.h>

#include "hardware/watchdog.h"
#include "pico/rand.h"
#include "pico/stdlib.h"

#include "bootsel.h"
#include "dhcp.h"
#include "flash_store.h"
#include "net_shared.h"
#include "protocol.h"
#include "pshb_config.h"
#include "socket.h"
#include "w5500_port.h"
#include "web.h"
#include "web_server.h"
#include "wizchip_conf.h"

#define LINK_POLL_MS        100
#define DHCP_RETRY_MS       5000
#define BOOTSEL_POLL_MS     100
#define FACTORY_RESET_HOLD_MS 5000

typedef enum { C1_NO_LINK, C1_DHCP, C1_DHCP_WAIT, C1_READY } c1_state_t;

static uint8_t s_dhcp_buf[548];  // RIP_MSG_SIZE
static volatile bool s_dhcp_assigned;
static volatile bool s_dhcp_conflict;
static bool s_udp_open;
static c1_state_t s_state = C1_NO_LINK;
static bool s_reboot;

// ---------------------------------------------------------------- network

// Our own handlers: the ioLibrary default for "IP changed" soft-resets the
// whole chip, which would kill the input socket.
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

static void set_state(c1_state_t st) {
    static const net_state_t map[] = {
        [C1_NO_LINK] = NET_STATE_NO_LINK,
        [C1_DHCP] = NET_STATE_DHCP,
        [C1_DHCP_WAIT] = NET_STATE_DHCP,
        [C1_READY] = NET_STATE_READY,
    };
    s_state = st;
    atomic_store(&g_net.state, map[st]);
}

static void udp_open(uint16_t port) {
    if (socket(W5500_SOCK_UDP, Sn_MR_UDP, port, SF_BROAD_BLOCK | SF_IO_NONBLOCK) != W5500_SOCK_UDP) {
        printf("net: cannot open UDP socket\n");
        return;
    }
    setSn_IR(W5500_SOCK_UDP, 0xFF);
    setSn_IMR(W5500_SOCK_UDP, Sn_IR_RECV);
    setSIMR(1u << W5500_SOCK_UDP);
    atomic_store(&g_net.udp_port, port);
    atomic_store(&g_net.udp_enable, true);
    s_udp_open = true;
}

static void publish_ip(const char *what) {
    wiz_NetInfo ni;
    wizchip_getnetinfo(&ni);
    printf("net: %s %u.%u.%u.%u/%u.%u.%u.%u gw %u.%u.%u.%u\n", what, ni.ip[0], ni.ip[1], ni.ip[2], ni.ip[3],
           ni.sn[0], ni.sn[1], ni.sn[2], ni.sn[3], ni.gw[0], ni.gw[1], ni.gw[2], ni.gw[3]);
    atomic_store(&g_net.ip, ip_to_u32(ni.ip));
}

static void start_addressing(void) {
    if (g_settings.ip_mode == PSHB_IP_STATIC) {
        wiz_NetInfo ni;
        wizchip_getnetinfo(&ni);
        memcpy(ni.ip, g_settings.ip, 4);
        memcpy(ni.sn, g_settings.mask, 4);
        memcpy(ni.gw, g_settings.gw, 4);
        memcpy(ni.dns, g_settings.dns, 4);
        ni.dhcp = NETINFO_STATIC;
        wizchip_setnetinfo(&ni);
        publish_ip("static");
        if (!s_udp_open) udp_open(g_settings.udp_port);
        set_state(C1_READY);
    } else {
        printf("net: starting DHCP\n");
        DHCP_init(W5500_SOCK_DHCP, s_dhcp_buf);
        set_state(C1_DHCP);
    }
}

static void network_service(void) {
    static absolute_time_t next_link, dhcp_retry_at;
    if (time_reached(next_link)) {
        next_link = make_timeout_time_ms(LINK_POLL_MS);
        bool link = wizphy_getphylink() == PHY_LINK_ON;
        if (!link && s_state != C1_NO_LINK) {
            printf("net: link down\n");
            if (s_state != C1_READY || g_settings.ip_mode == PSHB_IP_DHCP) DHCP_stop();
            atomic_store(&g_net.ip, 0);
            set_state(C1_NO_LINK);
        } else if (link && s_state == C1_NO_LINK) {
            printf("net: link up\n");
            start_addressing();
        }
    }

    if (s_state == C1_DHCP_WAIT && time_reached(dhcp_retry_at)) {
        DHCP_init(W5500_SOCK_DHCP, s_dhcp_buf);
        set_state(C1_DHCP);
    }

    if ((s_state == C1_DHCP || s_state == C1_READY) && g_settings.ip_mode == PSHB_IP_DHCP) {
        uint8_t r = DHCP_run();
        if (s_dhcp_assigned) {
            s_dhcp_assigned = false;
            publish_ip("DHCP lease");
            if (!s_udp_open) udp_open(g_settings.udp_port);
            set_state(C1_READY);
        }
        if (s_dhcp_conflict || r == DHCP_FAILED) {
            printf("net: DHCP %s, retrying\n", s_dhcp_conflict ? "conflict" : "failed");
            s_dhcp_conflict = false;
            DHCP_stop();
            atomic_store(&g_net.ip, 0);
            dhcp_retry_at = make_timeout_time_ms(DHCP_RETRY_MS);
            set_state(C1_DHCP_WAIT);
        }
    }
}

// ---------------------------------------------------------------- web backend

static const char *net_state_text(void) {
    switch (atomic_load(&g_net.state)) {
    case NET_STATE_CHIP_ERROR: return "Netzwerkchip antwortet nicht";
    case NET_STATE_NO_LINK: return "Kein Netzwerkkabel";
    case NET_STATE_DHCP: return "DHCP läuft";
    case NET_STATE_READY: return "Bereit";
    default: return "Start";
    }
}

static void be_status(void *ctx, web_status_t *out) {
    (void)ctx;
    wiz_NetInfo ni;
    wizchip_getnetinfo(&ni);
    out->fw_version = PSHB_FW_VERSION;
    out->uptime_ms = to_ms_since_boot(get_absolute_time());
    memcpy(out->mac, ni.mac, 6);
    memcpy(out->ip, ni.ip, 4);
    memcpy(out->mask, ni.sn, 4);
    memcpy(out->gw, ni.gw, 4);
    memcpy(out->dns, ni.dns, 4);
    out->link_up = s_state != C1_NO_LINK;
    out->net_state = net_state_text();
    out->usb_status = g_bridge.usb_status;
    out->failsafe = g_bridge.failsafe_active;
    out->rx_ok = g_bridge.stats.rx_ok;
    out->rx_auth_fail = g_bridge.stats.rx_auth_fail;
    out->rx_replay = g_bridge.stats.rx_replay;
    out->rx_filtered = g_bridge.stats.rx_filtered;
    out->rx_malformed = g_bridge.stats.rx_malformed;
    out->partition = "-";
}

static bool be_save(void *ctx, const pshb_settings_t *s) {
    (void)ctx;
    pshb_settings_t copy = *s;
    if (!settings_save(&g_settings_flash, &copy)) return false;
    g_settings = copy;
    return true;
}

static void be_apply(void *ctx, const pshb_settings_t *s) {
    (void)ctx;
    input_cfg_post(s);
}

static void be_reboot(void *ctx) {
    (void)ctx;
    s_reboot = true;
}

static void be_factory_reset(void *ctx) {
    (void)ctx;
    settings_erase(&g_settings_flash);
    s_reboot = true;
}

static void be_random(void *ctx, uint8_t *buf, size_t n) {
    (void)ctx;
    while (n) {
        uint64_t r = get_rand_64();
        size_t k = n < sizeof(r) ? n : sizeof(r);
        memcpy(buf, &r, k);
        buf += k;
        n -= k;
    }
}

static uint64_t be_now(void *ctx) {
    (void)ctx;
    return time_us_64();
}

static const web_backend_t k_backend = {
    NULL, be_status, be_save, be_apply, be_reboot, be_factory_reset, be_random, be_now,
};

// ---------------------------------------------------------------- BOOTSEL

static void bootsel_service(void) {
    static absolute_time_t next, pressed_since;
    static bool pressed;
    if (!time_reached(next)) return;
    next = make_timeout_time_ms(BOOTSEL_POLL_MS);
    bool now = bootsel_pressed();
    if (now && !pressed) pressed_since = get_absolute_time();
    pressed = now;
    if (pressed && absolute_time_diff_us(pressed_since, get_absolute_time()) > FACTORY_RESET_HOLD_MS * 1000) {
        printf("system: BOOTSEL held, factory reset\n");
        settings_erase(&g_settings_flash);
        s_reboot = true;
    }
}

// ---------------------------------------------------------------- main

void core1_main(void) {
    absolute_time_t next_second = make_timeout_time_ms(1000);

    reg_dhcp_cbfunc(on_dhcp_assign, on_dhcp_assign, on_dhcp_conflict);
    set_state(C1_NO_LINK);
    web_server_init(&k_backend, &g_settings);

    for (;;) {
        network_service();
        if (s_state == C1_READY) web_server_service();
        bootsel_service();

        if (time_reached(next_second)) {
            next_second = delayed_by_ms(next_second, 1000);
            DHCP_time_handler();
        }

        if (s_reboot) {
            printf("system: reboot\n");
            sleep_ms(50);  // let the last TCP segment leave the chip
            watchdog_reboot(0, 0, 0);
            for (;;) {
                tight_loop_contents();
            }
        }
    }
}
