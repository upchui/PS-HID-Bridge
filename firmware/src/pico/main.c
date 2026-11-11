// PS5 HID Bridge firmware -- entry point and core 0 loop.
//
// Core 0 owns the latency-critical input path: the USB device stack and the
// UDP input socket. It never blocks. Core 1 runs network management.

#include <stdio.h>

#include <string.h>

#include "pico/critical_section.h"
#include "pico/multicore.h"
#include "pico/rand.h"
#include "pico/stdlib.h"
#include "tusb.h"

#include "bridge.h"
#include "core1.h"
#include "flash_store.h"
#include "hexutil.h"
#include "hid_out.h"
#include "input_state.h"
#include "net_shared.h"
#include "ota.h"
#include "pshb_config.h"
#include "w5500_port.h"
#include "wizchip_conf.h"
#if PSHB_TEST_PATTERN
#include "test_pattern.h"
#endif

net_shared_t g_net;
pshb_settings_t g_settings;
bridge_t g_bridge;
const char *g_running_slot = "-";

static input_state_t g_input;
static bool s_chip_ok;

// ---------------------------------------------------------------- core 1 -> core 0 config

static critical_section_t s_cfg_lock;
static struct {
    uint8_t key[32];
    bool key_set;
    uint32_t allowed_net, allowed_mask;
} s_cfg;
static volatile bool s_cfg_dirty;

void input_cfg_post(const pshb_settings_t *s) {
    critical_section_enter_blocking(&s_cfg_lock);
    memcpy(s_cfg.key, s->psk, 32);
    s_cfg.key_set = s->psk_set;
    settings_sender_filter(s, &s_cfg.allowed_net, &s_cfg.allowed_mask);
    s_cfg_dirty = true;
    critical_section_exit(&s_cfg_lock);
}

void input_cfg_poll(bridge_t *b) {
    if (!s_cfg_dirty) return;
    critical_section_enter_blocking(&s_cfg_lock);
    bridge_set_key(b, s_cfg.key_set ? s_cfg.key : NULL);
    b->allowed_net = s_cfg.allowed_net;
    b->allowed_mask = s_cfg.allowed_mask;
    s_cfg_dirty = false;
    critical_section_exit(&s_cfg_lock);
}

// ---------------------------------------------------------------- LED
// USB not mounted: 1 Hz blink. Mounted, no network: 4 Hz blink.
// Mounted and ready: on, briefly off on received input.

static absolute_time_t s_activity_until;

static void led_service(void) {
    static absolute_time_t next;
    static bool on;
    bool ready = atomic_load(&g_net.state) == NET_STATE_READY;
    if (hid_out_mounted() && ready) {
        gpio_put(PICO_DEFAULT_LED_PIN, !time_reached(s_activity_until) ? false : true);
        return;
    }
    if (time_reached(next)) {
        on = !on;
        gpio_put(PICO_DEFAULT_LED_PIN, on);
        next = make_timeout_time_ms(hid_out_mounted() ? 125 : 500);
    }
}

// ---------------------------------------------------------------- network input

static void net_service(void) {
    static bool active;
    static absolute_time_t next_poll;

    // Handshake with core 1: only use socket 1 while it is enabled.
    bool enable = atomic_load(&g_net.udp_enable);
    if (enable != active) {
        active = enable;
        atomic_store(&g_net.udp_active, active);
    }
    if (!active) return;

    w5500_udp_tx_poll(W5500_SOCK_UDP);

    // INTn signals new data; a slow poll covers a missed edge.
    if (!w5500_int_asserted() && !time_reached(next_poll)) return;
    next_poll = make_timeout_time_ms(1);
    setSn_IR(W5500_SOCK_UDP, Sn_IR_RECV);

    for (int i = 0; i < 8; i++) {
        uint8_t pkt[PSHB_MAX_PACKET_LEN];
        uint8_t ip[4];
        uint16_t port;
        int n = w5500_udp_recv(W5500_SOCK_UDP, pkt, sizeof(pkt), ip, &port);
        if (n == W5500_RX_EMPTY) break;
        if (n < 0) {
            g_bridge.stats.rx_malformed++;
            continue;
        }
        uint8_t reply[PSHB_MAX_PACKET_LEN];
        size_t rlen = bridge_handle_packet(&g_bridge, pkt, (size_t)n, ip_to_u32(ip), time_us_64(),
                                           reply, sizeof(reply));
        if (rlen) w5500_udp_send_nb(W5500_SOCK_UDP, reply, (uint16_t)rlen, ip, port);
        s_activity_until = make_timeout_time_ms(20);
    }
}

