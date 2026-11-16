#include "ds_desc.h"
#include "mini_test.h"

// Shaped like the DualSense's: audio control (0), speaker/haptics and
// microphone streaming (1, 2: alternate settings 0 and 1), HID (3).
static const uint8_t cfg[] = {
    0x09, 0x02, 0x00, 0x00, 0x04, 0x01, 0x00, 0xC0, 0xFA,  // configuration (wTotalLength unused here)
    // 0: audio control
    0x09, 0x04, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00,
    0x0A, 0x24, 0x01, 0x00, 0x01, 0x49, 0x00, 0x02, 0x01, 0x02,
    0x0C, 0x24, 0x02, 0x01, 0x01, 0x01, 0x06, 0x04, 0x33, 0x00, 0x00, 0x00,
    // 1: speaker, alt 0 and 1
    0x09, 0x04, 0x01, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00,
    0x09, 0x04, 0x01, 0x01, 0x01, 0x01, 0x02, 0x00, 0x00,
    0x07, 0x24, 0x01, 0x01, 0x01, 0x01, 0x00,
    0x09, 0x05, 0x01, 0x09, 0x88, 0x01, 0x01, 0x00, 0x00,
    0x07, 0x25, 0x01, 0x00, 0x00, 0x00, 0x00,
    // 2: microphone, alt 0 and 1
    0x09, 0x04, 0x02, 0x00, 0x00, 0x01, 0x02, 0x00, 0x00,
    0x09, 0x04, 0x02, 0x01, 0x01, 0x01, 0x02, 0x00, 0x00,
    0x09, 0x05, 0x82, 0x05, 0xC4, 0x00, 0x01, 0x00, 0x00,
    // 3: HID
    0x09, 0x04, 0x03, 0x00, 0x02, 0x03, 0x00, 0x00, 0x00,
    0x09, 0x21, 0x11, 0x01, 0x00, 0x01, 0x22, 0x11, 0x01,
    0x07, 0x05, 0x84, 0x03, 0x40, 0x00, 0x04,
    0x07, 0x05, 0x03, 0x03, 0x40, 0x00, 0x04,
};
#define LEN ((uint16_t)sizeof cfg)
#define OFF_SPK 40   // interface 1, alt 0
#define OFF_MIC 81   // interface 2, alt 0
#define OFF_HID 108  // interface 3

TEST(finds_interfaces_by_class) {
    CHECK_EQ_U(cfg[OFF_SPK + 1], 0x04);
    CHECK_EQ_U(cfg[OFF_MIC + 2], 2);
    CHECK_EQ_U(cfg[OFF_HID + 2], 3);
    CHECK_EQ_U(ds_desc_find_class(cfg, LEN, DS_CLASS_HID), 3);
    CHECK_EQ_U(ds_desc_find_class(cfg, LEN, DS_CLASS_AUDIO), 0);
    CHECK(ds_desc_find_class(cfg, LEN, 0x08) == -1);
}

TEST(span_covers_every_alternate_setting) {
    CHECK_EQ_U(ds_desc_interface_span(cfg + OFF_SPK, (uint16_t)(LEN - OFF_SPK)), OFF_MIC - OFF_SPK);
    CHECK_EQ_U(ds_desc_interface_span(cfg + OFF_MIC, (uint16_t)(LEN - OFF_MIC)), OFF_HID - OFF_MIC);
    CHECK_EQ_U(ds_desc_interface_span(cfg + OFF_HID, (uint16_t)(LEN - OFF_HID)), LEN - OFF_HID);
    CHECK_EQ_U(ds_desc_interface_span(cfg + 9, (uint16_t)(LEN - 9)), OFF_SPK - 9);
    CHECK_EQ_U(ds_desc_interface_span(cfg, LEN), 0);  // not an interface
}

TEST(finds_descriptors_of_alternate_setting_0_only) {
    const uint8_t *hid = ds_desc_find(cfg, LEN, 3, DS_DESC_HID);
    CHECK(hid != NULL);
    if (hid) CHECK_EQ_U(hid[7] | (hid[8] << 8), 273);
    CHECK(ds_desc_find(cfg, LEN, 3, DS_DESC_ENDPOINT) == cfg + OFF_HID + 18);
    // The speaker's endpoint belongs to alternate setting 1.
    CHECK(ds_desc_find(cfg, LEN, 1, DS_DESC_ENDPOINT) == NULL);
    CHECK(ds_desc_find(cfg, LEN, 7, DS_DESC_INTERFACE) == NULL);
}

TEST(single_interface_configuration) {
    uint8_t out[64];
    uint16_t n = ds_desc_single_interface(cfg, LEN, 3, out, sizeof out);
    CHECK_EQ_U(n, 9 + LEN - OFF_HID);
    CHECK_EQ_U(out[2] | (out[3] << 8), n);  // wTotalLength
    CHECK_EQ_U(out[4], 1);                  // bNumInterfaces
    CHECK_EQ_U(out[9 + 2], 0);              // renumbered
    CHECK_MEM(out + 18, cfg + OFF_HID + 9, LEN - OFF_HID - 9);
    CHECK_EQ_U(ds_desc_single_interface(cfg, LEN, 3, out, 20), 0);  // does not fit
    CHECK_EQ_U(ds_desc_single_interface(cfg, LEN, 9, out, sizeof out), 0);
}

TEST(broken_descriptors_are_refused) {
    uint8_t bad[sizeof cfg];
    memcpy(bad, cfg, sizeof cfg);
    bad[OFF_HID + 18] = 0;  // an endpoint descriptor of length 0
    CHECK_EQ_U(ds_desc_interface_span(bad + OFF_HID, (uint16_t)(LEN - OFF_HID)), 0);
    uint8_t out[64];
    CHECK_EQ_U(ds_desc_single_interface(bad, LEN, 3, out, sizeof out), 0);
    bad[OFF_HID + 18] = 0x40;  // runs past the end
    CHECK_EQ_U(ds_desc_interface_span(bad + OFF_HID, (uint16_t)(LEN - OFF_HID)), 0);
}

int main(void) {
    RUN(finds_interfaces_by_class);
    RUN(span_covers_every_alternate_setting);
    RUN(finds_descriptors_of_alternate_setting_0_only);
    RUN(single_interface_configuration);
    RUN(broken_descriptors_are_refused);
    return TEST_SUMMARY();
}
