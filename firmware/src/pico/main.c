// PS HID Bridge firmware -- entry point and core 0 loop.
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

#include "aead.h"
#include "bridge.h"
#include "clock.h"
#include "guard.h"
#include "proto.h"
#include "core1.h"
#include "crypto_help.h"
#include "diag.h"
#include "flash_store.h"
#include "hexutil.h"
#include "hid_out.h"
#include "input_state.h"
#include "net_shared.h"
#include "ota.h"
#include "pshb_config.h"
#include "pshb_version.h"
#include "w5500_port.h"
#include "wizchip_conf.h"
#if PSHB_TEST_PATTERN
#include "test_pattern.h"
#endif

net_shared_t g_net;
pshb_settings_t g_settings;
bridge_t g_bridge;
pshb_diag_t g_diag;
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

// ---------------------------------------------------------------- crypto

void pshb_chacha20_block_m33(uint32_t out[16], const uint32_t in[16]);

// The assembly ChaCha20 block (chacha20_m33.S) replaces the C one only if
// both agree on all-zero, all-one and pseudo-random states. Before core 1
// starts: nothing else decrypts yet.
static void crypto_select(void) {
    uint32_t in[16], a[16], b[16];
    uint32_t x = get_rand_32() | 1;
    for (int t = 0; t < 64; t++) {
        for (int i = 0; i < 16; i++) {
            x = x * 1664525u + 1013904223u;
            in[i] = t == 0 ? 0 : t == 1 ? 0xFFFFFFFFu : x;
        }
        pshb_chacha20_block_c(a, in);
        pshb_chacha20_block_m33(b, in);
        if (memcmp(a, b, sizeof(a)) != 0) {
            printf("crypto: the fast ChaCha20 block differs, using the C version\n");
            return;
        }
    }
    pshb_chacha20_block = pshb_chacha20_block_m33;
    g_diag.crypto_fast = true;
}

// ---------------------------------------------------------------- diagnostics

void diag_init(void) {
    // Kept: the crypto measurement (made once at start) and the current lead.
    uint32_t cmin = g_diag.crypto_min_ns, cavg = g_diag.crypto_avg_ns, cmax = g_diag.crypto_max_ns;
    uint32_t lead = g_diag.mouse_lead_us, floor = g_diag.mouse_floor_us;
    bool fast = g_diag.crypto_fast;
    memset(&g_diag, 0, sizeof(g_diag));
    g_diag.crypto_fast = fast;
    latency_init(&g_diag.usb, 50);
    latency_init(&g_diag.rx, 5);
    latency_init(&g_diag.ready, 4);
    latency_init(&g_diag.ready1, 2);
    latency_init(&g_diag.detect, 2);
    latency_init(&g_diag.bus, 4);
    latency_init(&g_diag.spi, 1);
    latency_init(&g_diag.handle, 1);
    latency_init(&g_diag.gap, 20);
    latency_init(&g_diag.armed, 20);
    g_diag.crypto_min_ns = cmin;
    g_diag.crypto_avg_ns = cavg;
    g_diag.crypto_max_ns = cmax;
    g_diag.mouse_lead_us = lead;
    g_diag.mouse_floor_us = floor;
    w5500_core0_spi_reset();
    crypto_help_reset_counts();
}

static inline void timing_pin(uint pin, bool on) {
#if PSHB_TIMING_GPIO
    gpio_put(pin, on);
#else
    (void)pin;
    (void)on;
#endif
}

// ---------------------------------------------------------------- network input

// Clock for the PONG_TIMED send stamp (bridge.c).
static uint64_t bridge_clock_us(void) {
    return time_us_64();
}

// ChaCha20 blocks for the datagram being read: computed from its header
// while the DMA still brings in the rest (w5500_udp_recv's `early`).
static pshb_aead_pre_t s_pre;

static void rx_early(void *ctx, const uint8_t *payload, uint16_t len) {
    (void)ctx;
    if (g_bridge.key_set) proto_prepare(&s_pre, g_bridge.key, PSHB_DIR_APP_TO_BOARD, payload, len);
}

// Datagrams per receive round. In between, the main loop serves USB (arms
// reports): a burst of datagrams (WLAN delivers them in bunches) must not
// keep it away for long.
#define RX_PER_ROUND 4
// About the longest receiving and decrypting a datagram takes (bus wait,
// SPI, tag check): a mouse report due within that time is armed before
// (hid_out_service_ahead), so the lead before the host's fetch need not
// cover it.
#define RX_BUDGET_US 100