// ---------------------------------------------------------------- test pattern

#if PSHB_TEST_PATTERN
// Moves the mouse along a square, starting a few seconds after mounting.
static void test_pattern_service(void) {
    static uint32_t step;
    static absolute_time_t next;
    if (!hid_out_mounted()) {
        step = 0;
        return;
    }
    if (hid_out_mounted_ms() < PSHB_TP_START_DELAY_MS || !time_reached(next)) return;
    int16_t dx, dy;
    test_pattern_delta(step++, &dx, &dy);
    input_state_add_motion(&g_input, dx, dy, 0, 0);
    next = make_timeout_time_ms(PSHB_TP_STEP_MS);
}
#endif

// ---------------------------------------------------------------- main

static uint8_t usb_status(void) {
    uint8_t s = 0;
    if (hid_out_mounted()) s |= PSHB_STATUS_USB_MOUNTED;
    if (tud_suspended()) s |= PSHB_STATUS_USB_SUSPENDED;
    return s;
}

// Track USB state; release everything when the host goes away or sleeps.
static void usb_status_service(void) {
    static uint8_t prev;
    uint8_t s = usb_status();
    bool lost = (prev & PSHB_STATUS_USB_MOUNTED) && !(s & PSHB_STATUS_USB_MOUNTED);
    bool slept = !(prev & PSHB_STATUS_USB_SUSPENDED) && (s & PSHB_STATUS_USB_SUSPENDED);
    if (lost || slept) bridge_release(&g_bridge);
    g_bridge.usb_status = s;
    prev = s;
}

static void load_settings(void) {
    if (!settings_load(&g_settings_flash, &g_settings)) {
        printf("settings: none stored, using defaults\n");
    }
#ifdef PSHB_DEV_PSK
    // Development builds: a compiled-in key is used until one is set in the web UI.
    if (!g_settings.psk_set) {
        if (hex_decode(PSHB_DEV_PSK, g_settings.psk, sizeof(g_settings.psk))) {
            g_settings.psk_set = 1;
            printf("security: development PSK active\n");
        } else {
            printf("security: PSHB_DEV_PSK is not 64 hex digits\n");
        }
    }
#endif
    bridge_set_key(&g_bridge, g_settings.psk_set ? g_settings.psk : NULL);
    settings_sender_filter(&g_settings, &g_bridge.allowed_net, &g_bridge.allowed_mask);
    if (!g_settings.psk_set) printf("security: no key set yet -- create one in the web interface\n");
}

int main(void) {
    stdio_init_all();
    gpio_init(PICO_DEFAULT_LED_PIN);
    gpio_set_dir(PICO_DEFAULT_LED_PIN, GPIO_OUT);

    printf("ps5-hid-bridge %d.%d.%d build %d: boot%s%s\n", (PSHB_FW_VERSION >> 16) & 0xFF,
           (PSHB_FW_VERSION >> 8) & 0xFF, PSHB_FW_VERSION & 0xFF, PSHB_FW_BUILD,
           PSHB_TEST_PATTERN ? " (test pattern)" : "",
           PSHB_PLAINTEXT_DEBUG ? " (PLAINTEXT DEBUG)" : "");

    input_state_init(&g_input);
    bridge_init(&g_bridge, &g_input, get_rand_64(), get_rand_64(), time_us_64());
    g_bridge.allow_plaintext = PSHB_PLAINTEXT_DEBUG;
    critical_section_init(&s_cfg_lock);
    load_settings();
    hid_out_init(&g_input);
    tud_init(0);

    s_chip_ok = w5500_port_init();
    // After an update the bootrom runs us "on trial": confirm only once the
    // essentials work, otherwise the next reset returns to the old firmware.
#if !PSHB_OTA_NO_CONFIRM
    if (s_chip_ok) ota_confirm_if_trial();
#endif
    g_running_slot = ota_running_slot();
    if (s_chip_ok) {
        multicore_launch_core1(core1_main);
    } else {
        printf("net: W5500 not responding\n");
        atomic_store(&g_net.state, NET_STATE_CHIP_ERROR);
    }

    while (true) {
        tud_task();
        if (s_chip_ok) net_service();
#if PSHB_TEST_PATTERN
        test_pattern_service();
#endif
        input_cfg_poll(&g_bridge);
        usb_status_service();
        bridge_tick(&g_bridge, time_us_64());
        hid_out_service();
        led_service();
    }
}
