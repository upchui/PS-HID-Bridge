// ps-hid-bridge-usbhosttest: checks the USB host port of the controller mode
// (USB-A socket, D+ GP2, D- GP3) without the PlayStation. Whatever is
// plugged in there is enumerated by Pico-PIO-USB on core 1 and described in
// the log (netlog.h: UART0 and tools/boardlog.py): speed, IDs, descriptors,
// for a DualSense also some feature reports and its sticks and buttons.
// Core 0 runs the log, the LED, the supply voltage and the line state.
//
// LED: slow blink = nothing attached; fast blink = something attached, not
// (yet) enumerated; steady = enumerated and sending reports; three flashes =
// a problem (see the log), e.g. D+/D- look swapped.
//
// Commands (tools/boardlog.py IP -c CMD): "status", "swap" (start again with
// D+ and D- swapped, for a socket wired the other way round), "restart"
// (start this test image again), "back" (restart into the normal firmware).
//
// Installed over the network (pshb-cli update --firmware --file ...), it
// runs on trial and never confirms itself: any reset or power cut returns
// to the normal firmware.
#include <stdio.h>
#include <string.h>

#include "hardware/adc.h"
#include "hardware/clocks.h"
#include "hardware/dma.h"
#include "hardware/gpio.h"
#include "hardware/watchdog.h"
#include "pico/multicore.h"
#include "pico/platform.h"
#include "pico/stdlib.h"

#include "pio_usb.h"
#include "tusb.h"

#include "flash_store.h"
#include "netlog.h"
#include "ota.h"
#include "pshb_config.h"
#include "pshb_version.h"
#include "settings.h"
#include "w5500_port.h"

#define ORIENT_SCRATCH 1
#define ORIENT_SWAPPED 0x50415753u  // "SWAP"
#define VSYS_ADC_PIN   29
#define VSYS_ADC_INPUT 3
#define VBUS_SENSE_PIN 24

static pio_usb_configuration_t s_host_cfg = PIO_USB_DEFAULT_CONFIG;
static bool s_swapped;
static uint s_pin_dp, s_pin_dm;

// Shared between the cores (core 1 writes, core 0 reads for LED/status).
static volatile uint32_t s_core1_beat;
static volatile uint8_t s_dev_addr;
static volatile uint64_t s_last_report_us;
static volatile uint32_t s_reports;
static volatile bool s_problem;

// ------------------------------------------------------------- core 1

static volatile uint8_t s_new_dev;
static uint8_t s_hid_addr, s_hid_idx;
static int s_feature_step;  // 0 = none, 1.. = next feature report to read
static bool s_feature_busy;
static uint8_t s_feature_buf[64];
static const uint8_t s_feature_ids[] = {0x05, 0x09, 0x20};  // calibration, pairing (MAC), firmware

// DualSense input report 0x01 over USB: sticks, triggers, buttons.
static volatile uint8_t s_input[11];
static volatile bool s_input_new;

static const char *speed_text(tusb_speed_t s) {
    switch (s) {
    case TUSB_SPEED_LOW: return "low";
    case TUSB_SPEED_FULL: return "full";
    case TUSB_SPEED_HIGH: return "high";
    default: return "unknown";
    }
}

static void print_string(uint8_t daddr, const char *what, uint8_t index) {
    if (index == 0) return;
    uint16_t buf[64];
    if (tuh_descriptor_get_string_sync(daddr, index, 0x0409, buf, sizeof buf) != XFER_RESULT_SUCCESS) {
        printf("host: %s string %u: no answer\n", what, index);
        return;
    }
    // UTF-16LE after the 2-byte header.
    char text[64];
    uint32_t n = ((buf[0] & 0xFF) - 2) / 2, k = 0;
    for (uint32_t i = 0; i < n && k + 1 < sizeof text; i++) text[k++] = buf[1 + i] < 128 ? (char)buf[1 + i] : '?';
    text[k] = '\0';
    printf("host: %s: \"%s\"\n", what, text);
}