// Datagrams known to be waiting that raise no interrupt any more (it was
// acknowledged at the start of their round), and since when they wait.
static bool s_rx_more;
static uint64_t s_rx_more_since;

static void net_service(void) {
    static bool active;

    // Handshake with core 1: only use socket 1 while it is enabled.
    bool enable = atomic_load(&g_net.udp_enable);
    if (enable != active) {
        active = enable;
        atomic_store(&g_net.udp_active, active);
        // The socket was (re)opened: look once, whatever INTn says.
        s_rx_more = active;
        s_rx_more_since = time_us_64();
    }
    if (!active) return;

    // INTn is level-triggered: it stays asserted while socket 1 has an
    // unacknowledged RECV (only RECV is enabled, see core1.c udp_open), so
    // no datagram can be missed and nothing needs to be polled: without
    // input, core 0 causes no SPI traffic at all.
    if (!s_rx_more && !w5500_int_asserted()) return;
    uint64_t noticed = time_us_64();
    // When INTn fell (GPIO IRQ). Left-over datagrams wait since their round.
    uint64_t edge = w5500_int_time_take();
    bool had_edge = edge && edge <= noticed;
    if (s_rx_more) {
        edge = s_rx_more_since;
        had_edge = false;
    } else if (!had_edge) {
        edge = noticed;
    }
    timing_pin(PSHB_TIMING_PIN_RX, true);
    hid_out_service_ahead(RX_BUDGET_US);

    // One lock for the whole round: core 1 cannot slip in between the
    // register accesses of a datagram. The interrupt is acknowledged before
    // the chip is asked what it holds: a datagram arriving later raises it
    // again (and SIR follows Sn_IR).
    if (!w5500_lock_for(W5500_LOCK_TIMEOUT_US)) {
        // Core 1 holds the bus far longer than any piece takes: it hangs.
        // Keep USB and the failsafe going (input is let go after 250 ms);
        // the watchdog restarts the board (guard.c).
        g_diag.bus_timeouts++;
        timing_pin(PSHB_TIMING_PIN_RX, false);
        return;
    }
    uint64_t locked = time_us_64();
    w5500_udp_irq_ack(W5500_SOCK_UDP);
    uint16_t left = 0;
    int got = 0;
    s_rx_more = false;
    for (int i = 0; i < RX_PER_ROUND; i++) {
        // A mouse report that fell due while the previous datagram was
        // processed goes out first, and one due during the next.
        if (i > 0) hid_out_service_ahead(RX_BUDGET_US);
        uint8_t pkt[PSHB_MAX_PACKET_LEN];
        uint8_t ip[4];
        uint16_t port;
        uint64_t t0 = time_us_64();
        s_pre.valid = false;
        int n = w5500_udp_recv(W5500_SOCK_UDP, pkt, sizeof(pkt), ip, &port, &left, rx_early, PSHB_HDR_LEN,
                               NULL);
        if (n == W5500_RX_EMPTY) break;
        if (n == W5500_RX_AGAIN) {
            s_rx_more = true;  // look again in a new round, right away
            s_rx_more_since = edge;
            break;
        }
        uint64_t t1 = time_us_64();
        latency_add(&g_diag.spi, (uint32_t)(t1 - t0));
        got++;
        if (n < 0) {
            g_bridge.stats.rx_malformed++;
        } else {
            uint8_t reply[PSHB_MAX_PACKET_LEN];
            g_bridge.input_applied = false;
            g_bridge.rx_at_us = edge;  // PONG_TIMED: when the datagram arrived (INTn)
            size_t rlen = bridge_handle_packet_pre(&g_bridge, pkt, (size_t)n, ip_to_u32(ip), t1, reply,
                                                   sizeof(reply), &s_pre);
            if (s_pre.valid) pshb_wipe(&s_pre, sizeof(s_pre));  // keystream
            uint64_t t2 = time_us_64();
            latency_add(&g_diag.handle, (uint32_t)(t2 - t1));
            if (rlen) w5500_udp_send_nb(W5500_SOCK_UDP, reply, (uint16_t)rlen, ip, port);
            if (g_bridge.input_applied) {
                latency_add(&g_diag.rx, (uint32_t)(t2 - noticed));
                latency_add(&g_diag.ready, (uint32_t)(t2 - edge));
                // Not queued behind another datagram: the path itself.
                if (got == 1 && had_edge) latency_add(&g_diag.ready1, (uint32_t)(t2 - edge));
                hid_out_note_input(edge);
            }
            s_activity_until = make_timeout_time_ms(20);
        }
        hid_out_gap_restart();  // guarded above: no gap for the lead's floor
        if (left < 8) break;  // nothing more known; a new datagram raises INTn again
        if (i == RX_PER_ROUND - 1) {
            // More is waiting: next loop, right after serving USB.
            s_rx_more = true;
            s_rx_more_since = edge;
        }
    }
    // The space of the datagrams goes back to the chip only now, after their
    // input is applied: one pointer write and one RECV for the whole round.
    w5500_udp_commit(W5500_SOCK_UDP);
    w5500_unlock();
    timing_pin(PSHB_TIMING_PIN_RX, false);
    if (got) {
        g_diag.rounds++;
        g_diag.datagrams += (uint32_t)got;
        latency_add(&g_diag.bus, (uint32_t)(locked - noticed));
        if (had_edge) latency_add(&g_diag.detect, (uint32_t)(noticed - edge));
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

// The settings were read at the very start (main, for the clock).
static void load_settings(bool stored) {
    if (!stored) {
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

// Core 1 (web server, login with Argon2 at about 4 KB of stack, settings,
// OTA) gets its own stack instead of the SDK's 2 KB default.
static uint32_t s_core1_stack[8192 / sizeof(uint32_t)];

int main(void) {
    // The clock first: UART, USB and SPI depend on it, and its setting is in
    // the flash (read through XIP, which needs no set-up).
    bool stored = settings_load(&g_settings_flash, &g_settings);
    guard_start(ota_trial_boot());
    clock_init(&g_settings);
    stdio_init_all();
    gpio_init(PICO_DEFAULT_LED_PIN);
    gpio_set_dir(PICO_DEFAULT_LED_PIN, GPIO_OUT);
#if PSHB_TIMING_GPIO
    for (uint pin = PSHB_TIMING_PIN_RX; pin <= PSHB_TIMING_PIN_FETCH; pin++) {
        gpio_init(pin);
        gpio_set_dir(pin, GPIO_OUT);
        gpio_put(pin, 0);
    }
#endif

    printf("ps-hid-bridge %u.%u.%u: boot%s%s\n", (PSHB_FW_VERSION >> 24) & 0xFF,
           (PSHB_FW_VERSION >> 16) & 0xFF, PSHB_FW_VERSION & 0xFFFF,
           PSHB_TEST_PATTERN ? " (test pattern)" : "",
           PSHB_PLAINTEXT_DEBUG ? " (PLAINTEXT DEBUG)" : "");
    printf("clock: CPU at %lu MHz%s\n", (unsigned long)(clock_cpu_hz() / 1000000),
           clock_fell_back() ? " (200 MHz did not get through its trial last time)" : "");

    input_state_init(&g_input);
    diag_init();
    bridge_init(&g_bridge, &g_input, get_rand_64(), get_rand_64(), time_us_64());
    g_bridge.allow_plaintext = PSHB_PLAINTEXT_DEBUG;
    g_bridge.clock_us = bridge_clock_us;
    critical_section_init(&s_cfg_lock);
    load_settings(stored);
    hid_out_init(&g_input);
    tud_init(0);

    crypto_select();
    crypto_help_init_core0();
    s_chip_ok = w5500_port_init(g_settings.spi_fast);
    guard_service();
    // Before the confirmation below: buying may erase the other slot.
    g_running_slot = ota_running_slot();
    g_bridge.no_partition_table = !ota_has_partition_table();
    // After an update the bootrom runs us "on trial": confirm only once the
    // essentials work, otherwise the next reset returns to the old firmware.
#if !PSHB_OTA_NO_CONFIRM
    if (s_chip_ok && ota_confirm_if_trial()) guard_confirmed();
#endif
    guard_service();
    if (s_chip_ok) {
        multicore_launch_core1_with_stack(core1_main, s_core1_stack, sizeof(s_core1_stack));
        guard_core1_launched();
    } else {
        printf("net: W5500 not responding\n");
        atomic_store(&g_net.state, NET_STATE_CHIP_ERROR);
    }

    while (true) {
        guard_service();
        clock_service();
        tud_task();
        if (s_chip_ok) net_service();
#if PSHB_TEST_PATTERN
        test_pattern_service();
#endif
        input_cfg_poll(&g_bridge);
        if (g_diag.reset_request) {
            diag_init();  // clears the request as well
        }
        usb_status_service();
        bridge_tick(&g_bridge, time_us_64());
        hid_out_service();
        led_service();
    }
}
