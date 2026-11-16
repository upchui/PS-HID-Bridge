// ps-hid-bridge-dspassthrough: a real DualSense on the USB host port
// (GP2/GP3) is passed through to the PlayStation, its authentication
// included -- the go/no-go test of the controller mode (README,
// "Controller mode"). No input from the app; everything is logged
// (netlog.h: UART0 and tools/boardlog.py).
//
// Core 1 runs the host (Pico-PIO-USB): it reads the DualSense's
// descriptors, receives its input reports, sends it the PlayStation's
// output reports and carries out the control requests relayed to it.
// Core 0 runs the device side towards the PlayStation with the DualSense's
// own descriptors, and a class driver of its own: every class request
// (feature reports -- the authentication -- and audio requests) goes to the
// real DualSense; a request with an answer is answered once the DualSense
// answered (the PlayStation's IN data stage waits until then, at most
// RELAY_TIMEOUT_US). Input reports go 1:1 to the PlayStation, output
// reports 1:1 to the DualSense.
//
// Variants (settings, "variant a|b", the test image starts again):
//  a: the DualSense's HID interface alone (renumbered to 0);
//  b: all its interfaces; audio is accepted and dropped (PIO-USB has no
//     isochronous transfers), audio class requests are relayed as well.
//
// Commands: status, pattern on|off (the left stick circles: proves that
// the PlayStation takes input that is not the DualSense's), variant a|b,
// restart (start the test image again), back (the normal firmware).
//
// Installed over the network (pshb-cli update --firmware --file ...), it
// runs on trial and never confirms itself: any reset or power cut returns
// to the normal firmware.
//
// LED: slow blink = no DualSense; fast blink = DualSense there, the
// PlayStation has not configured the board; steady = configured, reports
// flowing; three flashes = a relayed request failed in the last 5 s.
#include <math.h>
#include <stdio.h>
#include <string.h>

#include "hardware/adc.h"
#include "hardware/clocks.h"
#include "hardware/dma.h"
#include "hardware/gpio.h"
#include "hardware/sync.h"
#include "hardware/watchdog.h"
#include "pico/multicore.h"
#include "pico/platform.h"
#include "pico/stdlib.h"
#include "pico/util/queue.h"

#include "device/dcd.h"
#include "device/usbd_pvt.h"
#include "pio_usb.h"
#include "tusb.h"

#include "ds_desc.h"
#include "flash_store.h"
#include "netlog.h"
#include "ota.h"
#include "pshb_config.h"
#include "pshb_version.h"
#include "settings.h"
#include "w5500_port.h"

#define RHPORT_DEV        0
#define RELAY_TIMEOUT_US  500000u
#define RELAY_DATA_MAX    256u
#define CFG_MAX           CFG_TUH_ENUMERATION_BUFSIZE
#define REPORT_DESC_MAX   512u
#define STRINGS_MAX       4
#define STRING_MAX        128u
#define REPORT_MAX        64u
#define VSYS_ADC_PIN      29
#define VSYS_ADC_INPUT    3

static pio_usb_configuration_t s_host_cfg = PIO_USB_DEFAULT_CONFIG;
static pshb_settings_t s_settings;
static uint8_t s_variant;  // 0 = a, 1 = b

// ------------------------------------------------------- shared state

// What the DualSense looks like; filled by core 1, then published by
// raising s_ident_seq. Core 0 copies it while the device side is off.
typedef struct {
    uint8_t dev[18];
    uint8_t cfg[CFG_MAX];
    uint16_t cfg_len;
    uint8_t report[REPORT_DESC_MAX];
    uint16_t report_len;
    uint8_t hid_itf;  // the DualSense's own number of its HID interface
    uint8_t string_index[STRINGS_MAX];
    uint16_t string[STRINGS_MAX][STRING_MAX / 2];  // raw string descriptors
} ident_t;
static ident_t s_ident_new;
static volatile uint32_t s_ident_seq;
static volatile bool s_host_mounted;

// A control request relayed from the PlayStation to the DualSense.
typedef struct {
    uint32_t seq;
    tusb_control_request_t setup;  // as the DualSense must see it
    uint16_t len;                  // data for OUT requests
    uint8_t data[RELAY_DATA_MAX];
} relay_req_t;
typedef struct {
    uint32_t seq;
    bool ok;
    uint16_t len;
    uint32_t us;  // how long the DualSense took
    uint8_t data[RELAY_DATA_MAX];
} relay_res_t;
static queue_t s_req_q, s_res_q;

