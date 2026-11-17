#include "core1.h"
#include "clock.h"
#include "guard.h"
#include "crypto_help.h"

#include <stdio.h>
#include <string.h>

#include "hardware/watchdog.h"
#include "pico/rand.h"
#include "pico/stdlib.h"

#include "board_name.h"
#include "bootsel.h"
#include "dhcp.h"
#include "diag.h"
#include "discovery.h"
#include "flash_store.h"
#include "mdns.h"
#include "net_shared.h"
#include "ota.h"
#include "proto.h"
#include "protocol.h"
#include "pshb_config.h"
#include "pshb_version.h"
#include "socket.h"
#include "w5500_port.h"
#include "web.h"
#include "web_server.h"
#include "wizchip_conf.h"

#define LINK_POLL_MS        100
#define DHCP_RETRY_MS       5000
#define UDP_REOPEN_MS       1000
#define CHIP_CHECK_MS       3000
// Core 1 shares the SPI bus with core 0's input path: poll the chip only as
// often as needed (DHCP works in seconds; a new HTTP connection may wait
// 1 ms, an open one is served without pause).
#define DHCP_POLL_US        10000
#define WEB_IDLE_POLL_US    1000
#define UDP_TOS_EF          0xB8  // DSCP Expedited Forwarding, like the app's packets
// Discovery requests wait this long at most: the app listens for a second.
#define DISCOVERY_POLL_US   20000
#define BOOTSEL_POLL_MS     100
#define FACTORY_RESET_HOLD_MS 5000
// A firmware on trial is confirmed after core 1 ran this long (core1_main),
// well inside GUARD_TRIAL_MS.
#define CONFIRM_AFTER_MS    3000

typedef enum { C1_NO_LINK, C1_DHCP, C1_DHCP_WAIT, C1_READY } c1_state_t;

// ioLibrary's DHCP client reads every datagram for port 68 into this buffer
// in one piece, as large as it arrived (up to the socket's receive buffer),
// not only the 548 bytes of a DHCP message. Any larger datagram -- also a
// broadcast answer meant for another device -- would overwrite what follows.
// Cast to RIP_MSG *. Room behind the largest datagram: ioLibrary's option
// parser does not check an option's length against the end of the message
// and can read up to 257 bytes past it; those reads stay in this buffer.
static uint8_t s_dhcp_buf[W5500_DHCP_RX_KB * 1024 + 260] __attribute__((aligned(4)));
static volatile bool s_dhcp_assigned;
static volatile bool s_dhcp_conflict;
static bool s_udp_open;
static bool s_disc_open;  // the discovery socket (W5500_SOCK_DISCOVERY)
static bool s_mdns_open;  // the mDNS socket (W5500_SOCK_MDNS)
// An address was published (link up, new lease, static start, also after
// the chip was set up anew): mDNS probes and announces again.
static bool s_mdns_restart;
static uint8_t s_mask[4];  // subnet mask of the published address
static c1_state_t s_state = C1_NO_LINK;
// The DHCP lease in use. Its address stays until the lease ends, also while
// the server does not answer a renewal or the cable was out for a moment:
// ioLibrary's client forgets a lease after a failed renewal, and starting
// it over (DHCP_init) clears the address at once.
static bool s_lease;
static absolute_time_t s_lease_end;
// The address mode this start runs with: a mode saved in the web interface
// applies after the restart that follows (until then the DHCP client of a
// static start was never set up).
static uint8_t s_ip_mode;
static bool s_reboot;
// The settings were erased (factory reset), the restart is pending: nothing
// may write the old ones from RAM back meanwhile (a signed request from the
// app, the web interface while the BOOTSEL button is still held).
static bool s_settings_erased;

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

// Called by our copy of dhcp.c (cmake/iolibrary.cmake) after it declined a
// conflicting address, instead of its busy wait for a 1 s tick that only
// this loop advances: give the DECLINE time to leave.
void pshb_dhcp_decline_wait(void) {
    for (int i = 0; i < 10; i++) {
        sleep_ms(100);
        guard_core1_alive();
    }
}

