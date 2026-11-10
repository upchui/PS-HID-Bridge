// Parses the report descriptors with a small HID item parser and checks
// that the declared layout matches the report sizes the firmware sends.
#include "hid_desc.h"
#include "mini_test.h"

typedef struct {
    int in_bits, out_bits, feature_bits;
    int depth, errors, report_ids;
    int xy_items;           // relative 2-axis input items (X/Y)
    long xy_min, xy_max;    // logical range seen at that item
} hid_summary_t;

static long item_value(const uint8_t *p, int size, int is_signed) {
    unsigned long v = 0;
    for (int i = 0; i < size; i++) v |= (unsigned long)p[i] << (8 * i);
    if (is_signed && size > 0 && size < 4 && (v & (1ul << (8 * size - 1)))) {
        v |= ~0ul << (8 * size);
    }
    return (long)v;
}

static hid_summary_t parse(const uint8_t *d, size_t n) {
    hid_summary_t s = {0};
    long report_size = 0, report_count = 0, log_min = 0, log_max = 0;
    size_t i = 0;
    while (i < n) {
        uint8_t prefix = d[i];
        if (prefix == 0xFE) {  // long items are not used by us
            s.errors++;
            break;
        }
        int size = prefix & 3;
        if (size == 3) size = 4;
        int type = (prefix >> 2) & 3;
        int tag = prefix >> 4;
        if (i + 1 + (size_t)size > n) {
            s.errors++;
            break;
        }
        const uint8_t *data = &d[i + 1];
        if (type == 0) {  // main
            long bits = report_size * report_count;
            switch (tag) {
            case 0x8:
                s.in_bits += (int)bits;
                // Data, Variable, Relative with two fields: the X/Y item.
                if (report_count == 2 && (item_value(data, size, 0) & 0x07) == 0x06) {
                    s.xy_items++;
                    s.xy_min = log_min;
                    s.xy_max = log_max;
                }
                break;
            case 0x9: s.out_bits += (int)bits; break;
            case 0xB: s.feature_bits += (int)bits; break;
            case 0xA: s.depth++; break;
            case 0xC:
                if (--s.depth < 0) s.errors++;
                break;
            default: s.errors++;
            }
        } else if (type == 1) {  // global
            switch (tag) {
            case 0x1: log_min = item_value(data, size, 1); break;
            case 0x2: log_max = item_value(data, size, 1); break;
            case 0x7: report_size = item_value(data, size, 0); break;
            case 0x8: s.report_ids++; break;
            case 0x9: report_count = item_value(data, size, 0); break;
            default: break;
            }
        } else if (type == 3) {
            s.errors++;  // reserved
        }
        i += 1 + (size_t)size;
    }
    return s;
}

TEST(keyboard_layout) {
    hid_summary_t s = parse(pshb_hid_kbd_desc, sizeof(pshb_hid_kbd_desc));
    CHECK_EQ_I(s.errors, 0);
    CHECK_EQ_I(s.depth, 0);
    CHECK_EQ_I(s.report_ids, 0);  // boot compatible: no report ID
    CHECK_EQ_I(s.in_bits, PSHB_KBD_REPORT_LEN * 8);
    CHECK_EQ_I(s.out_bits, 8);
    CHECK_EQ_I(s.feature_bits, 0);
}

TEST(mouse_layout) {
    hid_summary_t s = parse(pshb_hid_mouse_desc, sizeof(pshb_hid_mouse_desc));
    CHECK_EQ_I(s.errors, 0);
    CHECK_EQ_I(s.depth, 0);
    CHECK_EQ_I(s.report_ids, 0);
    CHECK_EQ_I(s.in_bits, PSHB_MOUSE_REPORT_LEN * 8);
    CHECK_EQ_I(s.out_bits, 0);
    CHECK_EQ_I(s.xy_items, 1);
    CHECK_EQ_I(s.xy_max, PSHB_MOUSE_XY_MAX);
    CHECK_EQ_I(s.xy_min, -PSHB_MOUSE_XY_MAX);
}

TEST(descriptor_starts_like_boot_devices) {
    // Usage Page (Generic Desktop), Usage (Keyboard / Mouse), Collection (Application)
    static const uint8_t kbd_head[] = {0x05, 0x01, 0x09, 0x06, 0xA1, 0x01};
    static const uint8_t mouse_head[] = {0x05, 0x01, 0x09, 0x02, 0xA1, 0x01};
    CHECK_MEM(pshb_hid_kbd_desc, kbd_head, sizeof(kbd_head));
    CHECK_MEM(pshb_hid_mouse_desc, mouse_head, sizeof(mouse_head));
    CHECK_EQ_U(pshb_hid_kbd_desc[PSHB_HID_KBD_DESC_LEN - 1], 0xC0);
    CHECK_EQ_U(pshb_hid_mouse_desc[PSHB_HID_MOUSE_DESC_LEN - 1], 0xC0);
}

int main(void) {
    RUN(keyboard_layout);
    RUN(mouse_layout);
    RUN(descriptor_starts_like_boot_devices);
    return TEST_SUMMARY();
}