// Input reports DualSense -> PlayStation (the latest one), output reports
// PlayStation -> DualSense (in order).
static spin_lock_t *s_in_lock;
static uint8_t s_in_report[REPORT_MAX];
static uint16_t s_in_len;
static volatile uint32_t s_in_seq;
typedef struct {
    uint16_t len;
    uint8_t data[REPORT_MAX];
} out_report_t;
static queue_t s_out_q;

// Counters (logged every 10 s).
static volatile uint32_t s_core1_beat, s_from_ds, s_to_ps, s_from_ps, s_to_ds, s_out_dropped;

// ------------------------------------------------------------ core 1

static uint8_t s_daddr, s_hid_idx;
static volatile bool s_report_desc_ok;
static volatile uint8_t s_mounted_addr;
static relay_req_t s_cur;
static bool s_cur_busy, s_cur_waiting;
static uint64_t s_cur_start;
static tusb_control_request_t s_cur_setup;

void tuh_hid_mount_cb(uint8_t daddr, uint8_t idx, uint8_t const *desc, uint16_t len) {
    tuh_itf_info_t info;
    tuh_hid_itf_get_info(daddr, idx, &info);
    printf("host: HID interface %u, report descriptor %u bytes\n", info.desc.bInterfaceNumber, len);
    if (len == 0 || len > REPORT_DESC_MAX) {
        printf("host: report descriptor missing or too large -- not usable\n");
        return;
    }
    memcpy(s_ident_new.report, desc, len);
    s_ident_new.report_len = len;
    s_ident_new.hid_itf = info.desc.bInterfaceNumber;
    s_daddr = daddr;
    s_hid_idx = idx;
    s_report_desc_ok = true;
    tuh_hid_receive_report(daddr, idx);
}

void tuh_hid_umount_cb(uint8_t daddr, uint8_t idx) {
    (void)daddr;
    (void)idx;
    s_report_desc_ok = false;
}

void tuh_mount_cb(uint8_t daddr) {
    s_mounted_addr = daddr;  // the rest is read from the main loop
}

void tuh_umount_cb(uint8_t daddr) {
    printf("host: DualSense %u removed\n", daddr);
    s_mounted_addr = 0;
    s_host_mounted = false;
    s_daddr = 0;
}

void tuh_hid_report_received_cb(uint8_t daddr, uint8_t idx, uint8_t const *report, uint16_t len) {
    if (len > REPORT_MAX) len = REPORT_MAX;
    uint32_t irq = spin_lock_blocking(s_in_lock);
    memcpy(s_in_report, report, len);
    s_in_len = len;
    s_in_seq++;
    spin_unlock(s_in_lock, irq);
    s_from_ds++;
    tuh_hid_receive_report(daddr, idx);
}

static bool read_identity(uint8_t daddr) {
    ident_t *id = &s_ident_new;
    uint16_t vid = 0, pid = 0;
    tuh_vid_pid_get(daddr, &vid, &pid);
    printf("host: device %04x:%04x, %s speed\n", vid, pid,
           tuh_speed_get(daddr) == TUSB_SPEED_FULL ? "full" : "not full");
    if (vid != 0x054C || pid != 0x0CE6) printf("host: not a DualSense (054c:0ce6) -- passed through anyway\n");
    if (tuh_descriptor_get_device_sync(daddr, id->dev, 18) != XFER_RESULT_SUCCESS) return false;
    netlog_hex("host: device descriptor", id->dev, 18);
    if (tuh_descriptor_get_configuration_sync(daddr, 0, id->cfg, 9) != XFER_RESULT_SUCCESS) return false;
    uint16_t total = (uint16_t)(id->cfg[2] | (id->cfg[3] << 8));
    if (total > CFG_MAX) {
        printf("host: configuration descriptor of %u bytes is too large\n", total);
        return false;
    }
    if (tuh_descriptor_get_configuration_sync(daddr, 0, id->cfg, total) != XFER_RESULT_SUCCESS) return false;
    id->cfg_len = total;
    netlog_hex("host: configuration", id->cfg, total);
    // String descriptors: the language list and those the device descriptor names.
    const uint8_t want[STRINGS_MAX] = {0, id->dev[14], id->dev[15], id->dev[16]};
    for (int i = 0; i < STRINGS_MAX; i++) {
        id->string_index[i] = 0xFF;
        if (i > 0 && want[i] == 0) continue;
        memset(id->string[i], 0, sizeof id->string[i]);
        if (tuh_descriptor_get_string_sync(daddr, want[i], i == 0 ? 0 : 0x0409, id->string[i],
                                           sizeof id->string[i]) == XFER_RESULT_SUCCESS) {
            id->string_index[i] = want[i];
        }
    }
    return true;
}