static void list_descriptors(const uint8_t *d, uint16_t len) {
    static const char *const ep_type[] = {"control", "isochronous", "bulk", "interrupt"};
    for (uint16_t off = 0; off + 2 <= len && d[off] >= 2; off += d[off]) {
        const uint8_t *p = d + off;
        if (p[1] == TUSB_DESC_INTERFACE && p[0] >= 9) {
            printf("host:   interface %u alt %u: class %02x/%02x/%02x, %u endpoints\n", p[2], p[3], p[5], p[6],
                   p[7], p[4]);
        } else if (p[1] == TUSB_DESC_ENDPOINT && p[0] >= 7) {
            printf("host:     endpoint %02x %s, %u bytes, interval %u\n", p[2], ep_type[p[3] & 3],
                   (unsigned)(p[4] | (p[5] << 8)) & 0x7FF, p[6]);
        } else if (p[1] == HID_DESC_TYPE_HID && p[0] >= 9) {
            printf("host:     HID descriptor: report descriptor %u bytes\n", (unsigned)(p[7] | (p[8] << 8)));
        }
    }
}

static void describe(uint8_t daddr) {
    uint16_t vid = 0, pid = 0;
    tuh_vid_pid_get(daddr, &vid, &pid);
    printf("host: device %u enumerated: %04x:%04x, %s speed%s\n", daddr, vid, pid, speed_text(tuh_speed_get(daddr)),
           vid == 0x054C && pid == 0x0CE6 ? " (DualSense)" : "");
    static uint8_t buf[CFG_TUH_ENUMERATION_BUFSIZE];
    if (tuh_descriptor_get_device_sync(daddr, buf, 18) == XFER_RESULT_SUCCESS) {
        netlog_hex("host: device descriptor", buf, 18);
        uint8_t imanu = buf[14], iprod = buf[15], iser = buf[16];
        print_string(daddr, "manufacturer", imanu);
        print_string(daddr, "product", iprod);
        print_string(daddr, "serial", iser);
    } else {
        printf("host: device descriptor: no answer\n");
    }
    if (tuh_descriptor_get_configuration_sync(daddr, 0, buf, 9) == XFER_RESULT_SUCCESS) {
        uint16_t total = (uint16_t)(buf[2] | (buf[3] << 8));
        printf("host: configuration descriptor: %u bytes, %u interfaces, %s, %u mA\n", total, buf[4],
               buf[7] & 0x40 ? "self-powered" : "bus-powered", buf[8] * 2u);
        if (total > sizeof buf) total = sizeof buf;
        if (tuh_descriptor_get_configuration_sync(daddr, 0, buf, total) == XFER_RESULT_SUCCESS) {
            netlog_hex("host: configuration", buf, total);
            list_descriptors(buf, total);
        }
    } else {
        printf("host: configuration descriptor: no answer\n");
    }
}

void tuh_mount_cb(uint8_t daddr) {
    s_dev_addr = daddr;
    s_new_dev = daddr;  // described from the main loop (blocking requests)
}

void tuh_umount_cb(uint8_t daddr) {
    printf("host: device %u removed\n", daddr);
    s_dev_addr = 0;
    s_feature_step = 0;
    s_feature_busy = false;
}

void tuh_hid_mount_cb(uint8_t daddr, uint8_t idx, uint8_t const *desc, uint16_t len) {
    printf("hid: interface %u (protocol %u) mounted, report descriptor %u bytes%s\n", idx,
           tuh_hid_interface_protocol(daddr, idx), len,
           len == 0 ? " (missing: larger than the enumeration buffer?)" : "");
    if (len) netlog_hex("hid: report descriptor", desc, len);
    s_hid_addr = daddr;
    s_hid_idx = idx;
    s_feature_step = 1;
    s_feature_busy = false;
    if (!tuh_hid_receive_report(daddr, idx)) printf("hid: cannot start receiving reports\n");
}