static void on_dhcp_conflict(void) {
    s_dhcp_conflict = true;
}

// Added to ioLibrary's dhcp.c (cmake/iolibrary.cmake).
uint8_t pshb_dhcp_bound(void);
void pshb_dhcp_renew_now(void);

// The host name the DHCP client sends (read by dhcp.c, see
// cmake/iolibrary.cmake): the board's network name.
char pshb_dhcp_hostname[PSHB_NAME_SIZE];

// The board's default name ("ps-hid-bridge-96c33d"), from its MAC address.
static void default_name(char out[PSHB_NAME_SIZE]) {
    uint8_t mac[6];
    w5500_get_mac(mac);
    pshb_board_name(mac, out, PSHB_NAME_SIZE);
}

// Set the DHCP host name from the settings; true if it changed.
static bool update_dhcp_hostname(void) {
    char def[PSHB_NAME_SIZE], host[PSHB_NAME_SIZE];
    default_name(def);
    pshb_hostname_from_name(settings_board_name(&g_settings), def, host, sizeof(host));
    if (strcmp(host, pshb_dhcp_hostname) == 0) return false;
    memcpy(pshb_dhcp_hostname, host, sizeof(host));
    return true;
}

static bool lease_valid(void) {
    return s_lease && !time_reached(s_lease_end);
}

// Call after every DHCP_run: each ACK (first one or renewal) starts the
// lease anew.
static void lease_track(void) {
    static bool bound;
    bool now = pshb_dhcp_bound();
    if (now && !bound) {
        uint32_t secs = getDHCPLeasetime();
        s_lease = true;
        s_lease_end = secs == 0xFFFFFFFFu ? at_the_end_of_time
                                          : delayed_by_us(get_absolute_time(), (uint64_t)secs * 1000000u);
    }
    bound = now;
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
    setSn_TOS(W5500_SOCK_UDP, UDP_TOS_EF);
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
    memcpy(s_mask, ni.sn, 4);
    s_mdns_restart = true;
}

static void start_addressing(void) {
    if (s_ip_mode == PSHB_IP_STATIC) {
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
        s_lease = false;
        DHCP_init(W5500_SOCK_DHCP, s_dhcp_buf);
        set_state(C1_DHCP);
    }
}

// The W5500 reset by itself (brown-out, ESD): its sockets are closed and its
// addresses gone, and input would stop without a word. Set it up again and
// start the network over (the link poll below).
static void chip_check(void) {
    static absolute_time_t next;
    if (!time_reached(next)) return;
    next = make_timeout_time_ms(CHIP_CHECK_MS);
    if (!w5500_chip_lost() || !w5500_chip_lost()) return;  // twice: not a garbled read
    printf("net: the network chip lost its setup, setting it up again\n");
    // Core 0 lets go of the input socket first (net_service).
    atomic_store(&g_net.udp_enable, false);
    absolute_time_t until = make_timeout_time_ms(100);
    while (atomic_load(&g_net.udp_active) && !time_reached(until)) tight_loop_contents();
    if (!w5500_chip_setup()) return;  // tried again at the next check
    s_udp_open = false;
    s_disc_open = false;
    s_mdns_open = false;
    s_lease = false;  // the address is gone with the reset
    atomic_store(&g_net.ip, 0);
    set_state(C1_NO_LINK);
}