static void relay_done(tuh_xfer_t *xfer) {
    relay_res_t res = {
        .seq = s_cur.seq,
        .ok = xfer->result == XFER_RESULT_SUCCESS,
        .len = (uint16_t)xfer->actual_len,
        .us = (uint32_t)(time_us_64() - s_cur_start),
    };
    if (res.len > RELAY_DATA_MAX) res.len = RELAY_DATA_MAX;
    memcpy(res.data, s_cur.data, res.len);
    if (!queue_try_add(&s_res_q, &res)) printf("relay: answer queue full\n");
    s_cur_busy = false;
}

static void relay_service_core1(void) {
    if (s_cur_busy) return;
    if (!s_cur_waiting) {
        if (!queue_try_remove(&s_req_q, &s_cur)) return;
        s_cur_waiting = true;
    }
    if (!s_host_mounted || s_daddr == 0) {
        relay_res_t res = {.seq = s_cur.seq, .ok = false};
        queue_try_add(&s_res_q, &res);
        s_cur_waiting = false;
        return;
    }
    s_cur_setup = s_cur.setup;
    tuh_xfer_t xfer = {
        .daddr = s_daddr,
        .ep_addr = 0,
        .setup = &s_cur_setup,
        .buffer = s_cur.data,
        .complete_cb = relay_done,
        .user_data = 0,
    };
    s_cur_start = time_us_64();
    s_cur_busy = true;
    if (tuh_control_xfer(&xfer)) {
        s_cur_waiting = false;
    } else {
        s_cur_busy = false;  // the control pipe is busy: try again next round
    }
}

static void out_service_core1(void) {
    if (!s_host_mounted || s_daddr == 0 || !tuh_hid_send_ready(s_daddr, s_hid_idx)) return;
    static out_report_t r;
    if (!queue_try_remove(&s_out_q, &r) || r.len == 0) return;
    if (tuh_hid_send_report(s_daddr, s_hid_idx, r.data[0], r.data + 1, (uint16_t)(r.len - 1))) s_to_ds++;
}

static void core1_main(void) {
    tuh_configure(PSHB_TUH_RHPORT, TUH_CFGID_RPI_PIO_USB_CONFIGURATION, &s_host_cfg);
    tuh_init(PSHB_TUH_RHPORT);
    printf("host: PIO-USB running on core 1 (D+ GP%u, D- GP%u)\n", PSHB_USBHOST_PIN_DP, PSHB_USBHOST_PIN_DP + 1);
    while (true) {
        tuh_task();
        uint8_t d = s_mounted_addr;
        if (d && !s_host_mounted && s_report_desc_ok) {
            if (read_identity(d)) {
                __dmb();
                s_host_mounted = true;
                s_ident_seq++;
                printf("host: DualSense ready\n");
            } else {
                printf("host: could not read the DualSense's descriptors\n");
                s_mounted_addr = 0;
            }
        }
        relay_service_core1();
        out_service_core1();
        s_core1_beat++;
    }
}

// ------------------------------------------------------------ core 0

// The device side's copy of the identity and the configuration it shows.
static ident_t s_ident;
static uint32_t s_ident_seen;
static uint8_t s_cfg[CFG_MAX];
static uint16_t s_cfg_len;
static bool s_dev_started, s_dev_on;
static volatile bool s_ps_configured, s_ps_suspended;
static bool s_pattern;
static uint64_t s_last_fail_us;

// The HID interface towards the PlayStation.
static uint8_t s_hid_itf_dev, s_ep_in, s_ep_out;
static uint16_t s_ep_in_size;
static uint8_t s_in_buf[REPORT_MAX], s_out_buf[REPORT_MAX];
static uint32_t s_in_sent_seq;

// Audio streaming interfaces (variant b): their alternate-setting-1
// endpoints, which only take data and give zeros.
typedef struct {
    uint8_t itf, alt, ep;
    uint16_t size;
    const uint8_t *ep_desc;
} audio_itf_t;
static audio_itf_t s_audio[2];
static int s_audio_n;
static uint8_t s_iso_out_buf[1024], s_iso_in_buf[1024];
static volatile uint32_t s_iso_bytes;

// The control request in progress (one at a time on endpoint 0).
static struct {
    bool pending;  // waiting for the DualSense's answer
    uint32_t seq;
    tusb_control_request_t req;  // as the PlayStation sent it
    uint64_t since;
    uint32_t setups;  // s_setups when it came
} s_ctl;
// SETUP packets received (USB interrupt): a newer one means the
// PlayStation has given up on the pending request, whatever it was.
static volatile uint32_t s_setups;
static uint32_t s_seq;
static uint8_t s_ctl_buf[RELAY_DATA_MAX];