void tuh_hid_umount_cb(uint8_t daddr, uint8_t idx) {
    printf("hid: interface %u of device %u gone\n", idx, daddr);
}

void tuh_hid_report_received_cb(uint8_t daddr, uint8_t idx, uint8_t const *report, uint16_t len) {
    s_last_report_us = time_us_64();
    s_reports++;
    if (len >= sizeof s_input && report[0] == 0x01) {
        bool changed = false;
        for (uint32_t i = 0; i < sizeof s_input; i++) {
            if (i == 7) continue;  // sequence number: changes every time
            if (s_input[i] != report[i]) changed = true;
        }
        for (uint32_t i = 0; i < sizeof s_input; i++) s_input[i] = report[i];
        if (changed) s_input_new = true;
    } else if (s_reports <= 3) {
        netlog_hex("hid: report", report, len);  // not what a DualSense sends: show it
    }
    tuh_hid_receive_report(daddr, idx);
}

void tuh_hid_get_report_complete_cb(uint8_t daddr, uint8_t idx, uint8_t report_id, uint8_t report_type,
                                    uint16_t len) {
    (void)daddr;
    (void)idx;
    (void)report_type;
    char what[40];
    snprintf(what, sizeof what, "hid: feature 0x%02x (%u bytes)", report_id, len);
    if (len) netlog_hex(what, s_feature_buf, len);
    else printf("%s: failed\n", what);
    s_feature_busy = false;
    s_feature_step++;
}

static void host_service(void) {
    uint8_t d = s_new_dev;
    if (d) {
        s_new_dev = 0;
        describe(d);
    }
    if (s_feature_step > 0 && !s_feature_busy && s_dev_addr) {
        if (s_feature_step > (int)sizeof s_feature_ids) {
            s_feature_step = 0;
            printf("hid: feature reports read -- move the sticks and press buttons\n");
        } else {
            uint8_t id = s_feature_ids[s_feature_step - 1];
            memset(s_feature_buf, 0, sizeof s_feature_buf);
            s_feature_busy = tuh_hid_get_report(s_hid_addr, s_hid_idx, id, HID_REPORT_TYPE_FEATURE, s_feature_buf,
                                                sizeof s_feature_buf);
            if (!s_feature_busy) {
                printf("hid: feature 0x%02x: request not accepted\n", id);
                s_feature_step++;
            }
        }
    }
}

static void core1_main(void) {
    tuh_configure(PSHB_TUH_RHPORT, TUH_CFGID_RPI_PIO_USB_CONFIGURATION, &s_host_cfg);
    tuh_init(PSHB_TUH_RHPORT);
    printf("host: PIO-USB running on core 1 (D+ GP%u, D- GP%u)\n", s_pin_dp, s_pin_dm);
    while (true) {
        tuh_task();
        host_service();
        s_core1_beat++;
    }
}

// ------------------------------------------------------------- core 0

// Commands "restart" and "back". A test image installed over the network
// runs on trial and never confirms itself: a plain reset returns to the
// normal firmware, a restart on trial keeps the test image.
static void restart_test_image(void) {
    uint64_t until = time_us_64() + 300000;
    while (time_us_64() < until) netlog_service();  // let the log go out
    ota_restart_on_trial();
}

static void back_to_firmware(void) {
    printf("back to the normal firmware\n");
    uint64_t until = time_us_64() + 300000;
    while (time_us_64() < until) netlog_service();
    watchdog_reboot(0, 0, 10);
}