static void network_service(void) {
    static absolute_time_t next_link, dhcp_retry_at, next_open;
    chip_check();
    bool dhcp = s_ip_mode == PSHB_IP_DHCP;
    if (time_reached(next_link)) {
        next_link = make_timeout_time_ms(LINK_POLL_MS);
        bool link = wizphy_getphylink() == PHY_LINK_ON;
        if (!link && s_state != C1_NO_LINK) {
            printf("net: link down\n");
            // A valid lease is kept (its client paused): the cable may be
            // back in a moment, or the switch only renegotiated.
            if (dhcp && !lease_valid()) DHCP_stop();
            atomic_store(&g_net.ip, 0);
            set_state(C1_NO_LINK);
        } else if (link && s_state == C1_NO_LINK) {
            printf("net: link up\n");
            if (dhcp && lease_valid()) {
                // Ask the server right away whether the address still holds
                // (on another network its server answers NAK and the client
                // starts over).
                pshb_dhcp_renew_now();
                publish_ip("DHCP lease kept");
                set_state(C1_READY);
            } else {
                start_addressing();
            }
        }
    }

    // The input socket did not open (the chip refused it): try again, the
    // app cannot reach the board without it.
    if (s_state == C1_READY && !s_udp_open && time_reached(next_open)) {
        next_open = make_timeout_time_ms(UDP_REOPEN_MS);
        udp_open(g_settings.udp_port);
    }

    if (s_state == C1_DHCP_WAIT && time_reached(dhcp_retry_at)) {
        DHCP_init(W5500_SOCK_DHCP, s_dhcp_buf);
        set_state(C1_DHCP);
    }

    static absolute_time_t next_dhcp;
    if ((s_state == C1_DHCP || s_state == C1_READY) && dhcp && time_reached(next_dhcp)) {
        next_dhcp = make_timeout_time_us(DHCP_POLL_US);
        uint8_t r = DHCP_run();
        lease_track();
        if (s_dhcp_assigned) {
            s_dhcp_assigned = false;
            publish_ip("DHCP lease");
            if (!s_udp_open) udp_open(g_settings.udp_port);
            set_state(C1_READY);
        }
        if (s_dhcp_conflict || (r == DHCP_FAILED && !lease_valid())) {
            printf("net: DHCP %s, retrying\n", s_dhcp_conflict ? "conflict" : "failed");
            s_dhcp_conflict = false;
            s_lease = false;
            DHCP_stop();
            atomic_store(&g_net.ip, 0);
            dhcp_retry_at = make_timeout_time_ms(DHCP_RETRY_MS);
            set_state(C1_DHCP_WAIT);
        } else if (r == DHCP_FAILED) {
            // The client asks again on its own (DISCOVER).
            printf("net: no DHCP answer, the address stays until its lease ends\n");
        } else if (s_lease && !lease_valid()) {
            printf("net: DHCP lease ended\n");
            s_lease = false;
            atomic_store(&g_net.ip, 0);
            DHCP_init(W5500_SOCK_DHCP, s_dhcp_buf);  // clears the address
            set_state(C1_DHCP);
        }
    }
}

// ---------------------------------------------------------------- discovery

// Answers an app looking for boards (protocol/SPEC.md "Discovery") on its
// own socket, polled here: no interrupt, so INTn and core 0's input path
// stay as they are. Broadcasts are welcome on this socket.
static void discovery_service(void) {
    static absolute_time_t next_poll, next_open;
    static pshb_discovery_limit_t limit;
    // A datagram never exceeds the socket's 1 KB receive buffer: read in
    // one piece, whatever its size.
    static uint8_t req[1024];
    // An input port set to the discovery port by older firmware (the web
    // interface refuses it now) keeps it: no discovery then.
    if (s_state != C1_READY || g_settings.udp_port == PSHB_DISCOVERY_PORT || !time_reached(next_poll)) return;
    next_poll = make_timeout_time_us(DISCOVERY_POLL_US);
    if (!s_disc_open) {
        if (!time_reached(next_open)) return;
        next_open = make_timeout_time_ms(UDP_REOPEN_MS);
        if (socket(W5500_SOCK_DISCOVERY, Sn_MR_UDP, PSHB_DISCOVERY_PORT, SF_IO_NONBLOCK) != W5500_SOCK_DISCOVERY) {
            printf("net: cannot open the discovery socket\n");
            return;
        }
        s_disc_open = true;
    }
    if (getSn_RX_RSR(W5500_SOCK_DISCOVERY) == 0) return;
    uint8_t ip[4];
    uint16_t port;
    int32_t n = recvfrom(W5500_SOCK_DISCOVERY, req, sizeof(req), ip, &port);
    if (n <= 0) return;

    pshb_discovery_info_t info = {0};
    w5500_get_mac(info.mac);
    const char *name = settings_board_name(&g_settings);
    if (name[0]) {
        snprintf(info.name, sizeof(info.name), "%s", name);
    } else {
        pshb_board_name(info.mac, info.name, sizeof(info.name));
    }
    info.udp_port = g_settings.udp_port;
    info.fw_version = PSHB_FW_VERSION;
    if (g_settings.psk_set) info.flags |= PSHB_DISCOVERY_KEY_SET;
    if (s_ip_mode == PSHB_IP_STATIC) info.flags |= PSHB_DISCOVERY_STATIC_IP;
    if (g_bridge.no_partition_table) info.flags |= PSHB_DISCOVERY_NO_PARTITION_TABLE;
    uint8_t answer[PSHB_DISCOVERY_LEN];
    size_t len = pshb_discovery_answer(req, (size_t)n, &info, answer, sizeof(answer));
    if (len && port != 0 && pshb_discovery_allow(&limit, time_us_64())) {
        // Waits for the chip's ARP to the asker (fresh: it just sent this),
        // at most the retry time (w5500_chip_setup).
        sendto(W5500_SOCK_DISCOVERY, answer, (uint16_t)len, ip, port);
    }
}