static const char *hid_req_name(uint8_t r) {
    switch (r) {
    case 0x01: return "GET_REPORT";
    case 0x02: return "GET_IDLE";
    case 0x03: return "GET_PROTOCOL";
    case 0x09: return "SET_REPORT";
    case 0x0A: return "SET_IDLE";
    case 0x0B: return "SET_PROTOCOL";
    default: return NULL;
    }
}

static const char *audio_req_name(uint8_t r) {
    switch (r) {
    case 0x01: return "SET_CUR";
    case 0x81: return "GET_CUR";
    case 0x82: return "GET_MIN";
    case 0x83: return "GET_MAX";
    case 0x84: return "GET_RES";
    default: return NULL;
    }
}

static void describe_request(char *out, size_t cap, const tusb_control_request_t *r) {
    bool to_hid = r->bmRequestType_bit.recipient == TUSB_REQ_RCPT_INTERFACE && (r->wIndex & 0xFF) == s_hid_itf_dev;
    const char *name = to_hid ? hid_req_name(r->bRequest) : audio_req_name(r->bRequest);
    if (to_hid && name && (r->bRequest == 0x01 || r->bRequest == 0x09)) {
        static const char *const types[] = {"?", "input", "output", "feature"};
        snprintf(out, cap, "%s %s 0x%02x (%u)", name, types[(r->wValue >> 8) & 3], r->wValue & 0xFF, r->wLength);
    } else {
        snprintf(out, cap, "%s type %02x req %02x value %04x index %04x (%u)", name ? name : "request",
                 r->bmRequestType, r->bRequest, r->wValue, r->wIndex, r->wLength);
    }
}

// Relay a class request; returns false if the queue is full.
static bool relay(const tusb_control_request_t *req, const uint8_t *data, uint16_t len) {
    static relay_req_t r;
    r.seq = ++s_seq;
    r.setup = *req;
    // Variant a renumbered the HID interface: the DualSense knows its own.
    if (req->bmRequestType_bit.recipient == TUSB_REQ_RCPT_INTERFACE && (req->wIndex & 0xFF) == s_hid_itf_dev)
        r.setup.wIndex = (uint16_t)((req->wIndex & 0xFF00) | s_ident.hid_itf);
    r.len = len;
    if (len) memcpy(r.data, data, len);
    if (req->bmRequestType_bit.direction == TUSB_DIR_OUT) {
        char what[96];
        describe_request(what, sizeof what, req);
        printf("ctl #%lu %s\n", (unsigned long)r.seq, what);
        if (len) netlog_hex("ctl   data", data, len);
    }
    if (!queue_try_add(&s_req_q, &r)) {
        printf("ctl #%lu: relay queue full\n", (unsigned long)r.seq);
        return false;
    }
    return true;
}

// As TinyUSB does for a request it cannot handle.
static void ctl_stall(void) {
    dcd_edpt_stall(RHPORT_DEV, 0x00);
    dcd_edpt_stall(RHPORT_DEV, 0x80);
}

void tud_event_hook_cb(uint8_t rhport, uint32_t eventid, bool in_isr) {
    (void)rhport;
    (void)in_isr;
    if (eventid == DCD_EVENT_SETUP_RECEIVED) s_setups++;
}

static bool handle_standard(uint8_t rhport, const tusb_control_request_t *req) {
    uint8_t itf = req->wIndex & 0xFF;
    switch (req->bRequest) {
    case TUSB_REQ_GET_DESCRIPTOR: {
        uint8_t type = req->wValue >> 8;
        printf("ps5: GET_DESCRIPTOR 0x%02x of interface %u (%u)\n", type, itf, req->wLength);
        if (itf == s_hid_itf_dev && type == DS_DESC_HID + 1) {  // report descriptor
            return tud_control_xfer(rhport, req, s_ident.report, s_ident.report_len);
        }
        if (itf == s_hid_itf_dev && type == DS_DESC_HID) {
            const uint8_t *hid = ds_desc_find(s_cfg, s_cfg_len, itf, DS_DESC_HID);
            return hid && tud_control_xfer(rhport, req, (void *)hid, hid[0]);
        }
        return false;
    }
    case TUSB_REQ_SET_INTERFACE:
        for (int i = 0; i < s_audio_n; i++) {
            audio_itf_t *a = &s_audio[i];
            if (a->itf != itf) continue;
            a->alt = (uint8_t)req->wValue;
            printf("ps5: audio interface %u alternate setting %u\n", itf, a->alt);
            if (a->alt == 1 && a->ep_desc) {
                usbd_edpt_iso_activate(rhport, (const tusb_desc_endpoint_t *)a->ep_desc);
                if (a->ep & 0x80) usbd_edpt_xfer(rhport, a->ep, s_iso_in_buf, a->size);
                else usbd_edpt_xfer(rhport, a->ep, s_iso_out_buf, a->size);
            }
            return tud_control_status(rhport, req);
        }
        if (itf == s_hid_itf_dev) return tud_control_status(rhport, req);
        return false;
    case TUSB_REQ_GET_INTERFACE:
        for (int i = 0; i < s_audio_n; i++) {
            if (s_audio[i].itf == itf) return tud_control_xfer(rhport, req, &s_audio[i].alt, 1);
        }
        return false;
    default: return false;
    }
}

