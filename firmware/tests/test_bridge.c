// Scenario tests of the input path state machine (encrypted protocol v1).
#include "bridge.h"
#include "settings.h"
#include "mini_test.h"
#include "proto.h"

static const uint8_t KEY[32] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
                                17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32};
static const uint8_t OTHER_KEY[32] = {0xEE};
#define SESSION 0xABCDEF0011223344ull
#define APP_IP 0xC0A80164u  // 192.168.1.100
#define T0 1000000ull

typedef struct {
    input_state_t input;
    bridge_t b;
    uint8_t reply[PSHB_MAX_PACKET_LEN];
    size_t reply_len;
} fixture_t;

static void setup(fixture_t *f) {
    input_state_init(&f->input);
    uint8_t kbd[PSHB_KBD_REPORT_LEN], mouse[PSHB_MOUSE_REPORT_LEN];
    input_state_take_kbd_report(&f->input, kbd);
    input_state_take_mouse_report(&f->input, mouse, false);
    bridge_init(&f->b, &f->input, SESSION, 0xFFFFFFFFFFFFFFF0ull, T0);
    bridge_set_key(&f->b, KEY);
}

static size_t make_input(uint8_t *out, const uint8_t *key, uint8_t type, uint64_t counter,
                         uint64_t session, uint8_t flags, uint8_t key0, int16_t dx) {
    pshb_input_t in = {.session_id = session, .flags = flags, .keys = {key0}, .dx = dx};
    uint8_t payload[PSHB_INPUT_PAYLOAD_LEN];
    pshb_input_encode(payload, &in);
    return proto_seal(key, PSHB_DIR_APP_TO_BOARD, type, counter, payload, sizeof(payload), out,
                      PSHB_MAX_PACKET_LEN);
}

static size_t make_ping_type(uint8_t *out, const uint8_t *key, uint8_t type, uint64_t counter,
                             uint64_t t, uint32_t seq) {
    pshb_ping_t p = {.t_app_us = t, .seq = seq};
    uint8_t payload[PSHB_PING_PAYLOAD_LEN];
    pshb_ping_encode(payload, &p);
    return proto_seal(key, PSHB_DIR_APP_TO_BOARD, type, counter, payload, sizeof(payload), out,
                      PSHB_MAX_PACKET_LEN);
}

static size_t make_ping(uint8_t *out, const uint8_t *key, uint64_t counter, uint64_t t, uint32_t seq) {
    return make_ping_type(out, key, PSHB_TYPE_PING, counter, t, seq);
}

static void feed(fixture_t *f, const uint8_t *pkt, size_t len, uint64_t now) {
    f->reply_len = bridge_handle_packet(&f->b, pkt, len, APP_IP, now, f->reply, sizeof(f->reply));
}

static bool decode_pong(fixture_t *f, pshb_pong_t *pong) {
    pshb_header_t h;
    uint8_t payload[PSHB_PONG_PAYLOAD_LEN];
    if (!f->reply_len || !proto_check(f->reply, f->reply_len, &h) || h.type != PSHB_TYPE_PONG) return false;
    if (!proto_decrypt(KEY, PSHB_DIR_BOARD_TO_APP, f->reply, f->reply_len, payload)) return false;
    pshb_pong_decode(payload, pong);
    return true;
}

TEST(ping_gets_encrypted_pong) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    feed(&f, pkt, make_ping(pkt, KEY, 100, 777, 5), T0 + 5000);
    pshb_pong_t pong;
    CHECK(decode_pong(&f, &pong));
    CHECK_EQ_U(pong.t_app_us, 777);
    CHECK_EQ_U(pong.seq, 5);
    CHECK_EQ_U(pong.session_id, SESSION);
    CHECK_EQ_U(pong.uptime_ms, 5);
    CHECK_EQ_U(pong.rx_ok, 1);
    CHECK(pong.status & PSHB_STATUS_FAILSAFE);  // no input yet
    // The board counter wraps within 62 bits.
    pshb_header_t h;
    proto_check(f.reply, f.reply_len, &h);
    CHECK(h.counter <= PSHB_TX_COUNTER_MASK);
}

static uint64_t s_clock = 0;
static uint64_t test_clock(void) {
    return s_clock;
}