static void on_command(const char *cmd) {
    if (strcmp(cmd, "swap") == 0) {
        watchdog_hw->scratch[ORIENT_SCRATCH] = s_swapped ? 0 : ORIENT_SWAPPED;
        printf("starting again with D+ on GP%u and D- on GP%u\n", s_pin_dm, s_pin_dp);
        restart_test_image();
    } else if (strcmp(cmd, "restart") == 0) {
        restart_test_image();
    } else if (strcmp(cmd, "back") == 0) {
        back_to_firmware();
    } else if (strcmp(cmd, "status") == 0) {
        printf("status: device %u, %lu reports, D+ GP%u, D- GP%u%s\n", s_dev_addr, (unsigned long)s_reports,
               s_pin_dp, s_pin_dm, s_swapped ? " (swapped)" : "");
    } else {
        printf("unknown command (status, swap, restart, back)\n");
    }
}

// Before anything is enumerated: which line is pulled up (full speed: D+,
// low speed: D-). Logged when it stays the same for 3 samples.
static void line_service(void) {
    static uint64_t next;
    static int last = -1, candidate = -1, same;
    uint64_t now = time_us_64();
    if (now < next || s_dev_addr) return;
    next = now + 50000;
    int state = (gpio_get(s_pin_dp) ? 1 : 0) | (gpio_get(s_pin_dm) ? 2 : 0);
    if (state != candidate) {
        candidate = state;
        same = 0;
        return;
    }
    if (++same < 3 || state == last) return;
    last = state;
    s_problem = state == 2 || state == 3;
    switch (state) {
    case 0: printf("line: D+ and D- low: nothing attached\n"); break;
    case 1: printf("line: D+ (GP%u) high: a full-speed device is attached\n", s_pin_dp); break;
    case 2:
        printf("line: D- (GP%u) high: a low-speed device -- or D+ and D- are swapped (a DualSense is full speed; "
               "fix the wiring or send \"swap\")\n", s_pin_dm);
        break;
    default: printf("line: D+ and D- both high: wiring fault?\n"); break;
    }
}

static void power_service(void) {
    static uint64_t next_sample, next_report;
    static uint32_t min_mv = UINT32_MAX, max_mv;
    uint64_t now = time_us_64();
    if (now < next_sample) return;
    next_sample = now + 10000;
    uint32_t mv = (uint32_t)adc_read() * 3u * 3300u / 4096u;  // VSYS / 3 on GP29
    if (mv < min_mv) min_mv = mv;
    if (mv > max_mv) max_mv = mv;
    if (now >= next_report) {
        next_report = now + 10u * 1000u * 1000u;
        printf("power: VSYS %lu.%02lu..%lu.%02lu V, VBUS %s\n", (unsigned long)(min_mv / 1000),
               (unsigned long)(min_mv % 1000 / 10), (unsigned long)(max_mv / 1000),
               (unsigned long)(max_mv % 1000 / 10), gpio_get(VBUS_SENSE_PIN) ? "present" : "missing");
        if (min_mv < 4300) printf("power: VSYS dropped below 4.3 V -- the port may not supply enough current\n");
        min_mv = UINT32_MAX;
        max_mv = 0;
    }
}

static void input_service(void) {
    static uint64_t next_print, next_rate;
    static uint32_t last_reports;
    uint64_t now = time_us_64();
    if (s_input_new && now >= next_print) {
        next_print = now + 200000;  // 5 per second at most
        s_input_new = false;
        uint8_t r[sizeof s_input];
        for (uint32_t i = 0; i < sizeof r; i++) r[i] = s_input[i];
        printf("ds: LX %3u LY %3u RX %3u RY %3u L2 %3u R2 %3u buttons %02x %02x %02x\n", r[1], r[2], r[3], r[4],
               r[5], r[6], r[8], r[9], r[10]);
    }
    if (now >= next_rate) {
        next_rate = now + 10u * 1000u * 1000u;
        uint32_t n = s_reports;
        if (s_dev_addr) printf("hid: %lu reports/s\n", (unsigned long)((n - last_reports) / 10));
        last_reports = n;
    }
}