static bool dp_control(uint8_t rhport, uint8_t stage, tusb_control_request_t const *req) {
    if (stage == CONTROL_STAGE_SETUP) {
        if (s_ctl.pending) {
            printf("ctl #%lu: the PlayStation gave up on it\n", (unsigned long)s_ctl.seq);
            s_ctl.pending = false;
        }
        if (req->bmRequestType_bit.type == TUSB_REQ_TYPE_STANDARD) return handle_standard(rhport, req);
        if (req->bmRequestType_bit.type != TUSB_REQ_TYPE_CLASS || req->wLength > RELAY_DATA_MAX) {
            printf("ps5: request type %02x req %02x not passed on\n", req->bmRequestType, req->bRequest);
            return false;
        }
        if (req->bmRequestType_bit.direction == TUSB_DIR_IN) {
            // Answered once the DualSense answered (relay_service).
            if (!relay(req, NULL, 0)) return false;
            s_ctl.pending = true;
            s_ctl.seq = s_seq;
            s_ctl.req = *req;
            s_ctl.since = time_us_64();
            s_ctl.setups = s_setups;
            return true;
        }
        if (req->wLength == 0) return relay(req, NULL, 0) && tud_control_status(rhport, req);
        return tud_control_xfer(rhport, req, s_ctl_buf, req->wLength);  // the data first
    }
    if (stage == CONTROL_STAGE_DATA && req->bmRequestType_bit.type == TUSB_REQ_TYPE_CLASS &&
        req->bmRequestType_bit.direction == TUSB_DIR_OUT) {
        return relay(req, s_ctl_buf, req->wLength);
    }
    return true;
}

// Called once per interface of the configuration (SET_CONFIGURATION).
static uint16_t dp_open(uint8_t rhport, tusb_desc_interface_t const *itf, uint16_t max_len) {
    uint16_t span = ds_desc_interface_span((const uint8_t *)itf, max_len);
    if (span == 0) return 0;
    const uint8_t *p = (const uint8_t *)itf, *end = p + span;
    if (itf->bInterfaceClass == DS_CLASS_HID) {
        s_hid_itf_dev = itf->bInterfaceNumber;
        for (; p < end; p += p[0]) {
            if (p[1] != DS_DESC_ENDPOINT) continue;
            const tusb_desc_endpoint_t *ep = (const tusb_desc_endpoint_t *)p;
            if (!usbd_edpt_open(rhport, ep)) return 0;
            if (ep->bEndpointAddress & 0x80) {
                s_ep_in = ep->bEndpointAddress;
                s_ep_in_size = tu_edpt_packet_size(ep);
            } else {
                s_ep_out = ep->bEndpointAddress;
                usbd_edpt_xfer(rhport, s_ep_out, s_out_buf, sizeof s_out_buf);
            }
        }
        return span;
    }
    if (itf->bInterfaceClass == DS_CLASS_AUDIO) {
        // Audio control: nothing to open. Streaming: remember the
        // endpoint of alternate setting 1 and set aside its buffer.
        uint8_t alt = 0;
        for (; p < end; p += p[0]) {
            if (p[1] == DS_DESC_INTERFACE) alt = p[3];
            if (p[1] != DS_DESC_ENDPOINT || alt == 0 || s_audio_n >= 2) continue;
            const tusb_desc_endpoint_t *ep = (const tusb_desc_endpoint_t *)p;
            audio_itf_t *a = &s_audio[s_audio_n++];
            a->itf = itf->bInterfaceNumber;
            a->alt = 0;
            a->ep = ep->bEndpointAddress;
            a->size = tu_edpt_packet_size(ep);
            if (a->size > sizeof s_iso_out_buf) a->size = sizeof s_iso_out_buf;
            a->ep_desc = p;
            usbd_edpt_iso_alloc(rhport, a->ep, a->size);
        }
        return span;
    }
    printf("ps5: interface %u of class %02x not handled\n", itf->bInterfaceNumber, itf->bInterfaceClass);
    return 0;
}