TEST(timed_ping_gets_time_stamps) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    // Every PONG says that the board answers PING_TIMED.
    feed(&f, pkt, make_ping(pkt, KEY, 100, 777, 5), T0 + 5000);
    pshb_pong_t pong;
    CHECK(decode_pong(&f, &pong));
    CHECK(pong.status & PSHB_STATUS_TIMED_PING);

    // Arrival as the platform noted it, answer time from its clock.
    f.b.rx_at_us = T0 + 6000;
    f.b.clock_us = test_clock;
    s_clock = T0 + 6040;
    feed(&f, pkt, make_ping_type(pkt, KEY, PSHB_TYPE_PING_TIMED, 101, 888, 6), T0 + 6020);
    pshb_header_t h;
    CHECK(proto_check(f.reply, f.reply_len, &h));
    CHECK_EQ_U(h.type, PSHB_TYPE_PONG_TIMED);
    CHECK_EQ_U(f.reply_len, PSHB_HDR_LEN + PSHB_PONG_TIMED_PAYLOAD_LEN + PSHB_TAG_LEN);
    uint8_t payload[PSHB_PONG_TIMED_PAYLOAD_LEN];
    CHECK(proto_decrypt(KEY, PSHB_DIR_BOARD_TO_APP, f.reply, f.reply_len, payload));
    pshb_pong_timed_decode(payload, &pong);
    CHECK_EQ_U(pong.t_app_us, 888);
    CHECK_EQ_U(pong.seq, 6);
    CHECK_EQ_U(pong.session_id, SESSION);
    CHECK_EQ_U(pong.board_rx_us, T0 + 6000);
    CHECK_EQ_U(pong.board_tx_us, T0 + 6040);
    CHECK(f.b.rx_at_us == 0);  // good for one datagram only

    // Without a noted arrival or a clock: the time of the call.
    f.b.clock_us = NULL;
    feed(&f, pkt, make_ping_type(pkt, KEY, PSHB_TYPE_PING_TIMED, 102, 999, 7), T0 + 7000);
    CHECK(proto_decrypt(KEY, PSHB_DIR_BOARD_TO_APP, f.reply, f.reply_len, payload));
    pshb_pong_timed_decode(payload, &pong);
    CHECK_EQ_U(pong.board_rx_us, T0 + 7000);
    CHECK_EQ_U(pong.board_tx_us, T0 + 7000);

    // A PONG_TIMED sent to the board is no request.
    uint32_t malformed = f.b.stats.rx_malformed;
    uint8_t answer[PSHB_PONG_TIMED_PAYLOAD_LEN] = {0};
    size_t n = proto_seal(KEY, PSHB_DIR_APP_TO_BOARD, PSHB_TYPE_PONG_TIMED, 103, answer, sizeof(answer), pkt,
                          sizeof(pkt));
    feed(&f, pkt, n, T0 + 8000);
    CHECK_EQ_U(f.reply_len, 0);
    CHECK_EQ_U(f.b.stats.rx_malformed, malformed + 1);
}

TEST(pong_reports_missing_partition_table) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    pshb_pong_t pong;
    feed(&f, pkt, make_ping(pkt, KEY, 100, 1, 1), T0);
    CHECK(decode_pong(&f, &pong));
    CHECK(!(pong.status & PSHB_STATUS_NO_PARTITION_TABLE));
    f.b.no_partition_table = true;
    feed(&f, pkt, make_ping(pkt, KEY, 101, 2, 2), T0 + 1000);
    CHECK(decode_pong(&f, &pong));
    CHECK(pong.status & PSHB_STATUS_NO_PARTITION_TABLE);
}

TEST(input_is_applied) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    feed(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_INPUT, 10, SESSION, 0, 0x04, 25), T0);
    CHECK_EQ_U(f.reply_len, 0);
    CHECK_EQ_U(f.b.stats.rx_ok, 1);
    CHECK(input_state_kbd_pending(&f.input));
    CHECK_EQ_U(f.input.keys[0], 0x04);
    CHECK_EQ_I(f.input.dx, 25);
    CHECK(!f.b.failsafe_active);
}

TEST(replay_and_old_counter_rejected) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    size_t n = make_input(pkt, KEY, PSHB_TYPE_INPUT, 50, SESSION, 0, 0, 1);
    feed(&f, pkt, n, T0);
    feed(&f, pkt, n, T0 + 1);  // exact replay
    CHECK_EQ_U(f.b.stats.rx_replay, 1);
    feed(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_INPUT, 49, SESSION, 0, 0, 1), T0 + 2);
    CHECK_EQ_U(f.b.stats.rx_replay, 2);
    CHECK_EQ_I(f.input.dx, 1);  // applied once only
    CHECK_EQ_U(f.b.stats.rx_ok, 1);
}