// ---------------------------------------------------------------- mDNS

// The board answers as "<network name>.local" and announces its web
// interface (core/mdns.c) on a socket of its own, polled here like
// discovery: no interrupt, core 0's input path never waits for more than
// one SPI piece of it.
//
// The socket joins 224.0.0.251 when it opens (Sn_MR MULTI, with Sn_DIPR,
// Sn_DPORT and Sn_DHAR set to the group before OPEN). The W5500 sends its
// IGMP report only then and does not answer a querier's membership queries,
// so a switch with IGMP snooping forgets the board after a few minutes
// (260 s by default) and stops passing the group's packets to its port.
// The socket is therefore opened anew every MDNS_REJOIN_MS: a Leave and a
// Join, at worst a datagram in the receive buffer is lost (mDNS asks again).
// Sn_MR bit 5 stays 0, which is IGMP version 2: ioLibrary's SF_IGMP_VER2
// sets that bit, and on the W5500 it selects version 1.
#define MDNS_POLL_US 20000
#define MDNS_REJOIN_MS 60000
// Datagrams handled per poll at most: every mDNS packet in the network
// lands in the 1 KB receive buffer (other devices' queries and answers
// too) and is read away here.
#define MDNS_READS_PER_POLL 4
#define MDNS_SEND_WAIT_MS 10

static const uint8_t k_mdns_group[4] = PSHB_MDNS_GROUP_INIT;
static const uint8_t k_mdns_mac[6] = PSHB_MDNS_GROUP_MAC_INIT;
static pshb_mdns_t s_mdns;
static pshb_mdns_info_t s_mdns_info;  // what s_mdns was last started with

// The socket's destination back to the group: a unicast answer changes it
// (and the chip's ARP writes the asker's MAC address into Sn_DHAR).
static void mdns_to_group(void) {
    setSn_DIPR(W5500_SOCK_MDNS, (uint8_t *)k_mdns_group);
    setSn_DPORT(W5500_SOCK_MDNS, PSHB_MDNS_PORT);
    setSn_DHAR(W5500_SOCK_MDNS, (uint8_t *)k_mdns_mac);
}

static bool mdns_open(void) {
    mdns_to_group();  // before OPEN: the group to join
    s_mdns_open = socket(W5500_SOCK_MDNS, Sn_MR_UDP, PSHB_MDNS_PORT,
                         SF_MULTI_ENABLE | SF_BROAD_BLOCK | SF_IO_NONBLOCK) == W5500_SOCK_MDNS;
    if (!s_mdns_open) {
        printf("net: cannot open the mDNS socket\n");
        return false;
    }
    setSn_TTL(W5500_SOCK_MDNS, 255);  // RFC 6762 section 11
    return true;
}