static bool dp_xfer(uint8_t rhport, uint8_t ep_addr, xfer_result_t result, uint32_t n) {
    (void)result;
    if (ep_addr == s_ep_in) {
        s_to_ps++;
        return true;
    }
    if (ep_addr == s_ep_out) {
        out_report_t r = {.len = (uint16_t)n};
        if (r.len > REPORT_MAX) r.len = REPORT_MAX;
        memcpy(r.data, s_out_buf, r.len);
        if (queue_try_add(&s_out_q, &r)) s_from_ps++;
        else s_out_dropped++;
        usbd_edpt_xfer(rhport, s_ep_out, s_out_buf, sizeof s_out_buf);
        return true;
    }
    for (int i = 0; i < s_audio_n; i++) {
        audio_itf_t *a = &s_audio[i];
        if (a->ep != ep_addr) continue;
        s_iso_bytes += n;
        if (a->alt == 1) usbd_edpt_xfer(rhport, a->ep, (a->ep & 0x80) ? s_iso_in_buf : s_iso_out_buf, a->size);
        return true;
    }
    return false;
}

static void dp_init(void) {
}

static bool dp_deinit(void) {
    return true;
}

static void dp_reset(uint8_t rhport) {
    (void)rhport;
    s_ep_in = s_ep_out = 0;
    s_audio_n = 0;
    s_ctl.pending = false;
    s_ps_configured = false;
}

static const usbd_class_driver_t s_driver = {
    .name = "dspass",
    .init = dp_init,
    .deinit = dp_deinit,
    .reset = dp_reset,
    .open = dp_open,
    .control_xfer_cb = dp_control,
    .xfer_cb = dp_xfer,
    .sof = NULL,
};

usbd_class_driver_t const *usbd_app_driver_get_cb(uint8_t *count) {
    *count = 1;
    return &s_driver;
}

uint8_t const *tud_descriptor_device_cb(void) {
    printf("ps5: GET_DESCRIPTOR device\n");
    return s_ident.dev;
}

uint8_t const *tud_descriptor_configuration_cb(uint8_t index) {
    printf("ps5: GET_DESCRIPTOR configuration %u\n", index);
    return index == 0 ? s_cfg : NULL;
}

uint16_t const *tud_descriptor_string_cb(uint8_t index, uint16_t langid) {
    for (int i = 0; i < STRINGS_MAX; i++) {
        if (s_ident.string_index[i] == index) {
            printf("ps5: GET_DESCRIPTOR string %u (language %04x)\n", index, langid);
            return s_ident.string[i];
        }
    }
    printf("ps5: GET_DESCRIPTOR string %u (language %04x): unknown\n", index, langid);
    return NULL;
}

void tud_mount_cb(void) {
    s_ps_configured = true;
    printf("ps5: configured (SET_CONFIGURATION)\n");
}

void tud_umount_cb(void) {
    s_ps_configured = false;
    printf("ps5: unconfigured\n");
}

void tud_suspend_cb(bool remote_wakeup_en) {
    s_ps_suspended = true;
    printf("ps5: bus suspended (rest mode?), remote wakeup %s\n", remote_wakeup_en ? "allowed" : "not allowed");
}

void tud_resume_cb(void) {
    s_ps_suspended = false;
    printf("ps5: bus resumed\n");
}

// Copy the published identity and build the configuration (device side off).
static bool take_identity(void) {
    memcpy(&s_ident, &s_ident_new, sizeof s_ident);
    s_audio_n = 0;
    if (s_variant == 0) {
        s_cfg_len = ds_desc_single_interface(s_ident.cfg, s_ident.cfg_len, s_ident.hid_itf, s_cfg, sizeof s_cfg);
        if (s_cfg_len == 0) {
            printf("dev: could not build the HID-only configuration\n");
            return false;
        }
    } else {
        memcpy(s_cfg, s_ident.cfg, s_ident.cfg_len);
        s_cfg_len = s_ident.cfg_len;
    }
    printf("dev: variant %c, configuration %u bytes, HID interface %u (DualSense: %u)\n", 'a' + s_variant, s_cfg_len,
           s_variant == 0 ? 0 : s_ident.hid_itf, s_ident.hid_itf);
    netlog_hex("dev: configuration", s_cfg, s_cfg_len);
    return true;
}