TEST(forged_packet_does_not_advance_counter) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    // Attacker without the key sends a huge counter.
    feed(&f, pkt, make_input(pkt, OTHER_KEY, PSHB_TYPE_INPUT, UINT64_MAX - 1, SESSION, 0, 0x04, 0), T0);
    CHECK_EQ_U(f.b.stats.rx_auth_fail, 1);
    CHECK_EQ_U(f.b.last_rx_counter, 0);
    // The real app keeps working.
    feed(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_INPUT, 60, SESSION, 0, 0x05, 0), T0 + 1);
    CHECK_EQ_U(f.b.stats.rx_ok, 1);
    CHECK_EQ_U(f.input.keys[0], 0x05);
}

TEST(tampered_packet_rejected) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    size_t n = make_input(pkt, KEY, PSHB_TYPE_INPUT, 70, SESSION, 0, 0x04, 0);
    pkt[15] ^= 0x10;
    feed(&f, pkt, n, T0);
    CHECK_EQ_U(f.b.stats.rx_auth_fail, 1);
    CHECK(!input_state_kbd_pending(&f.input));
}

TEST(no_key_rejects_everything) {
    fixture_t f;
    setup(&f);
    bridge_set_key(&f.b, NULL);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    feed(&f, pkt, make_ping(pkt, KEY, 1, 0, 0), T0);
    CHECK_EQ_U(f.reply_len, 0);
    CHECK_EQ_U(f.b.stats.rx_auth_fail, 1);
}

TEST(wrong_session_gets_rate_limited_hint) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    feed(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_INPUT, 100, 0x1234, 0, 0x04, 0), T0);
    CHECK_EQ_U(f.b.stats.rx_replay, 1);
    CHECK(!input_state_kbd_pending(&f.input));
    pshb_pong_t pong;
    CHECK(decode_pong(&f, &pong));
    CHECK_EQ_U(pong.t_app_us, 0);
    CHECK_EQ_U(pong.session_id, SESSION);

    feed(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_INPUT, 101, 0x1234, 0, 0x04, 0), T0 + 50000);
    CHECK_EQ_U(f.reply_len, 0);  // within 100 ms: no second hint
    feed(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_INPUT, 102, 0x1234, 0, 0x04, 0), T0 + 100000);
    CHECK(f.reply_len > 0);
    CHECK_EQ_U(f.b.stats.rx_replay, 3);
}

static void set_filter(fixture_t *f, uint32_t net, unsigned prefix) {
    f->b.allowed_mask = settings_prefix_mask(prefix);
    f->b.allowed_net = net & f->b.allowed_mask;
}

TEST(sender_filter) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    uint64_t ctr = 1;
    // APP_IP is 192.168.1.100
    static const struct {
        uint32_t net;
        unsigned prefix;
        bool pass;
    } cases[] = {
        {0x0A000001u, 32, false},  // 10.0.0.1
        {APP_IP, 32, true},
        {0x0A000000u, 8, false},   // 10.0.0.0/8
        {0xC0A80100u, 24, true},   // 192.168.1.0/24
        {0xC0A80200u, 24, false},  // 192.168.2.0/24
        {0xC0A80000u, 16, true},   // 192.168.0.0/16
        {0xC0A80166u, 31, false},  // 192.168.1.102/31: .102 and .103
        {0xC0A80165u, 31, true},   // .101 with host bit cleared: .100 and .101
        {0, 0, true},              // any sender
    };
    uint32_t filtered = 0;
    for (size_t i = 0; i < sizeof(cases) / sizeof(cases[0]); i++) {
        set_filter(&f, cases[i].net, cases[i].prefix);
        f.reply_len = 0;
        feed(&f, pkt, make_ping(pkt, KEY, ctr++, 0, 0), T0);
        if (cases[i].pass) {
            CHECK(f.reply_len > 0);
        } else {
            CHECK_EQ_U(f.reply_len, 0);
            filtered++;
        }
        CHECK_EQ_U(f.b.stats.rx_filtered, filtered);
    }
}

