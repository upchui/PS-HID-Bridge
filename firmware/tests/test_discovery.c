#include "discovery.h"
#include "mini_test.h"

static const pshb_discovery_info_t k_info = {
    .mac = {0x02, 0x68, 0x9d, 0x96, 0xc3, 0x3d},
    .udp_port = 45045,
    .fw_version = 0x01000042u,  // 1.0.66
    .flags = PSHB_DISCOVERY_KEY_SET,
    .name = "ps-hid-bridge-96c33d",
};

// The same bytes are checked by the app (desktop/core/src/discover.rs).
static const uint8_t k_answer[PSHB_DISCOVERY_LEN] = {
    'P', 'S', 'H', 'B', 'D', 'I', 'S', 'C', 0x01, 0x02, 0x01, 0x00,
    0x02, 0x68, 0x9d, 0x96, 0xc3, 0x3d, 0xf5, 0xaf, 0x42, 0x00, 0x00, 0x01,
    'p', 's', '-', 'h', 'i', 'd', '-', 'b', 'r', 'i', 'd', 'g', 'e', '-', '9', '6', 'c', '3', '3', 'd',
};

static void request(uint8_t req[PSHB_DISCOVERY_LEN]) {
    memset(req, 0, PSHB_DISCOVERY_LEN);
    memcpy(req, PSHB_DISCOVERY_MAGIC, 8);
    req[8] = PSHB_DISCOVERY_VERSION;
    req[9] = PSHB_DISCOVERY_REQUEST;
}

TEST(discovery_answers_a_request) {
    uint8_t req[PSHB_DISCOVERY_LEN], out[PSHB_DISCOVERY_LEN + 8];
    request(req);
    memset(out, 0xAA, sizeof(out));
    CHECK_EQ_U(pshb_discovery_answer(req, sizeof(req), &k_info, out, sizeof(out)), PSHB_DISCOVERY_LEN);
    CHECK_MEM(out, k_answer, PSHB_DISCOVERY_LEN);
    CHECK_EQ_U(out[PSHB_DISCOVERY_LEN], 0xAA);  // nothing written past the answer
}

TEST(discovery_ignores_anything_else) {
    uint8_t req[PSHB_DISCOVERY_LEN + 1], out[PSHB_DISCOVERY_LEN];
    request(req);
    // Shorter or longer than 64 bytes: the answer would be larger, or it is no request.
    CHECK_EQ_U(pshb_discovery_answer(req, PSHB_DISCOVERY_LEN - 1, &k_info, out, sizeof(out)), 0);
    CHECK_EQ_U(pshb_discovery_answer(req, PSHB_DISCOVERY_LEN + 1, &k_info, out, sizeof(out)), 0);
    CHECK_EQ_U(pshb_discovery_answer(req, PSHB_DISCOVERY_LEN, &k_info, out, sizeof(out) - 1), 0);

    request(req);
    req[0] = 'X';
    CHECK_EQ_U(pshb_discovery_answer(req, PSHB_DISCOVERY_LEN, &k_info, out, sizeof(out)), 0);
    request(req);
    req[8] = 2;
    CHECK_EQ_U(pshb_discovery_answer(req, PSHB_DISCOVERY_LEN, &k_info, out, sizeof(out)), 0);
    request(req);
    req[9] = PSHB_DISCOVERY_ANSWER;  // another board's answer is no request
    CHECK_EQ_U(pshb_discovery_answer(req, PSHB_DISCOVERY_LEN, &k_info, out, sizeof(out)), 0);
}

TEST(discovery_name_stays_terminated) {
    pshb_discovery_info_t info = k_info;
    memset(info.name, 'x', sizeof(info.name));  // no NUL at all
    uint8_t req[PSHB_DISCOVERY_LEN], out[PSHB_DISCOVERY_LEN];
    request(req);
    CHECK_EQ_U(pshb_discovery_answer(req, sizeof(req), &info, out, sizeof(out)), PSHB_DISCOVERY_LEN);
    CHECK_EQ_U(out[24 + PSHB_DISCOVERY_NAME_LEN - 2], 'x');
    CHECK_EQ_U(out[24 + PSHB_DISCOVERY_NAME_LEN - 1], 0);
}

TEST(discovery_limit_per_second) {
    pshb_discovery_limit_t l = {0};
    uint64_t t = 5000000;
    for (int i = 0; i < PSHB_DISCOVERY_MAX_PER_S; i++) CHECK(pshb_discovery_allow(&l, t + (uint64_t)i));
    CHECK(!pshb_discovery_allow(&l, t + 999999));
    CHECK(pshb_discovery_allow(&l, t + 1000000));  // a new second
}

int main(void) {
    RUN(discovery_answers_a_request);
    RUN(discovery_ignores_anything_else);
    RUN(discovery_name_stays_terminated);
    RUN(discovery_limit_per_second);
    return TEST_SUMMARY();
}