static void device_service(void) {
    if (s_host_mounted && s_ident_seq != s_ident_seen && !s_dev_on) {
        s_ident_seen = s_ident_seq;
        if (!take_identity()) return;
        if (!s_dev_started) {
            tud_init(RHPORT_DEV);  // connects
            s_dev_started = true;
        } else {
            tud_connect();
        }
        s_dev_on = true;
        printf("dev: connected to the PlayStation side\n");
    } else if (!s_host_mounted && s_dev_on) {
        tud_disconnect();
        s_dev_on = false;
        printf("dev: disconnected (no DualSense)\n");
    }
    if (s_dev_started) tud_task();
}

static void relay_service(void) {
    relay_res_t res;
    while (queue_try_remove(&s_res_q, &res)) {
        if (s_ctl.pending && s_setups != s_ctl.setups) {
            printf("ctl #%lu: the PlayStation sent a new request meanwhile\n", (unsigned long)s_ctl.seq);
            s_ctl.pending = false;
        }
        bool answer = s_ctl.pending && res.seq == s_ctl.seq;
        if (!res.ok) s_last_fail_us = time_us_64();
        if (answer) {
            char what[96];
            describe_request(what, sizeof what, &s_ctl.req);
            printf("ctl #%lu %s -> %s %u bytes, %lu us\n", (unsigned long)res.seq, what, res.ok ? "ok" : "FAILED",
                   res.len, (unsigned long)res.us);
            if (res.ok && res.len) netlog_hex("ctl   data", res.data, res.len);
            s_ctl.pending = false;
            if (res.ok) {
                memcpy(s_ctl_buf, res.data, res.len);
                tud_control_xfer(RHPORT_DEV, &s_ctl.req, s_ctl_buf, res.len);
            } else {
                ctl_stall();
            }
        } else {
            printf("ctl #%lu -> %s, %lu us\n", (unsigned long)res.seq, res.ok ? "ok" : "FAILED",
                   (unsigned long)res.us);
        }
    }
    if (s_ctl.pending && time_us_64() - s_ctl.since > RELAY_TIMEOUT_US) {
        printf("ctl #%lu: no answer from the DualSense in time, stalled\n", (unsigned long)s_ctl.seq);
        s_ctl.pending = false;
        s_last_fail_us = time_us_64();
        ctl_stall();
    }
}

static void input_service(void) {
    if (!s_dev_on || !s_ps_configured || s_ep_in == 0 || s_in_seq == s_in_sent_seq) return;
    if (usbd_edpt_busy(RHPORT_DEV, s_ep_in)) return;
    uint16_t len;
    uint32_t irq = spin_lock_blocking(s_in_lock);
    len = s_in_len;
    memcpy(s_in_buf, s_in_report, len);
    s_in_sent_seq = s_in_seq;
    spin_unlock(s_in_lock, irq);
    if (len > s_ep_in_size) len = s_ep_in_size;
    if (s_pattern && len >= 3 && s_in_buf[0] == 0x01) {
        float a = (float)(time_us_64() % 2000000u) / 2000000.0f * 6.2831853f;
        s_in_buf[1] = (uint8_t)(128.0f + 100.0f * cosf(a));
        s_in_buf[2] = (uint8_t)(128.0f + 100.0f * sinf(a));
    }
    usbd_edpt_xfer(RHPORT_DEV, s_ep_in, s_in_buf, len);
}

static void stats_service(void) {
    static uint64_t next;
    static uint32_t from_ds, to_ps, from_ps, to_ds;
    uint64_t now = time_us_64();
    if (now < next) return;
    next = now + 10u * 1000u * 1000u;
    printf("rate: DualSense %lu/s -> PlayStation %lu/s; PlayStation %lu/s -> DualSense %lu/s%s; audio %lu B; %s%s\n",
           (unsigned long)((s_from_ds - from_ds) / 10), (unsigned long)((s_to_ps - to_ps) / 10),
           (unsigned long)((s_from_ps - from_ps) / 10), (unsigned long)((s_to_ds - to_ds) / 10),
           s_out_dropped ? " (some dropped)" : "", (unsigned long)s_iso_bytes,
           s_ps_configured ? "configured" : "not configured", s_ps_suspended ? ", suspended" : "");
    from_ds = s_from_ds;
    to_ps = s_to_ps;
    from_ps = s_from_ps;
    to_ds = s_to_ds;
    uint32_t mv = (uint32_t)adc_read() * 3u * 3300u / 4096u;
    if (mv < 4300) printf("power: VSYS %lu mV -- low\n", (unsigned long)mv);
}