// To the group: SEND_MAC with the group's Ethernet address, no ARP (which
// nobody would answer for a group address).
static void mdns_send_group(const uint8_t *buf, size_t len) {
    const uint8_t sn = W5500_SOCK_MDNS;
    if (len > getSn_TX_FSR(sn)) return;
    mdns_to_group();
    wiz_send_data(sn, (uint8_t *)buf, (uint16_t)len);
    setSn_IR(sn, (Sn_IR_SENDOK | Sn_IR_TIMEOUT));
    setSn_CR(sn, Sn_CR_SEND_MAC);
    while (getSn_CR(sn)) tight_loop_contents();
    // Out within microseconds without ARP; bounded all the same.
    absolute_time_t until = make_timeout_time_ms(MDNS_SEND_WAIT_MS);
    while (!(getSn_IR(sn) & (Sn_IR_SENDOK | Sn_IR_TIMEOUT)) && !time_reached(until)) tight_loop_contents();
    setSn_IR(sn, (Sn_IR_SENDOK | Sn_IR_TIMEOUT));
}

// What the board answers with right now.
static void mdns_info_now(pshb_mdns_info_t *info) {
    memset(info, 0, sizeof(*info));  // compared with memcmp below
    uint32_t ip = atomic_load(&g_net.ip);
    for (int i = 0; i < 4; i++) info->ip[i] = (uint8_t)(ip >> (24 - 8 * i));
    memcpy(info->mask, s_mask, 4);
    memcpy(info->host, pshb_dhcp_hostname, sizeof(info->host));  // the network name, as DHCP sends it
    default_name(info->fallback);
    const char *name = settings_board_name(&g_settings);
    snprintf(info->instance, sizeof(info->instance), "%s", name[0] ? name : info->fallback);
    info->http_port = 80;  // web_server.c
}

static void mdns_service(void) {
    static absolute_time_t next_poll, next_open, rejoin_at;
    // A datagram never exceeds the socket's 1 KB receive buffer (read in
    // one piece), an answer never its 1 KB send buffer.
    static uint8_t pkt[1024], out[1024];
    static char shown[PSHB_NAME_SIZE];  // the name last logged as answered
    // An input port set to 5353 by older firmware (the web interface
    // refuses it now) keeps it: no mDNS then.
    if (s_state != C1_READY || g_settings.udp_port == PSHB_MDNS_PORT || !time_reached(next_poll)) return;
    next_poll = make_timeout_time_us(MDNS_POLL_US);

    // A new address or link (publish_ip) or a new name (be_apply, noticed
    // here): probe and announce again. After a new address or link the
    // group is joined anew as well (another switch port, maybe).
    pshb_mdns_info_t info;
    mdns_info_now(&info);
    if (s_mdns_restart || memcmp(&info, &s_mdns_info, sizeof(info)) != 0) {
        if (s_mdns_restart) s_mdns_open = false;
        s_mdns_restart = false;
        s_mdns_info = info;
        pshb_mdns_reset(&s_mdns, &info, time_us_64() / 1000);
        printf("mdns: probing for %s.local\n", info.host);
    }
    if (!s_mdns_open || time_reached(rejoin_at)) {
        if (!time_reached(next_open)) return;
        next_open = make_timeout_time_ms(UDP_REOPEN_MS);
        if (!mdns_open()) return;
        rejoin_at = make_timeout_time_ms(MDNS_REJOIN_MS);
    }

    for (int i = 0; i < MDNS_READS_PER_POLL && getSn_RX_RSR(W5500_SOCK_MDNS) > 0; i++) {
        uint8_t ip[4];
        uint16_t port;
        int32_t n = recvfrom(W5500_SOCK_MDNS, pkt, sizeof(pkt), ip, &port);
        if (n <= 0) break;
        pshb_mdns_dest_t dest;
        size_t len = pshb_mdns_input(&s_mdns, pkt, (size_t)n, ip, port, time_us_64() / 1000, out, sizeof(out),
                                     &dest);
        if (!len) continue;
        if (dest == PSHB_MDNS_TO_GROUP) {
            mdns_send_group(out, len);
        } else if (port != 0) {
            // Waits for the chip's ARP to the asker (it just sent this), at
            // most the retry time (w5500_chip_setup); limited per second.
            sendto(W5500_SOCK_MDNS, out, (uint16_t)len, ip, port);
            mdns_to_group();
        }
    }
    size_t len = pshb_mdns_poll(&s_mdns, time_us_64() / 1000, out, sizeof(out));
    if (len) mdns_send_group(out, len);
    // The name answered changes after probing, or to the default name after
    // a conflict (or to none: both taken).
    const char *name = pshb_mdns_name(&s_mdns);
    if (strcmp(name ? name : "", shown) != 0) {
        snprintf(shown, sizeof(shown), "%s", name ? name : "");
        if (name) printf("mdns: answering as %s.local\n", name);
    }
}