TEST(malformed_and_plaintext_rejected) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    size_t n = make_input(pkt, KEY, PSHB_TYPE_INPUT, 5, SESSION, 0, 0x04, 0);
    feed(&f, pkt, n - 1, T0);  // truncated
    pkt[0] = PSHB_PROTO_VERSION_PLAIN;
    feed(&f, pkt, PSHB_HDR_LEN + PSHB_INPUT_PAYLOAD_LEN, T0);  // plaintext not allowed
    // A PONG addressed to the board is never valid.
    uint8_t payload[PSHB_PONG_PAYLOAD_LEN] = {0};
    n = proto_seal(KEY, PSHB_DIR_APP_TO_BOARD, PSHB_TYPE_PONG, 6, payload, sizeof(payload), pkt, sizeof(pkt));
    feed(&f, pkt, n, T0);
    CHECK_EQ_U(f.b.stats.rx_malformed, 3);
    CHECK_EQ_U(f.b.stats.rx_ok, 0);
}

TEST(failsafe_releases_after_timeout) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    uint8_t r[PSHB_KBD_REPORT_LEN];
    feed(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_INPUT, 10, SESSION, 0, 0x04, 0), T0);
    input_state_take_kbd_report(&f.input, r);
    CHECK_EQ_U(r[2], 0x04);

    bridge_tick(&f.b, T0 + PSHB_FAILSAFE_US_DEFAULT);
    CHECK(!f.b.failsafe_active);
    CHECK(!input_state_kbd_pending(&f.input));

    // A heartbeat keeps it alive.
    feed(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_HEARTBEAT, 11, SESSION, 0, 0x04, 0), T0 + 200000);
    bridge_tick(&f.b, T0 + 400000);
    CHECK(!f.b.failsafe_active);

    bridge_tick(&f.b, T0 + 200000 + PSHB_FAILSAFE_US_DEFAULT + 1);
    CHECK(f.b.failsafe_active);
    CHECK(input_state_kbd_pending(&f.input));
    input_state_take_kbd_report(&f.input, r);
    CHECK_EQ_U(r[2], 0x00);

    // The next valid packet restores the state.
    feed(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_INPUT, 12, SESSION, 0, 0x04, 0), T0 + 600000);
    CHECK(!f.b.failsafe_active);
    input_state_take_kbd_report(&f.input, r);
    CHECK_EQ_U(r[2], 0x04);
}

TEST(pings_do_not_feed_failsafe) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    feed(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_INPUT, 10, SESSION, 0, 0x04, 0), T0);
    feed(&f, pkt, make_ping(pkt, KEY, 11, 0, 0), T0 + 200000);
    bridge_tick(&f.b, T0 + PSHB_FAILSAFE_US_DEFAULT + 1);
    CHECK(f.b.failsafe_active);
}

TEST(release_all_flag) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN];
    uint8_t r[PSHB_KBD_REPORT_LEN];
    feed(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_INPUT, 10, SESSION, 0, 0x04, 0), T0);
    input_state_take_kbd_report(&f.input, r);
    feed(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_INPUT, 11, SESSION, PSHB_FLAG_RELEASE_ALL, 0x09, 5), T0);
    input_state_take_kbd_report(&f.input, r);
    CHECK_EQ_U(r[2], 0x00);
    CHECK_EQ_I(f.input.dx, 0);
}

static void feed_from(fixture_t *f, const uint8_t *pkt, size_t len, uint32_t ip, uint16_t port, uint64_t now) {
    f->b.rx_port = port;
    f->reply_len = bridge_handle_packet(&f->b, pkt, len, ip, now, f->reply, sizeof(f->reply));
}

#define NEW_IP 0xC0A80132u  // 192.168.1.50