static void led_service(void) {
    uint32_t t = (uint32_t)(time_us_64() / 1000);
    bool on;
    if (s_problem) {
        uint32_t p = t % 2000;
        on = p < 600 && (p % 200) < 100;  // three flashes
    } else if (s_dev_addr && time_us_64() - s_last_report_us < 500000) {
        on = true;
    } else if (s_dev_addr || gpio_get(s_pin_dp) || gpio_get(s_pin_dm)) {
        on = (t % 200) < 100;
    } else {
        on = (t % 1000) < 500;
    }
    gpio_put(PICO_DEFAULT_LED_PIN, on);
}

int main(void) {
    pshb_settings_t settings;
    bool stored = settings_load(&g_settings_flash, &settings);
    bool by_watchdog = watchdog_caused_reboot();
    netlog_init();
    s_swapped = watchdog_hw->scratch[ORIENT_SCRATCH] == ORIENT_SWAPPED;
    s_pin_dp = s_swapped ? PSHB_USBHOST_PIN_DP + 1 : PSHB_USBHOST_PIN_DP;
    s_pin_dm = s_swapped ? PSHB_USBHOST_PIN_DP : PSHB_USBHOST_PIN_DP + 1;

    printf("ps-hid-bridge %u.%u.%u usbhosttest: boot%s\n", (PSHB_FW_VERSION >> 24) & 0xFF,
           (PSHB_FW_VERSION >> 16) & 0xFF, PSHB_FW_VERSION & 0xFFFF,
           by_watchdog ? " (restarted by the watchdog)" : "");
    printf("chip: RP2350 revision %u, CPU at %lu MHz\n", rp2350_chip_version(),
           (unsigned long)(clock_get_hz(clk_sys) / 1000000));
    printf("host port: D+ GP%u, D- GP%u%s\n", s_pin_dp, s_pin_dm, s_swapped ? " (swapped by command)" : "");
    printf("image: slot %s, %s\n", ota_running_slot(),
           ota_trial_boot() ? "on trial (a reset returns to the normal firmware; \"back\" does that)"
                            : "not on trial (installed via BOOTSEL)");

    gpio_init(PICO_DEFAULT_LED_PIN);
    gpio_set_dir(PICO_DEFAULT_LED_PIN, GPIO_OUT);
    gpio_init(VBUS_SENSE_PIN);
    adc_init();
    adc_gpio_init(VSYS_ADC_PIN);
    adc_select_input(VSYS_ADC_INPUT);

    bool chip_ok = w5500_port_init(settings.spi_fast);
    if (!stored) printf("settings: none stored, network by DHCP\n");
    netlog_start_network(&settings, chip_ok);
    netlog_on_command(on_command);

    // PIO-USB claims its DMA channel by number: one the W5500 left free.
    int ch = dma_claim_unused_channel(true);
    dma_channel_unclaim((uint)ch);
    s_host_cfg.pin_dp = (uint8_t)s_pin_dp;
    s_host_cfg.pinout = s_swapped ? PIO_USB_PINOUT_DMDP : PIO_USB_PINOUT_DPDM;
    s_host_cfg.tx_ch = (uint8_t)ch;
    // Nested blocking descriptor requests: more than the default 2 KB stack.
    static uint32_t core1_stack[8192 / sizeof(uint32_t)];
    multicore_launch_core1_with_stack(core1_main, core1_stack, sizeof core1_stack);

    // Restart if core 1 (the host) stops: the log is kept.
    // Core 1 (the host) stopped: start the test image again (the log is
    // kept). If core 0 hangs, the watchdog resets the board, which returns
    // to the normal firmware.
    watchdog_enable(4000, true);
    uint32_t beat = 0;
    uint64_t beat_at = time_us_64();
    while (true) {
        uint64_t now = time_us_64();
        if (s_core1_beat != beat) {
            beat = s_core1_beat;
            beat_at = now;
        }
        if (now - beat_at > 2500000) {
            printf("core 1 (USB host) hangs: starting the test image again\n");
            restart_test_image();
        }
        watchdog_update();
        netlog_service();
        line_service();
        power_service();
        input_service();
        led_service();
    }
}