// ---------------------------------------------------------------- web backend

static const char *net_state_text(void) {
    switch (atomic_load(&g_net.state)) {
    case NET_STATE_CHIP_ERROR: return "Network chip not responding";
    case NET_STATE_NO_LINK: return "No network cable";
    case NET_STATE_DHCP: return "Waiting for DHCP";
    case NET_STATE_READY: return "Ready";
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
    out->partition = g_running_slot;
    out->watchdog_reset = guard_watchdog_reset();
    static char local[PSHB_NAME_SIZE + 6];
    const char *mdns_name = s_state == C1_READY ? pshb_mdns_name(&s_mdns) : NULL;
    if (mdns_name) {
        snprintf(local, sizeof(local), "%s.local", mdns_name);
    } else {
        local[0] = '\0';
    }
    out->local_name = local;
    out->lat_usb_count = g_diag.usb.count;
    out->lat_usb_p50_us = latency_percentile(&g_diag.usb, 50);
    out->lat_usb_p95_us = latency_percentile(&g_diag.usb, 95);
    out->lat_usb_max_us = g_diag.usb.max_us;
    out->lat_rx_p50_us = latency_percentile(&g_diag.rx, 50);
    out->lat_rx_max_us = g_diag.rx.max_us;
    out->mouse_inputs = g_diag.mouse_inputs;
    out->mouse_waits = g_diag.mouse_waits;
    out->mouse_misses = g_diag.mouse_misses;
    out->mouse_lead_us = g_diag.mouse_lead_us;
}

static void lat(web_lat_t *out, const latency_hist_t *h) {
    out->n = h->count;
    out->min = h->min_us;
    out->p50 = latency_percentile(h, 50);
    out->p99 = latency_percentile(h, 99);
    out->max = h->max_us;
}

static void be_diag(void *ctx, web_diag_t *out) {
    (void)ctx;
    lat(&out->ready, &g_diag.ready);
    lat(&out->ready1, &g_diag.ready1);
    lat(&out->detect, &g_diag.detect);
    lat(&out->bus, &g_diag.bus);
    lat(&out->spi, &g_diag.spi);
    lat(&out->handle, &g_diag.handle);
    lat(&out->usb, &g_diag.usb);
    lat(&out->gap, &g_diag.gap);
    lat(&out->armed, &g_diag.armed);
    out->rounds = g_diag.rounds;
    out->datagrams = g_diag.datagrams;
    out->core0_spi = w5500_core0_spi();
    out->cr_busy = w5500_cr_busy();
    out->mouse_inputs = g_diag.mouse_inputs;
    out->mouse_waits = g_diag.mouse_waits;
    out->mouse_fetches = g_diag.mouse_fetches;
    out->mouse_misses = g_diag.mouse_misses;
    out->mouse_lead_us = g_diag.mouse_lead_us;
    out->mouse_host_skips = g_diag.mouse_host_skips;
    out->mouse_floor_us = g_diag.mouse_floor_us;
    out->crypto_min_ns = g_diag.crypto_min_ns;
    out->crypto_avg_ns = g_diag.crypto_avg_ns;
    out->crypto_max_ns = g_diag.crypto_max_ns;
    out->crypto_fast = g_diag.crypto_fast;
    out->crypto_helped = crypto_help_helped();
    out->crypto_alone = crypto_help_alone();
    out->spi_hz = w5500_spi_hz();
    out->bus_timeouts = g_diag.bus_timeouts;
}

static void be_clock(void *ctx, web_clock_t *out) {
    (void)ctx;
    out->cpu_hz = clock_cpu_hz();
    out->spi_hz = w5500_spi_hz();
    out->cpu_fell_back = clock_fell_back();
    out->spi_fast_failed = w5500_spi_fast_failed();
}

static void be_diag_reset(void *ctx) {
    (void)ctx;
    g_diag.reset_request = true;  // core 0 owns the statistics
}

// How long decrypting one INPUT packet takes, as on the input path (same
// CPU speed on both cores; core 1 measures, so core 0 does not wait).
static void crypto_bench(void) {
    uint8_t key[PSHB_KEY_LEN], payload[PSHB_INPUT_PAYLOAD_LEN] = {0}, out[PSHB_INPUT_PAYLOAD_LEN];
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    for (size_t i = 0; i < sizeof(key); i++) key[i] = (uint8_t)get_rand_32();
    size_t len = proto_seal(key, PSHB_DIR_APP_TO_BOARD, PSHB_TYPE_INPUT, 1, payload, sizeof(payload), pkt,
                            sizeof(pkt));
    enum { RUNS = 1000 };
    uint32_t min = UINT32_MAX, max = 0;
    uint64_t start = time_us_64();
    for (int i = 0; i < RUNS; i++) {
        uint64_t t0 = time_us_64();
        if (!proto_decrypt(key, PSHB_DIR_APP_TO_BOARD, pkt, len, out)) {
            printf("crypto: self-check failed\n");
            return;
        }
        uint32_t d = (uint32_t)(time_us_64() - t0);
        if (d < min) min = d;
        if (d > max) max = d;
    }
    uint64_t total = time_us_64() - start;
    g_diag.crypto_min_ns = min * 1000;
    g_diag.crypto_max_ns = max * 1000;
    g_diag.crypto_avg_ns = (uint32_t)(total * 1000 / RUNS);
    printf("crypto: decrypting a %u-byte packet takes %lu ns on average\n", (unsigned)len,
           (unsigned long)g_diag.crypto_avg_ns);
}

#if PSHB_TIMING_UART
static void timing_print(void) {
    web_diag_t d;
    be_diag(NULL, &d);
    printf("lat: ready %lu/%lu/%lu us (p50/p99/max), bus %lu/%lu/%lu, spi %lu, handle %lu, "
           "missed %lu of %lu fetches\n",
           (unsigned long)d.ready.p50, (unsigned long)d.ready.p99, (unsigned long)d.ready.max,
           (unsigned long)d.bus.p50, (unsigned long)d.bus.p99, (unsigned long)d.bus.max,
           (unsigned long)d.spi.p50, (unsigned long)d.handle.p50, (unsigned long)d.mouse_misses,
           (unsigned long)d.mouse_fetches);
}
#endif

static bool be_save(void *ctx, const pshb_settings_t *s) {
    (void)ctx;
    if (s_settings_erased) return false;
    pshb_settings_t copy = *s;
    if (!settings_save(&g_settings_flash, &copy)) return false;
    g_settings = copy;
    return true;
}

static void be_apply(void *ctx, const pshb_settings_t *s) {
    (void)ctx;
    input_cfg_post(s);
    // A new name: the router learns it with a lease renewal right away
    // (with a static address there is no DHCP to tell); mDNS takes it up
    // by itself (mdns_service).
    if (update_dhcp_hostname()) {
        printf("net: network name %s\n", pshb_dhcp_hostname);
        if (s_ip_mode == PSHB_IP_DHCP) pshb_dhcp_renew_now();
    }
}

static void be_reboot(void *ctx) {
    (void)ctx;
    s_reboot = true;
}

static void be_factory_reset(void *ctx) {
    (void)ctx;
    settings_erase(&g_settings_flash);
    s_settings_erased = true;
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

static bool be_ota_begin(void *ctx, uint32_t size, char *err, size_t cap) {
    (void)ctx;
    return ota_begin(size, err, cap);
}

static bool be_ota_write(void *ctx, const uint8_t *data, size_t len) {
    (void)ctx;
    return ota_write(data, len);
}

static bool be_ota_finish(void *ctx, uint32_t crc, char *err, size_t cap) {
    (void)ctx;
    return ota_finish(crc, err, cap);
}

static void be_ota_abort(void *ctx) {
    (void)ctx;
    ota_abort();
}

static volatile bool s_ota_activate;

static void be_ota_activate(void *ctx) {
    (void)ctx;
    s_ota_activate = true;
}

static const web_backend_t k_backend = {
    .ctx = NULL,
    .status = be_status,
    .diag = be_diag,
    .diag_reset = be_diag_reset,
    .clock = be_clock,
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

// ---------------------------------------------------------------- BOOTSEL

// Held for 5 s: erase the settings, then restart once the button is let go.
// A restart while it is still held would start the boot ROM's USB drive
// (BOOTSEL mode) instead of the firmware.
static void bootsel_service(void) {
    static absolute_time_t next, pressed_since;
    static bool pressed, erased;
    if (!time_reached(next)) return;
    next = make_timeout_time_ms(BOOTSEL_POLL_MS);
    bool now = bootsel_pressed();
    if (now && !pressed) pressed_since = get_absolute_time();
    pressed = now;
    if (erased) {
        if (!pressed) s_reboot = true;
        return;
    }
    if (pressed && absolute_time_diff_us(pressed_since, get_absolute_time()) > FACTORY_RESET_HOLD_MS * 1000) {
        printf("system: BOOTSEL held, factory reset (restart when released)\n");
        settings_erase(&g_settings_flash);
        s_settings_erased = true;
        erased = true;
    }
}

// ---------------------------------------------------------------- main

void core1_main(void) {
    absolute_time_t next_second = make_timeout_time_ms(1000);

    crypto_bench();
    crypto_help_init_core1();
    if (clock_fell_back() && g_settings.cpu_200mhz) {
        // 200 MHz did not run stably: keep 150 MHz from now on (the web
        // interface and the app say why).
        pshb_settings_t next = g_settings;
        next.cpu_200mhz = 0;
        if (!s_settings_erased && settings_save(&g_settings_flash, &next)) g_settings = next;
        printf("clock: back to 150 MHz for good\n");
    }
    update_dhcp_hostname();  // before the first DHCP message
    reg_dhcp_cbfunc(on_dhcp_assign, on_dhcp_assign, on_dhcp_conflict);
    s_ip_mode = g_settings.ip_mode;
    set_state(C1_NO_LINK);
    web_server_init(&k_backend, &g_settings);

    absolute_time_t next_web = get_absolute_time();
    // A new firmware on trial is confirmed once this loop (network, web
    // server) has run this long; core 0 is alive too, or the guard would
    // have let the watchdog reset the board.
    absolute_time_t confirm_at = make_timeout_time_ms(CONFIRM_AFTER_MS);
    for (;;) {
        guard_core1_alive();
        if (g_confirm_trial && time_reached(confirm_at)) {
            g_confirm_trial = false;
            if (ota_confirm_if_trial()) guard_confirmed();
        }
        network_service();
        discovery_service();
        mdns_service();
        if (s_state == C1_READY && (web_server_busy() || time_reached(next_web))) {
            next_web = make_timeout_time_us(WEB_IDLE_POLL_US);
            web_server_service();
        }
        bootsel_service();

        if (time_reached(next_second)) {
            next_second = delayed_by_ms(next_second, 1000);
            DHCP_time_handler();
#if PSHB_TIMING_UART
            static unsigned seconds;
            if (++seconds % 10 == 0) timing_print();
#endif
        }

        // Only with the image still ready: another upload may have begun in
        // the same web pass and discarded it. Then nothing restarts, and the
        // guard must keep feeding the watchdog.
        if (s_ota_activate) {
            s_ota_activate = false;
            if (ota_ready()) {
                web_server_drain(1000);  // let the response reach the client
                clock_before_reboot();
                guard_before_reboot();
                ota_activate();
            }
        }

        if (s_reboot) {
            printf("system: reboot\n");
            web_server_drain(1000);  // let the last answer reach the client
            clock_before_reboot();
            guard_before_reboot();
            watchdog_reboot(0, 0, 0);
            for (;;) {
                tight_loop_contents();
            }
        }
    }
}