static void led_service(void) {
    uint64_t now = time_us_64();
    uint32_t t = (uint32_t)(now / 1000);
    bool on;
    if (s_last_fail_us && now - s_last_fail_us < 5000000) {
        uint32_t p = t % 2000;
        on = p < 600 && (p % 200) < 100;
    } else if (s_ps_configured && s_host_mounted) {
        on = true;
    } else if (s_host_mounted) {
        on = (t % 200) < 100;
    } else {
        on = (t % 1000) < 500;
    }
    gpio_put(PICO_DEFAULT_LED_PIN, on);
}

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
    for (;;) tight_loop_contents();  // the main loop would keep feeding the watchdog
}

static void on_command(const char *cmd) {
    if (strcmp(cmd, "pattern on") == 0 || strcmp(cmd, "pattern off") == 0) {
        s_pattern = cmd[9] == 'n';
        printf("pattern %s\n", s_pattern ? "on: the left stick circles" : "off");
    } else if (strcmp(cmd, "variant a") == 0 || strcmp(cmd, "variant b") == 0) {
        s_settings.ds_test_variant = cmd[8] == 'b';
        bool ok = settings_save(&g_settings_flash, &s_settings);
        printf("variant %c %s, starting again\n", cmd[8], ok ? "saved" : "NOT saved");
        restart_test_image();
    } else if (strcmp(cmd, "restart") == 0) {
        restart_test_image();
    } else if (strcmp(cmd, "back") == 0) {
        back_to_firmware();
    } else if (strcmp(cmd, "status") == 0) {
        printf("status: variant %c, DualSense %s, PlayStation %s%s, pattern %s, relayed %lu\n", 'a' + s_variant,
               s_host_mounted ? "ready" : "missing", s_ps_configured ? "configured" : "not configured",
               s_ps_suspended ? " (suspended)" : "", s_pattern ? "on" : "off", (unsigned long)s_seq);
    } else {
        printf("unknown command (status, pattern on|off, variant a|b, restart, back)\n");
    }
}

int main(void) {
    bool stored = settings_load(&g_settings_flash, &s_settings);
    bool by_watchdog = watchdog_caused_reboot();
    netlog_init();
    s_variant = s_settings.ds_test_variant ? 1 : 0;
    printf("ps-hid-bridge %u.%u.%u dspassthrough: boot%s\n", (PSHB_FW_VERSION >> 24) & 0xFF,
           (PSHB_FW_VERSION >> 16) & 0xFF, PSHB_FW_VERSION & 0xFFFF,
           by_watchdog ? " (restarted by the watchdog)" : "");
    printf("chip: RP2350 revision %u, CPU at %lu MHz; variant %c (%s)\n", rp2350_chip_version(),
           (unsigned long)(clock_get_hz(clk_sys) / 1000000), 'a' + s_variant,
           s_variant ? "all interfaces" : "HID interface only");
    printf("image: slot %s, %s\n", ota_running_slot(),
           ota_trial_boot() ? "on trial (a reset returns to the normal firmware; \"back\" does that)"
                            : "not on trial (installed via BOOTSEL)");

    gpio_init(PICO_DEFAULT_LED_PIN);
    gpio_set_dir(PICO_DEFAULT_LED_PIN, GPIO_OUT);
    adc_init();
    adc_gpio_init(VSYS_ADC_PIN);
    adc_select_input(VSYS_ADC_INPUT);

    s_in_lock = spin_lock_init(spin_lock_claim_unused(true));
    queue_init(&s_req_q, sizeof(relay_req_t), 8);
    queue_init(&s_res_q, sizeof(relay_res_t), 8);
    queue_init(&s_out_q, sizeof(out_report_t), 8);

    bool chip_ok = w5500_port_init(s_settings.spi_fast);
    if (!stored) printf("settings: none stored, network by DHCP\n");
    netlog_start_network(&s_settings, chip_ok);
    netlog_on_command(on_command);

    int ch = dma_claim_unused_channel(true);
    dma_channel_unclaim((uint)ch);
    s_host_cfg.pin_dp = PSHB_USBHOST_PIN_DP;
    s_host_cfg.pinout = PIO_USB_PINOUT_DPDM;
    s_host_cfg.tx_ch = (uint8_t)ch;
    static uint32_t core1_stack[8192 / sizeof(uint32_t)];
    multicore_launch_core1_with_stack(core1_main, core1_stack, sizeof core1_stack);

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
        device_service();
        relay_service();
        input_service();
        netlog_service();
        stats_service();
        led_service();
    }
}
