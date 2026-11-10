#include "core1.h"

#include <stdio.h>
#include <string.h>

#include "pico/stdlib.h"

#include "dhcp.h"
#include "net_shared.h"
#include "protocol.h"
#include "socket.h"
#include "w5500_port.h"
#include "wizchip_conf.h"

#define LINK_POLL_MS   100
#define DHCP_RETRY_MS  5000

typedef enum { C1_NO_LINK, C1_DHCP, C1_DHCP_WAIT, C1_READY } c1_state_t;

static uint8_t s_dhcp_buf[548];  // RIP_MSG_SIZE
static volatile bool s_dhcp_assigned;
static volatile bool s_dhcp_conflict;
static bool s_udp_open;

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

// Close socket 1 after core 0 has let go of it. (Used when the port changes.)
__attribute__((unused)) static void udp_close(void) {
    atomic_store(&g_net.udp_enable, false);
    while (atomic_load(&g_net.udp_active)) {
        tight_loop_contents();
    }
    close(W5500_SOCK_UDP);
    s_udp_open = false;
}

static void print_ip(const char *what) {
    wiz_NetInfo ni;
    wizchip_getnetinfo(&ni);
    printf("net: %s %u.%u.%u.%u/%u.%u.%u.%u gw %u.%u.%u.%u\n", what, ni.ip[0], ni.ip[1], ni.ip[2],
           ni.ip[3], ni.sn[0], ni.sn[1], ni.sn[2], ni.sn[3], ni.gw[0], ni.gw[1], ni.gw[2], ni.gw[3]);
    atomic_store(&g_net.ip, ip_to_u32(ni.ip));
}

void core1_main(void) {
    c1_state_t st = C1_NO_LINK;
    absolute_time_t next_link = get_absolute_time();
    absolute_time_t next_second = make_timeout_time_ms(1000);
    absolute_time_t dhcp_retry_at = get_absolute_time();
    uint16_t port = PSHB_UDP_PORT_DEFAULT;

    reg_dhcp_cbfunc(on_dhcp_assign, on_dhcp_assign, on_dhcp_conflict);
    set_state(st);

    for (;;) {
        if (time_reached(next_link)) {
            next_link = make_timeout_time_ms(LINK_POLL_MS);
            bool link = wizphy_getphylink() == PHY_LINK_ON;
            if (!link && st != C1_NO_LINK) {
                printf("net: link down\n");
                if (st == C1_DHCP || st == C1_READY) DHCP_stop();
                atomic_store(&g_net.ip, 0);
                st = C1_NO_LINK;
                set_state(st);
            } else if (link && st == C1_NO_LINK) {
                printf("net: link up, starting DHCP\n");
                DHCP_init(W5500_SOCK_DHCP, s_dhcp_buf);
                st = C1_DHCP;
                set_state(st);
            }
        }

        if (st == C1_DHCP_WAIT && time_reached(dhcp_retry_at)) {
            DHCP_init(W5500_SOCK_DHCP, s_dhcp_buf);
            st = C1_DHCP;
        }

        if (st == C1_DHCP || st == C1_READY) {
            uint8_t r = DHCP_run();
            if (s_dhcp_assigned) {
                s_dhcp_assigned = false;
                print_ip("DHCP lease");
                if (!s_udp_open) udp_open(port);
                st = C1_READY;
                set_state(st);
            }
            if (s_dhcp_conflict || r == DHCP_FAILED) {
                printf("net: DHCP %s, retrying\n", s_dhcp_conflict ? "conflict" : "failed");
                s_dhcp_conflict = false;
                DHCP_stop();
                atomic_store(&g_net.ip, 0);
                dhcp_retry_at = make_timeout_time_ms(DHCP_RETRY_MS);
                st = C1_DHCP_WAIT;
                set_state(st);
            }
        }

        if (time_reached(next_second)) {
            next_second = delayed_by_ms(next_second, 1000);
            DHCP_time_handler();
        }
    }
}