TEST(moved_notice_goes_to_the_last_app) {
    fixture_t f;
    setup(&f);
    uint8_t pkt[PSHB_MAX_PACKET_LEN], out[PSHB_MAX_PACKET_LEN];
    uint32_t to_ip = 0;
    uint16_t to_port = 0;
    // No app yet: nobody to tell.
    CHECK_EQ_U(bridge_build_moved(&f.b, NEW_IP, 45045, T0, out, sizeof(out), &to_ip, &to_port), 0);
    // A forged packet does not make its sender the app.
    feed_from(&f, pkt, make_input(pkt, OTHER_KEY, PSHB_TYPE_INPUT, 10, SESSION, 0, 0, 0), 0x0A000001u, 4000, T0);
    CHECK_EQ_U(bridge_build_moved(&f.b, NEW_IP, 45045, T0, out, sizeof(out), &to_ip, &to_port), 0);
    // An authentic packet does, also a ping; its port is for this datagram only.
    feed_from(&f, pkt, make_ping(pkt, KEY, 11, 0, 1), APP_IP, 50123, T0 + 1000);
    CHECK_EQ_U(f.b.rx_port, 0);
    uint64_t before = f.b.tx_counter;
    size_t n = bridge_build_moved(&f.b, NEW_IP, 45045, T0 + 2000, out, sizeof(out), &to_ip, &to_port);
    CHECK_EQ_U(n, PSHB_HDR_LEN + PSHB_MOVED_PAYLOAD_LEN + PSHB_TAG_LEN);
    CHECK_EQ_U(to_ip, APP_IP);
    CHECK_EQ_U(to_port, 50123);
    pshb_header_t h;
    CHECK(proto_check(out, n, &h));
    CHECK_EQ_U(h.type, PSHB_TYPE_MOVED);
    CHECK_EQ_U(h.counter, (before + 1) & PSHB_TX_COUNTER_MASK);  // counts on from the PONGs
    uint8_t payload[PSHB_MOVED_PAYLOAD_LEN];
    CHECK(proto_decrypt(KEY, PSHB_DIR_BOARD_TO_APP, out, n, payload));
    pshb_moved_t m;
    pshb_moved_decode(payload, &m);
    CHECK_EQ_U(m.session_id, SESSION);
    CHECK_EQ_U(m.ip[0], 192);
    CHECK_EQ_U(m.ip[1], 168);
    CHECK_EQ_U(m.ip[2], 1);
    CHECK_EQ_U(m.ip[3], 50);
    CHECK_EQ_U(m.udp_port, 45045);
    CHECK_EQ_U(payload[14] | payload[15], 0);  // reserved

    // A later app is told instead; one silent for too long is not.
    feed_from(&f, pkt, make_input(pkt, KEY, PSHB_TYPE_HEARTBEAT, 12, SESSION, 0, 0, 0), APP_IP + 1, 40000,
              T0 + 3000);
    CHECK(bridge_build_moved(&f.b, NEW_IP, 45045, T0 + 4000, out, sizeof(out), &to_ip, &to_port) > 0);
    CHECK_EQ_U(to_ip, APP_IP + 1);
    CHECK_EQ_U(to_port, 40000);
    CHECK_EQ_U(bridge_build_moved(&f.b, NEW_IP, 45045, T0 + 3000 + PSHB_MOVED_PEER_US + 1, out, sizeof(out),
                                  &to_ip, &to_port),
               0);
    // Without a key: nothing.
    bridge_set_key(&f.b, NULL);
    CHECK_EQ_U(bridge_build_moved(&f.b, NEW_IP, 45045, T0 + 4000, out, sizeof(out), &to_ip, &to_port), 0);

    // A MOVED sent to the board is never valid.
    bridge_set_key(&f.b, KEY);
    uint32_t malformed = f.b.stats.rx_malformed;
    n = proto_seal(KEY, PSHB_DIR_APP_TO_BOARD, PSHB_TYPE_MOVED, 50, payload, sizeof(payload), pkt, sizeof(pkt));
    feed(&f, pkt, n, T0 + 5000);
    CHECK_EQ_U(f.b.stats.rx_malformed, malformed + 1);
}

int main(void) {
    RUN(ping_gets_encrypted_pong);
    RUN(timed_ping_gets_time_stamps);
    RUN(pong_reports_missing_partition_table);
    RUN(input_is_applied);
    RUN(replay_and_old_counter_rejected);
    RUN(forged_packet_does_not_advance_counter);
    RUN(tampered_packet_rejected);
    RUN(no_key_rejects_everything);
    RUN(wrong_session_gets_rate_limited_hint);
    RUN(sender_filter);
    RUN(malformed_and_plaintext_rejected);
    RUN(failsafe_releases_after_timeout);
    RUN(pings_do_not_feed_failsafe);
    RUN(release_all_flag);
    RUN(moved_notice_goes_to_the_last_app);
    return TEST_SUMMARY();
}
