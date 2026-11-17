#include <stdlib.h>
#include <string.h>
#include <strings.h>

#include "mdns.h"
#include "mini_test.h"

#define T_A 1
#define T_PTR 12
#define T_TXT 16
#define T_AAAA 28
#define T_SRV 33
#define T_NSEC 47
#define T_ANY 255
#define QU 0x8000
#define FLUSH 0x8000

#define HOST "wohnzimmer-ps5.local"
#define INST "Wohnzimmer PS5._http._tcp.local"
#define FALLBACK "ps-hid-bridge-96c33d"

static const uint8_t k_ip[4] = {192, 168, 1, 50};
static const uint8_t k_peer[4] = {192, 168, 1, 20};

static pshb_mdns_info_t info(void) {
    pshb_mdns_info_t i = {
        .ip = {192, 168, 1, 50},
        .mask = {255, 255, 255, 0},
        .host = "wohnzimmer-ps5",
        .fallback = FALLBACK,
        .instance = "Wohnzimmer PS5",
        .http_port = 80,
    };
    return i;
}

static uint8_t g_out[1500];

// ---------------------------------------------------------------- building packets

typedef struct {
    uint8_t b[1500];
    size_t n;
} pkt_t;

static void put8(pkt_t *p, unsigned v) {
    p->b[p->n++] = (uint8_t)v;
}

static void put16(pkt_t *p, unsigned v) {
    put8(p, (v >> 8) & 0xFF);
    put8(p, v & 0xFF);
}

static void put32(pkt_t *p, uint32_t v) {
    put16(p, v >> 16);
    put16(p, v & 0xFFFF);
}

static void put_bytes(pkt_t *p, const void *d, size_t n) {
    memcpy(p->b + p->n, d, n);
    p->n += n;
}

// "a.b.c" as labels (no escapes: labels may hold spaces).
static void put_name(pkt_t *p, const char *s) {
    while (*s) {
        const char *dot = strchr(s, '.');
        size_t n = dot ? (size_t)(dot - s) : strlen(s);
        put8(p, (unsigned)n);
        put_bytes(p, s, n);
        s += n;
        if (*s == '.') s++;
    }
    put8(p, 0);
}

static void header(pkt_t *p, unsigned id, unsigned flags, unsigned qd, unsigned an, unsigned ns, unsigned ar) {
    p->n = 0;
    put16(p, id);
    put16(p, flags);
    put16(p, qd);
    put16(p, an);
    put16(p, ns);
    put16(p, ar);
}

static void question(pkt_t *p, const char *name, unsigned type, unsigned cls) {
    put_name(p, name);
    put16(p, type);
    put16(p, cls);
}

static void rr_a(pkt_t *p, const char *name, unsigned cls, uint32_t ttl, const uint8_t ip[4]) {
    put_name(p, name);
    put16(p, T_A);
    put16(p, cls);
    put32(p, ttl);
    put16(p, 4);
    put_bytes(p, ip, 4);
}

static pkt_t query(const char *name, unsigned type, unsigned cls) {
    pkt_t p;
    header(&p, 0, 0, 1, 0, 0, 0);
    question(&p, name, type, cls);
    return p;
}

// ---------------------------------------------------------------- reading packets

typedef struct {
    char name[300];
    uint16_t type, cls;
    uint32_t ttl;
    size_t rdoff;
    uint16_t rdlen;
    int section;  // 0 answer, 1 authority, 2 additional
} trr_t;

typedef struct {
    const uint8_t *p;
    size_t len;
    uint16_t id, flags, qd, an, ns, ar;
    char qname[8][300];
    uint16_t qtype[8], qcls[8];
    trr_t rr[24];
    int nrr;
    bool ok;
} tmsg_t;

static uint16_t be16(const uint8_t *p) {
    return (uint16_t)((p[0] << 8) | p[1]);
}

// A name as "a.b.c" (pointers followed; at most 32 of them).
static bool tname(const uint8_t *p, size_t len, size_t *off, char *out, size_t cap) {
    size_t pos = *off, n = 0;
    bool jumped = false;
    int jumps = 0;
    out[0] = '\0';
    for (;;) {
        if (pos >= len) return false;
        uint8_t b = p[pos];
        if ((b & 0xC0) == 0xC0) {
            if (pos + 1 >= len || ++jumps > 32) return false;
            if (!jumped) *off = pos + 2;
            jumped = true;
            pos = ((size_t)(b & 0x3F) << 8) | p[pos + 1];
            continue;
        }
        if (b == 0) {
            if (!jumped) *off = pos + 1;
            return true;
        }
        if (b > 63 || pos + 1 + b > len || n + b + 2 > cap) return false;
        if (n) out[n++] = '.';
        memcpy(out + n, p + pos + 1, b);
        n += b;
        out[n] = '\0';
        pos += 1u + b;
    }
}

static tmsg_t parse(const uint8_t *p, size_t len) {
    tmsg_t m;
    memset(&m, 0, sizeof(m));
    m.p = p;
    m.len = len;
    if (len < 12) return m;
    m.id = be16(p);
    m.flags = be16(p + 2);
    m.qd = be16(p + 4);
    m.an = be16(p + 6);
    m.ns = be16(p + 8);
    m.ar = be16(p + 10);
    size_t off = 12;
    for (int i = 0; i < m.qd; i++) {
        if (i >= 8 || !tname(p, len, &off, m.qname[i], sizeof(m.qname[i])) || off + 4 > len) return m;
        m.qtype[i] = be16(p + off);
        m.qcls[i] = be16(p + off + 2);
        off += 4;
    }
    int total = m.an + m.ns + m.ar;
    for (int i = 0; i < total; i++) {
        if (m.nrr >= 24) return m;
        trr_t *r = &m.rr[m.nrr];
        if (!tname(p, len, &off, r->name, sizeof(r->name)) || off + 10 > len) return m;
        r->type = be16(p + off);
        r->cls = be16(p + off + 2);
        r->ttl = ((uint32_t)be16(p + off + 4) << 16) | be16(p + off + 6);
        r->rdlen = be16(p + off + 8);
        r->rdoff = off + 10;
        if (r->rdoff + r->rdlen > len) return m;
        r->section = i < m.an ? 0 : i < m.an + m.ns ? 1 : 2;
        off = r->rdoff + r->rdlen;
        m.nrr++;
    }
    m.ok = off == len;
    return m;
}

static const trr_t *find(const tmsg_t *m, int section, const char *name, uint16_t type) {
    for (int i = 0; i < m->nrr; i++) {
        const trr_t *r = &m->rr[i];
        if (r->section == section && r->type == type && strcasecmp(r->name, name) == 0) return r;
    }
    return NULL;
}

static bool rd_name_is(const tmsg_t *m, const trr_t *r, size_t at, const char *want) {
    size_t off = r->rdoff + at;
    char n[300];
    return tname(m->p, m->len, &off, n, sizeof(n)) && strcmp(n, want) == 0;
}

// ---------------------------------------------------------------- driving

static size_t ask_from(pshb_mdns_t *m, const pkt_t *q, const uint8_t ip[4], uint16_t port, uint64_t now,
                       pshb_mdns_dest_t *d) {
    return pshb_mdns_input(m, q->b, q->n, ip, port, now, g_out, sizeof(g_out), d);
}

static size_t ask(pshb_mdns_t *m, const pkt_t *q, uint16_t port, uint64_t now, pshb_mdns_dest_t *d) {
    return ask_from(m, q, k_peer, port, now, d);
}

// Polls every millisecond until the names are announced; the time then.
static uint64_t until_ready(pshb_mdns_t *m, uint64_t t) {
    for (int i = 0; i < 70000 && m->phase != PSHB_MDNS_READY; i++, t++) pshb_mdns_poll(m, t, g_out, sizeof(g_out));
    return t;
}

static uint64_t start_ready(pshb_mdns_t *m, const pshb_mdns_info_t *i) {
    memset(m, 0, sizeof(*m));
    pshb_mdns_reset(m, i, 1000);
    return until_ready(m, 1000);
}

// The next packet poll sends from `*t` on (stepping 1 ms), 0 if none within `ms`.
static size_t next_packet(pshb_mdns_t *m, uint64_t *t, uint64_t ms) {
    for (uint64_t end = *t + ms; *t < end; (*t)++) {
        size_t n = pshb_mdns_poll(m, *t, g_out, sizeof(g_out));
        if (n) return n;
    }
    return 0;
}

// ---------------------------------------------------------------- tests

TEST(probes_then_announces) {
    pshb_mdns_t m;
    memset(&m, 0, sizeof(m));
    pshb_mdns_info_t i = info();
    uint64_t t0 = 10000;
    pshb_mdns_reset(&m, &i, t0);
    CHECK(pshb_mdns_name(&m) == NULL);

    uint64_t times[8];
    uint8_t pkts[8][600];
    size_t lens[8];
    int n = 0;
    for (uint64_t t = t0; t < t0 + 5000; t++) {
        size_t len = pshb_mdns_poll(&m, t, g_out, sizeof(g_out));
        if (!len) continue;
        CHECK(n < 8 && len <= sizeof(pkts[0]));
        if (n >= 8 || len > sizeof(pkts[0])) break;
        if (n < 3) CHECK(pshb_mdns_name(&m) == NULL);  // still probing
        times[n] = t;
        memcpy(pkts[n], g_out, len);
        lens[n++] = len;
    }
    CHECK_EQ_I(n, 5);
    if (n != 5) return;
    CHECK(times[0] >= t0 && times[0] < t0 + 250);  // a random-looking start
    CHECK_EQ_U(times[1] - times[0], 250);
    CHECK_EQ_U(times[2] - times[1], 250);
    CHECK_EQ_U(times[3] - times[2], 250);  // nobody objected: announced
    CHECK_EQ_U(times[4] - times[3], 1000);
    CHECK(m.phase == PSHB_MDNS_READY);
    CHECK(pshb_mdns_name(&m) && strcmp(pshb_mdns_name(&m), "wohnzimmer-ps5") == 0);

    for (int k = 0; k < 3; k++) {
        tmsg_t p = parse(pkts[k], lens[k]);
        CHECK(p.ok);
        CHECK_EQ_U(p.flags, 0);
        CHECK_EQ_U(p.qd, 2);
        CHECK_EQ_U(p.an, 0);
        CHECK_EQ_U(p.ns, 3);
        CHECK(strcmp(p.qname[0], HOST) == 0 && p.qtype[0] == T_ANY && p.qcls[0] == (QU | 1));
        CHECK(strcmp(p.qname[1], INST) == 0 && p.qtype[1] == T_ANY && p.qcls[1] == (QU | 1));
        const trr_t *a = find(&p, 1, HOST, T_A);
        CHECK(a && a->rdlen == 4 && memcmp(p.p + a->rdoff, k_ip, 4) == 0 && a->cls == 1);
        CHECK(find(&p, 1, INST, T_SRV) && find(&p, 1, INST, T_TXT));
    }
    for (int k = 3; k < 5; k++) {
        tmsg_t p = parse(pkts[k], lens[k]);
        CHECK(p.ok);
        CHECK_EQ_U(p.flags, 0x8400);
        CHECK_EQ_U(p.id, 0);
        CHECK_EQ_U(p.qd, 0);
        CHECK_EQ_U(p.an, 6);
        const trr_t *a = find(&p, 0, HOST, T_A);
        CHECK(a && a->cls == (FLUSH | 1) && a->ttl == 120 && memcmp(p.p + a->rdoff, k_ip, 4) == 0);
        const trr_t *nsec = find(&p, 0, HOST, T_NSEC);
        CHECK(nsec && nsec->cls == (FLUSH | 1) && nsec->ttl == 120);
        const trr_t *svc = find(&p, 0, "_services._dns-sd._udp.local", T_PTR);
        CHECK(svc && svc->cls == 1 && svc->ttl == 4500 && rd_name_is(&p, svc, 0, "_http._tcp.local"));
        const trr_t *ptr = find(&p, 0, "_http._tcp.local", T_PTR);
        CHECK(ptr && ptr->cls == 1 && rd_name_is(&p, ptr, 0, INST));
        const trr_t *srv = find(&p, 0, INST, T_SRV);
        CHECK(srv && srv->cls == (FLUSH | 1) && srv->ttl == 120);
        CHECK(srv && be16(p.p + srv->rdoff + 4) == 80 && rd_name_is(&p, srv, 6, HOST));
        const trr_t *txt = find(&p, 0, INST, T_TXT);
        CHECK(txt && txt->cls == (FLUSH | 1) && txt->ttl == 4500 && txt->rdlen == 7 &&
              memcmp(p.p + txt->rdoff, "\x06path=/", 7) == 0);
        CHECK(lens[k] < 300);  // names compressed
    }
    // Nothing more on its own.
    uint64_t t = times[4] + 1;
    CHECK_EQ_U(next_packet(&m, &t, 60000), 0);
}

TEST(silent_while_probing) {
    pshb_mdns_t m;
    memset(&m, 0, sizeof(m));
    pshb_mdns_info_t i = info();
    pshb_mdns_reset(&m, &i, 0);
    pkt_t q = query(HOST, T_A, 1);
    pshb_mdns_dest_t d;
    CHECK_EQ_U(ask(&m, &q, 5353, 10, &d), 0);
    CHECK_EQ_U(ask(&m, &q, 40000, 10, &d), 0);
}

TEST(answers_an_a_query) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 2000;
    pkt_t q = query(HOST, T_A, 1);
    pshb_mdns_dest_t d = PSHB_MDNS_TO_SENDER;
    size_t n = ask(&m, &q, 5353, t, &d);
    CHECK(n > 0);
    CHECK(d == PSHB_MDNS_TO_GROUP);
    tmsg_t p = parse(g_out, n);
    CHECK(p.ok);
    CHECK_EQ_U(p.id, 0);
    CHECK_EQ_U(p.flags, 0x8400);
    CHECK_EQ_U(p.qd, 0);
    CHECK_EQ_U(p.an, 1);
    const trr_t *a = find(&p, 0, HOST, T_A);
    CHECK(a && a->cls == (FLUSH | 1) && a->ttl == 120 && a->rdlen == 4 && memcmp(p.p + a->rdoff, k_ip, 4) == 0);
    // No AAAA: the NSEC says so right away.
    const trr_t *nsec = find(&p, 2, HOST, T_NSEC);
    CHECK(nsec && rd_name_is(&p, nsec, 0, HOST));
    CHECK(nsec && memcmp(p.p + nsec->rdoff + nsec->rdlen - 3, "\x00\x01\x40", 3) == 0);
}

TEST(one_multicast_per_record_and_second) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 2000;
    pkt_t q = query(HOST, T_A, 1);
    pshb_mdns_dest_t d;
    CHECK(ask(&m, &q, 5353, t, &d) > 0);
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);
    CHECK_EQ_U(ask(&m, &q, 5353, t + 999, &d), 0);
    CHECK(ask(&m, &q, 5353, t + 1000, &d) > 0);
    // Right after the announcement: nothing new for the group either.
    t = start_ready(&m, &i);
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);
    // The other records are not held back by this one.
    pkt_t q2 = query("_http._tcp.local", T_PTR, 1);
    CHECK(ask(&m, &q2, 5353, t + 1000, &d) > 0);
}

TEST(qu_questions) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i);
    pkt_t q = query(HOST, T_A, QU | 1);
    pshb_mdns_dest_t d = PSHB_MDNS_TO_GROUP;
    // Just announced: the asker gets its own copy.
    size_t n = ask(&m, &q, 5353, t + 100, &d);
    CHECK(n > 0 && d == PSHB_MDNS_TO_SENDER);
    tmsg_t p = parse(g_out, n);
    CHECK(p.ok && p.id == 0 && p.qd == 0 && find(&p, 0, HOST, T_A));
    // A while later (over a quarter of the TTL): to everyone.
    n = ask(&m, &q, 5353, t + 31000, &d);
    CHECK(n > 0 && d == PSHB_MDNS_TO_GROUP);
    // ... which counts as sent to the group now.
    n = ask(&m, &q, 5353, t + 31100, &d);
    CHECK(n > 0 && d == PSHB_MDNS_TO_SENDER);
    // One QM question among QU ones: the group.
    pkt_t mix;
    header(&mix, 0, 0, 2, 0, 0, 0);
    question(&mix, HOST, T_A, QU | 1);
    question(&mix, "_http._tcp.local", T_PTR, 1);
    n = ask(&m, &mix, 5353, t + 40000, &d);
    CHECK(n > 0 && d == PSHB_MDNS_TO_GROUP);
}

TEST(legacy_unicast) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i);
    pkt_t q;
    header(&q, 0x1234, 0x0100, 1, 0, 0, 0);  // RD set, as a plain resolver sends it
    question(&q, "Wohnzimmer-PS5.local", T_A, 1);
    pshb_mdns_dest_t d = PSHB_MDNS_TO_GROUP;
    size_t n = ask(&m, &q, 40000, t, &d);  // right after the announcement: still answered
    CHECK(n > 0 && d == PSHB_MDNS_TO_SENDER);
    tmsg_t p = parse(g_out, n);
    CHECK(p.ok);
    CHECK_EQ_U(p.id, 0x1234);
    CHECK_EQ_U(p.flags & 0x8400, 0x8400);
    CHECK_EQ_U(p.qd, 1);
    CHECK(strcmp(p.qname[0], "Wohnzimmer-PS5.local") == 0 && p.qtype[0] == T_A && p.qcls[0] == 1);
    const trr_t *a = find(&p, 0, HOST, T_A);
    CHECK(a && a->cls == 1 && a->ttl == 10 && memcmp(p.p + a->rdoff, k_ip, 4) == 0);
    for (int k = 0; k < p.nrr; k++) CHECK(!(p.rr[k].cls & FLUSH) && p.rr[k].ttl <= 10);
    // Unicast answers per second are limited.
    int answered = 1;
    for (int k = 0; k < 20; k++) answered += ask(&m, &q, 40000, t + 10, &d) > 0;
    CHECK_EQ_I(answered, PSHB_MDNS_MAX_UNICAST_PER_S);
    CHECK(ask(&m, &q, 40000, t + 1010, &d) > 0);
}

TEST(any_and_missing_types) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 2000;
    pshb_mdns_dest_t d;
    pkt_t q = query(HOST, T_ANY, 1);
    size_t n = ask(&m, &q, 5353, t, &d);
    tmsg_t p = parse(g_out, n);
    CHECK(n > 0 && p.ok && find(&p, 0, HOST, T_A));

    q = query(HOST, T_AAAA, 1);
    n = ask(&m, &q, 5353, t + 2000, &d);
    p = parse(g_out, n);
    CHECK(n > 0 && p.ok && p.an == 1);
    const trr_t *nsec = find(&p, 0, HOST, T_NSEC);
    CHECK(nsec && nsec->cls == (FLUSH | 1) && rd_name_is(&p, nsec, 0, HOST));

    q = query(INST, T_ANY, 1);
    n = ask(&m, &q, 5353, t + 4000, &d);
    p = parse(g_out, n);
    CHECK(n > 0 && p.ok && p.an == 2 && find(&p, 0, INST, T_SRV) && find(&p, 0, INST, T_TXT));
    CHECK(find(&p, 2, HOST, T_A));

    // Class ANY is answered, other classes are not; nor other names.
    q = query(HOST, T_A, 255);
    CHECK(ask(&m, &q, 5353, t + 6000, &d) > 0);
    q = query(HOST, T_A, 3);
    CHECK_EQ_U(ask(&m, &q, 5353, t + 8000, &d), 0);
    q = query("other.local", T_A, 1);
    CHECK_EQ_U(ask(&m, &q, 5353, t + 8000, &d), 0);
    q = query("wohnzimmer-ps5.local.local", T_A, 1);
    CHECK_EQ_U(ask(&m, &q, 5353, t + 8000, &d), 0);
    q = query("wohnzimmer-ps5", T_A, 1);
    CHECK_EQ_U(ask(&m, &q, 5353, t + 8000, &d), 0);
}

TEST(names_in_any_case) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 2000;
    pshb_mdns_dest_t d;
    pkt_t q = query("WOHNZIMMER-PS5.LOCAL", T_A, 1);
    CHECK(ask(&m, &q, 5353, t, &d) > 0);
    q = query("wohnzimmer ps5._HTTP._Tcp.LOCAL", T_SRV, 1);
    size_t n = ask(&m, &q, 5353, t, &d);
    tmsg_t p = parse(g_out, n);
    CHECK(n > 0 && p.ok && find(&p, 0, INST, T_SRV));
    q = query("_HTTP._TCP.local", T_PTR, 1);
    CHECK(ask(&m, &q, 5353, t, &d) > 0);
}

TEST(compressed_queries) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 2000;
    pshb_mdns_dest_t d;
    // "_http._tcp.local" PTR, then "Wohnzimmer PS5" + pointer to it SRV,
    // then "wohnzimmer-ps5" + pointer to its "local" A.
    pkt_t q;
    header(&q, 0, 0, 3, 0, 0, 0);
    size_t http = q.n;
    question(&q, "_http._tcp.local", T_PTR, 1);
    size_t local = http + 1 + 5 + 1 + 4;
    put8(&q, 14);
    put_bytes(&q, "Wohnzimmer PS5", 14);
    put16(&q, 0xC000 | http);
    put16(&q, T_SRV);
    put16(&q, 1);
    put8(&q, 14);
    put_bytes(&q, "wohnzimmer-ps5", 14);
    put16(&q, 0xC000 | local);
    put16(&q, T_A);
    put16(&q, 1);
    size_t n = ask(&m, &q, 5353, t, &d);
    tmsg_t p = parse(g_out, n);
    CHECK(n > 0 && p.ok && p.an == 3);
    CHECK(find(&p, 0, "_http._tcp.local", T_PTR) && find(&p, 0, INST, T_SRV) && find(&p, 0, HOST, T_A));

    // Pointer chains: name -> name -> name.
    header(&q, 0, 0, 3, 0, 0, 0);
    question(&q, "local", T_PTR, 1);
    put8(&q, 4);
    put_bytes(&q, "_tcp", 4);
    put16(&q, 0xC000 | 12);
    put16(&q, 1);
    put16(&q, 1);
    size_t tcp = 12 + 7 + 4;
    put8(&q, 5);
    put_bytes(&q, "_http", 5);
    put16(&q, 0xC000 | tcp);
    put16(&q, T_PTR);
    put16(&q, 1);
    n = ask(&m, &q, 5353, t + 2000, &d);
    p = parse(g_out, n);
    CHECK(n > 0 && p.ok && find(&p, 0, "_http._tcp.local", T_PTR));
}

// Every way a name can be broken: no answer, nothing read outside the packet
// (the host tests run with AddressSanitizer).
TEST(malformed_names) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 2000;
    pshb_mdns_dest_t d;
    pkt_t q;

    // A pointer to itself.
    header(&q, 0, 0, 1, 0, 0, 0);
    put16(&q, 0xC000 | 12);
    put16(&q, T_A);
    put16(&q, 1);
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);

    // Two names pointing at each other.
    header(&q, 0, 0, 2, 0, 0, 0);
    put8(&q, 1);
    put8(&q, 'a');
    put16(&q, 0xC000 | 18);
    put16(&q, T_A);
    put16(&q, 1);
    put8(&q, 1);  // offset 18
    put8(&q, 'b');
    put16(&q, 0xC000 | 12);
    put16(&q, T_A);
    put16(&q, 1);
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);

    // Labels then a pointer back into its own labels.
    header(&q, 0, 0, 1, 0, 0, 0);
    put8(&q, 1);
    put8(&q, 'a');
    put16(&q, 0xC000 | 12);
    put16(&q, T_A);
    put16(&q, 1);
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);

    // A pointer forward.
    header(&q, 0, 0, 1, 0, 0, 0);
    put16(&q, 0xC000 | 20);
    put16(&q, T_A);
    put16(&q, 1);
    put_name(&q, HOST);
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);

    // A pointer outside the packet.
    header(&q, 0, 0, 1, 0, 0, 0);
    put16(&q, 0xFFFF);
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);

    // Reserved label types 0x40 and 0x80.
    for (unsigned type = 0x40; type <= 0x80; type += 0x40) {
        header(&q, 0, 0, 1, 0, 0, 0);
        put8(&q, type | 1);
        put8(&q, 'a');
        put8(&q, 0);
        put16(&q, T_A);
        put16(&q, 1);
        CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);
    }

    // Longer than 255 bytes (labels of 63, without pointers).
    header(&q, 0, 0, 1, 0, 0, 0);
    for (int k = 0; k < 5; k++) {
        put8(&q, 63);
        memset(q.b + q.n, 'x', 63);
        q.n += 63;
    }
    put8(&q, 0);
    put16(&q, T_A);
    put16(&q, 1);
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);

    // ... and built from pointers: four questions of 63-byte labels, each
    // pointing at the one before (257 bytes in the end), then one for our
    // name. The whole packet is dropped, while the same packet with every
    // question pointing at the first (128 bytes each) is answered.
    for (int over = 0; over < 2; over++) {
        header(&q, 0, 0, 5, 0, 0, 0);
        size_t at[4];
        for (int k = 0; k < 4; k++) {
            at[k] = q.n;
            put8(&q, 63);
            memset(q.b + q.n, 'y', 63);
            q.n += 63;
            if (k == 0) {
                put8(&q, 0);
            } else {
                put16(&q, 0xC000 | at[over ? k - 1 : 0]);
            }
            put16(&q, T_A);
            put16(&q, 1);
        }
        question(&q, HOST, T_A, 1);
        size_t n = ask(&m, &q, 5353, t, &d);
        if (over) {
            CHECK_EQ_U(n, 0);
        } else {
            CHECK(n > 0);
        }
        t += 1000;
    }

    // More questions announced than there are.
    header(&q, 0, 0, 2, 0, 0, 0);
    question(&q, HOST, T_A, 1);
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);

    // Header only, shorter than a header.
    header(&q, 0, 0, 0, 0, 0, 0);
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);
    CHECK_EQ_U(ask_from(&m, &(pkt_t){.n = 0}, k_peer, 5353, t, &d), 0);
    q.n = 11;
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);
}

// Every prefix of good packets (query with known answers and authority
// records, a response, a probe) is taken apart without reading past it.
TEST(truncated_packets) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 2000;
    pshb_mdns_dest_t d;

    pkt_t q;
    header(&q, 0, 0, 2, 1, 1, 0);
    question(&q, "_http._tcp.local", T_PTR, 1);
    question(&q, HOST, T_ANY, QU | 1);
    put16(&q, 0xC00C);  // known answer: _http._tcp.local PTR -> instance
    put16(&q, T_PTR);
    put16(&q, 1);
    put32(&q, 4500);
    put16(&q, 17);
    put8(&q, 14);
    put_bytes(&q, "Wohnzimmer PS5", 14);
    put16(&q, 0xC00C);
    rr_a(&q, HOST, 1, 120, k_peer);

    pkt_t r;
    header(&r, 0, 0x8400, 0, 2, 0, 0);
    rr_a(&r, "printer.local", FLUSH | 1, 120, k_peer);
    put_name(&r, "Printer._http._tcp.local");
    put16(&r, T_SRV);
    put16(&r, FLUSH | 1);
    put32(&r, 120);
    put16(&r, 8);
    put16(&r, 0);
    put16(&r, 0);
    put16(&r, 631);
    put16(&r, 0xC00C);

    const pkt_t *all[] = {&q, &r};
    for (size_t k = 0; k < 2; k++) {
        for (size_t len = 0; len <= all[k]->n; len++) {
            // An exactly sized copy, so the sanitizer sees any read past it.
            uint8_t *copy = malloc(len ? len : 1);
            memcpy(copy, all[k]->b, len);
            size_t n = pshb_mdns_input(&m, copy, len, k_peer, 5353, t, g_out, sizeof(g_out), &d);
            if (n) CHECK(parse(g_out, n).ok);
            free(copy);
            t += 1000;
        }
    }
    CHECK(m.phase == PSHB_MDNS_READY);  // the printer's records are no conflict
}

static uint32_t s_rand = 12345;

static uint32_t rnd(void) {
    s_rand = s_rand * 1103515245u + 12345u;
    return s_rand >> 8;
}

// Random bytes and random changes to good packets: never a crash, any
// answer is a well-formed packet that fits its buffer.
TEST(fuzz) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    pshb_mdns_dest_t d;
    pkt_t seeds[4];
    seeds[0] = query(HOST, T_ANY, QU | 1);
    seeds[1] = query("_http._tcp.local", T_PTR, 1);
    header(&seeds[2], 0, 0x8400, 0, 1, 0, 1);
    rr_a(&seeds[2], HOST, FLUSH | 1, 120, k_peer);
    rr_a(&seeds[2], INST, 1, 120, k_peer);
    header(&seeds[3], 7, 0, 1, 1, 1, 0);
    question(&seeds[3], INST, T_ANY, 1);
    rr_a(&seeds[3], HOST, 1, 120, k_ip);
    rr_a(&seeds[3], HOST, 1, 120, k_peer);

    uint64_t t = start_ready(&m, &i);
    for (int iter = 0; iter < 60000; iter++) {
        uint8_t pkt[600];
        size_t len;
        if (iter % 3 == 0) {
            len = rnd() % sizeof(pkt);
            for (size_t k = 0; k < len; k++) pkt[k] = (uint8_t)rnd();
            if (len >= 4) pkt[2] = pkt[3] = 0;  // mostly queries, so they get past the header
        } else {
            const pkt_t *s = &seeds[rnd() % 4];
            len = s->n;
            memcpy(pkt, s->b, len);
            int changes = 1 + rnd() % 4;
            for (int c = 0; c < changes; c++) {
                size_t at = rnd() % len;
                switch (rnd() % 4) {
                case 0: pkt[at] = (uint8_t)rnd(); break;
                case 1: pkt[at] ^= (uint8_t)(1u << (rnd() % 8)); break;
                case 2: pkt[at] = 0xC0 | (rnd() % 2); break;
                default: len = at + 1; break;
                }
            }
        }
        uint8_t *copy = malloc(len ? len : 1);
        memcpy(copy, pkt, len);
        size_t cap = 64 + rnd() % 600;
        uint8_t *out = malloc(cap);
        uint16_t port = rnd() % 2 ? 5353 : 40000;
        size_t n = pshb_mdns_input(&m, copy, len, k_peer, port, t, out, cap, &d);
        CHECK(n <= cap);
        if (n) CHECK(parse(out, n).ok);
        size_t pn = pshb_mdns_poll(&m, t, out, cap);
        CHECK(pn <= cap);
        if (pn) CHECK(parse(out, pn).ok);
        free(copy);
        free(out);
        t += rnd() % 300;
        if (iter % 5000 == 0) t = start_ready(&m, &i);
    }
}

TEST(dns_sd) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 2000;
    pshb_mdns_dest_t d;

    pkt_t q = query("_services._dns-sd._udp.local", T_PTR, 1);
    size_t n = ask(&m, &q, 5353, t, &d);
    tmsg_t p = parse(g_out, n);
    const trr_t *r = find(&p, 0, "_services._dns-sd._udp.local", T_PTR);
    CHECK(n > 0 && p.ok && p.an == 1 && r && r->ttl == 4500 && r->cls == 1);
    CHECK(r && rd_name_is(&p, r, 0, "_http._tcp.local"));

    q = query("_http._tcp.local", T_PTR, 1);
    n = ask(&m, &q, 5353, t, &d);
    p = parse(g_out, n);
    r = find(&p, 0, "_http._tcp.local", T_PTR);
    CHECK(n > 0 && p.ok && p.an == 1 && r && r->cls == 1 && rd_name_is(&p, r, 0, INST));
    // Everything needed to connect comes along.
    CHECK(find(&p, 2, INST, T_SRV) && find(&p, 2, INST, T_TXT) && find(&p, 2, HOST, T_A));
    CHECK(find(&p, 2, HOST, T_NSEC));

    q = query(INST, T_SRV, 1);
    n = ask(&m, &q, 5353, t + 2000, &d);
    p = parse(g_out, n);
    r = find(&p, 0, INST, T_SRV);
    CHECK(n > 0 && p.ok && r && r->ttl == 120 && be16(p.p + r->rdoff) == 0 && be16(p.p + r->rdoff + 2) == 0);
    CHECK(r && be16(p.p + r->rdoff + 4) == 80 && rd_name_is(&p, r, 6, HOST));
    CHECK(find(&p, 2, HOST, T_A));

    q = query(INST, T_TXT, 1);
    n = ask(&m, &q, 5353, t + 4000, &d);
    p = parse(g_out, n);
    r = find(&p, 0, INST, T_TXT);
    CHECK(n > 0 && p.ok && r && r->rdlen == 7 && memcmp(p.p + r->rdoff, "\x06path=/", 7) == 0);

    // Another port for the web server: the SRV says so.
    i.http_port = 8080;
    t = start_ready(&m, &i) + 2000;
    q = query(INST, T_SRV, 1);
    n = ask(&m, &q, 5353, t, &d);
    p = parse(g_out, n);
    r = find(&p, 0, INST, T_SRV);
    CHECK(r && be16(p.p + r->rdoff + 4) == 8080);
}

// A response from another device (port 5353) with `name` -> `ip`.
static size_t claim(pshb_mdns_t *m, const char *name, const uint8_t ip[4], uint32_t ttl, uint16_t port, uint64_t t) {
    pkt_t r;
    header(&r, 0, 0x8400, 0, 1, 0, 0);
    rr_a(&r, name, FLUSH | 1, ttl, ip);
    pshb_mdns_dest_t d;
    static const uint8_t other[4] = {192, 168, 1, 77};
    return pshb_mdns_input(m, r.b, r.n, other, port, t, g_out, sizeof(g_out), &d);
}

TEST(conflict_while_probing_takes_the_default_name) {
    pshb_mdns_t m;
    memset(&m, 0, sizeof(m));
    pshb_mdns_info_t i = info();
    uint64_t t = 1000;
    pshb_mdns_reset(&m, &i, t);
    CHECK(next_packet(&m, &t, 300) > 0);  // first probe
    static const uint8_t other[4] = {192, 168, 1, 77};
    CHECK_EQ_U(claim(&m, HOST, other, 120, 5353, t + 10), 0);
    t += 10;
    size_t n = next_packet(&m, &t, 300);
    tmsg_t p = parse(g_out, n);
    CHECK(n > 0 && p.ok && p.ns == 3);
    CHECK(strcmp(p.qname[0], FALLBACK ".local") == 0);
    CHECK(strcmp(p.qname[1], FALLBACK "._http._tcp.local") == 0);
    t = until_ready(&m, t);
    CHECK(pshb_mdns_name(&m) && strcmp(pshb_mdns_name(&m), FALLBACK) == 0);
    pshb_mdns_dest_t d;
    pkt_t q = query(HOST, T_A, 1);
    CHECK_EQ_U(ask(&m, &q, 5353, t + 2000, &d), 0);
    q = query(FALLBACK ".local", T_A, 1);
    n = ask(&m, &q, 5353, t + 2000, &d);
    p = parse(g_out, n);
    CHECK(n > 0 && find(&p, 0, FALLBACK ".local", T_A));
    q = query("_http._tcp.local", T_PTR, 1);
    n = ask(&m, &q, 5353, t + 2000, &d);
    p = parse(g_out, n);
    const trr_t *r = find(&p, 0, "_http._tcp.local", T_PTR);
    CHECK(r && rd_name_is(&p, r, 0, FALLBACK "._http._tcp.local"));

    // Taken by someone else as well: silent, then from the start a minute later.
    CHECK(m.phase == PSHB_MDNS_READY);
    claim(&m, FALLBACK ".local", other, 120, 5353, t + 3000);
    CHECK(m.phase == PSHB_MDNS_PROBING);  // first making sure again
    t += 3000;
    CHECK(next_packet(&m, &t, 300) > 0);
    claim(&m, FALLBACK ".local", other, 120, 5353, t);
    CHECK(m.phase == PSHB_MDNS_WAITING);
    CHECK(pshb_mdns_name(&m) == NULL);
    uint64_t gave_up = t;
    CHECK_EQ_U(next_packet(&m, &t, PSHB_MDNS_RETRY_MS - 1), 0);
    n = next_packet(&m, &t, 10);
    p = parse(g_out, n);
    CHECK(n > 0 && t == gave_up + PSHB_MDNS_RETRY_MS && p.ns == 3 && strcmp(p.qname[0], HOST) == 0);
}

TEST(conflict_without_a_name_of_its_own) {
    pshb_mdns_t m;
    memset(&m, 0, sizeof(m));
    pshb_mdns_info_t i = info();
    snprintf(i.host, sizeof(i.host), FALLBACK);
    snprintf(i.instance, sizeof(i.instance), FALLBACK);
    uint64_t t = 0;
    pshb_mdns_reset(&m, &i, t);
    CHECK(next_packet(&m, &t, 300) > 0);
    static const uint8_t other[4] = {192, 168, 1, 77};
    claim(&m, FALLBACK ".local", other, 120, 5353, t);
    CHECK(m.phase == PSHB_MDNS_WAITING);
}

TEST(no_conflict) {
    pshb_mdns_t m;
    memset(&m, 0, sizeof(m));
    pshb_mdns_info_t i = info();
    uint64_t t = 0;
    pshb_mdns_reset(&m, &i, t);
    CHECK(next_packet(&m, &t, 300) > 0);
    static const uint8_t other[4] = {192, 168, 1, 77};
    claim(&m, HOST, k_ip, 120, 5353, t);       // our own record (a reflector)
    claim(&m, HOST, other, 120, 40000, t);     // not from port 5353
    claim(&m, HOST, other, 0, 5353, t);        // a goodbye
    claim(&m, "printer.local", other, 120, 5353, t);
    claim(&m, "wohnzimmer-ps5.fritz.box", other, 120, 5353, t);
    CHECK(m.phase == PSHB_MDNS_PROBING);
    t = until_ready(&m, t);
    CHECK(pshb_mdns_name(&m) && strcmp(pshb_mdns_name(&m), "wohnzimmer-ps5") == 0);
    // A query is no claim either, nor another device's record for another name.
    pkt_t q;
    header(&q, 0, 0, 0, 1, 0, 0);
    rr_a(&q, HOST, 1, 120, other);
    pshb_mdns_dest_t d;
    ask(&m, &q, 5353, t, &d);
    CHECK(m.phase == PSHB_MDNS_READY);
}

TEST(conflict_when_announced_probes_again) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 5000;
    static const uint8_t other[4] = {192, 168, 1, 77};
    // An SRV under our instance name pointing elsewhere counts, too.
    pkt_t r;
    header(&r, 0, 0x8400, 0, 1, 0, 0);
    put_name(&r, INST);
    put16(&r, T_SRV);
    put16(&r, FLUSH | 1);
    put32(&r, 120);
    put16(&r, 6 + 13);
    put16(&r, 0);
    put16(&r, 0);
    put16(&r, 80);
    put_name(&r, "other.local");
    pshb_mdns_dest_t d;
    CHECK_EQ_U(pshb_mdns_input(&m, r.b, r.n, other, 5353, t, g_out, sizeof(g_out), &d), 0);
    CHECK(m.phase == PSHB_MDNS_PROBING);
    CHECK(pshb_mdns_name(&m) == NULL);
    size_t n = next_packet(&m, &t, 10);
    tmsg_t p = parse(g_out, n);
    CHECK(n > 0 && p.ns == 3 && strcmp(p.qname[0], HOST) == 0);  // the same names again
    t = until_ready(&m, t);
    CHECK(pshb_mdns_name(&m) && strcmp(pshb_mdns_name(&m), "wohnzimmer-ps5") == 0);
    // A record of ours with other data (another address) as well.
    claim(&m, HOST, other, 120, 5353, t + 5000);
    CHECK(m.phase == PSHB_MDNS_PROBING);
}

TEST(new_name_takes_the_old_one_back) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 5000;
    pshb_mdns_info_t j = i;
    snprintf(j.host, sizeof(j.host), "kueche");
    snprintf(j.instance, sizeof(j.instance), "K\xc3\xbc" "che");
    pshb_mdns_reset(&m, &j, t);
    CHECK(pshb_mdns_name(&m) == NULL);
    // First the goodbye for what changed (TTL 0) ...
    size_t n = pshb_mdns_poll(&m, t, g_out, sizeof(g_out));
    tmsg_t p = parse(g_out, n);
    CHECK(n > 0 && p.ok && p.flags == 0x8400 && p.an == 5);
    for (int k = 0; k < p.nrr; k++) CHECK(p.rr[k].ttl == 0);
    const trr_t *a = find(&p, 0, HOST, T_A);
    CHECK(a && memcmp(p.p + a->rdoff, k_ip, 4) == 0);
    const trr_t *ptr = find(&p, 0, "_http._tcp.local", T_PTR);
    CHECK(ptr && rd_name_is(&p, ptr, 0, INST));
    CHECK(find(&p, 0, INST, T_SRV) && find(&p, 0, INST, T_TXT) && find(&p, 0, HOST, T_NSEC));
    CHECK(!find(&p, 0, "_services._dns-sd._udp.local", T_PTR));  // still true
    // ... then probes for the new names.
    n = next_packet(&m, &t, 300);
    p = parse(g_out, n);
    CHECK(n > 0 && p.ns == 3 && strcmp(p.qname[0], "kueche.local") == 0);
    CHECK(strcmp(p.qname[1], "K\xc3\xbc" "che._http._tcp.local") == 0);
    t = until_ready(&m, t);
    CHECK(pshb_mdns_name(&m) && strcmp(pshb_mdns_name(&m), "kueche") == 0);
    pshb_mdns_dest_t d;
    pkt_t q = query(HOST, T_A, 1);
    CHECK_EQ_U(ask(&m, &q, 5353, t + 2000, &d), 0);
    q = query("KUECHE.local", T_A, 1);
    CHECK(ask(&m, &q, 5353, t + 2000, &d) > 0);

    // A new address only: just the old A record goes.
    pshb_mdns_info_t k = j;
    k.ip[3] = 51;
    t += 10000;
    pshb_mdns_reset(&m, &k, t);
    n = pshb_mdns_poll(&m, t, g_out, sizeof(g_out));
    p = parse(g_out, n);
    a = find(&p, 0, "kueche.local", T_A);
    CHECK(n > 0 && p.an == 1 && a && a->ttl == 0 && p.p[a->rdoff + 3] == 50);
    t = until_ready(&m, t);

    // The same again (link up, the chip set up anew): probes, no goodbye.
    pshb_mdns_reset(&m, &k, t);
    n = next_packet(&m, &t, 300);
    p = parse(g_out, n);
    CHECK(n > 0 && p.flags == 0 && p.ns == 3);

    // While still probing, nothing was announced: nothing to take back.
    pshb_mdns_reset(&m, &j, t);
    n = next_packet(&m, &t, 300);
    p = parse(g_out, n);
    CHECK(n > 0 && p.flags == 0 && p.ns == 3);
}

// Another device probes for the same name at the same time.
static void their_probe(pshb_mdns_t *m, const uint8_t ip[4], uint64_t t) {
    pkt_t q;
    header(&q, 0, 0, 1, 0, 1, 0);
    question(&q, HOST, T_ANY, QU | 1);
    rr_a(&q, HOST, 1, 120, ip);
    static const uint8_t other[4] = {192, 168, 1, 77};
    pshb_mdns_dest_t d;
    CHECK_EQ_U(pshb_mdns_input(m, q.b, q.n, other, 5353, t, g_out, sizeof(g_out), &d), 0);
}

TEST(simultaneous_probes) {
    pshb_mdns_t m;
    memset(&m, 0, sizeof(m));
    pshb_mdns_info_t i = info();
    uint64_t t = 0;
    pshb_mdns_reset(&m, &i, t);
    CHECK(next_packet(&m, &t, 300) > 0);
    uint64_t first = t;
    // Their address compares lower: we win and go on.
    static const uint8_t low[4] = {192, 168, 1, 20}, high[4] = {192, 168, 1, 99};
    their_probe(&m, low, t + 5);
    their_probe(&m, k_ip, t + 6);  // our own probe, reflected: a tie, nothing happens
    t++;
    CHECK(next_packet(&m, &t, 300) > 0);
    CHECK_EQ_U(t, first + 250);
    // Higher: we lose, wait a second, then probe three times anew.
    their_probe(&m, high, t + 5);
    uint64_t lost = t + 5;
    t++;
    CHECK(next_packet(&m, &t, 2000) > 0);
    CHECK_EQ_U(t, lost + PSHB_MDNS_DEFER_MS);
    int probes = 1;
    for (;;) {
        t++;
        size_t n = next_packet(&m, &t, 2000);
        if (!n || parse(g_out, n).flags != 0) break;
        probes++;
    }
    CHECK_EQ_I(probes, 3);
    CHECK(pshb_mdns_name(&m) && strcmp(pshb_mdns_name(&m), "wohnzimmer-ps5") == 0);

    // Once announced, their probe is answered at once: they pick another name.
    t = until_ready(&m, t) + 300;
    pkt_t q;
    header(&q, 0, 0, 1, 0, 1, 0);
    question(&q, HOST, T_ANY, QU | 1);
    rr_a(&q, HOST, 1, 120, high);
    pshb_mdns_dest_t d = PSHB_MDNS_TO_SENDER;
    size_t n = ask(&m, &q, 5353, t, &d);
    tmsg_t p = parse(g_out, n);
    CHECK(n > 0 && d == PSHB_MDNS_TO_GROUP && find(&p, 0, HOST, T_A));
    // Even within the second, but not twice within 250 ms.
    CHECK_EQ_U(ask(&m, &q, 5353, t + 200, &d), 0);
    CHECK(ask(&m, &q, 5353, t + 250, &d) > 0);
    CHECK(m.phase == PSHB_MDNS_READY);
}

TEST(known_answers) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 2000;
    pshb_mdns_dest_t d;
    for (int fresh = 1; fresh >= 0; fresh--) {
        pkt_t q;
        header(&q, 0, 0, 1, 1, 0, 0);
        question(&q, "_http._tcp.local", T_PTR, 1);
        put16(&q, 0xC00C);
        put16(&q, T_PTR);
        put16(&q, 1);
        put32(&q, fresh ? 4000 : 2000);  // our TTL is 4500
        put16(&q, 17);
        put8(&q, 14);
        put_bytes(&q, "wohnzimmer ps5", 14);  // any case
        put16(&q, 0xC00C);
        size_t n = ask(&m, &q, 5353, t, &d);
        if (fresh) {
            CHECK_EQ_U(n, 0);
        } else {
            tmsg_t p = parse(g_out, n);
            CHECK(p.ok && find(&p, 0, "_http._tcp.local", T_PTR));
        }
    }
    // Another device's PTR in the list does not stop ours.
    pkt_t q;
    header(&q, 0, 0, 1, 1, 0, 0);
    question(&q, "_http._tcp.local", T_PTR, 1);
    put16(&q, 0xC00C);
    put16(&q, T_PTR);
    put16(&q, 1);
    put32(&q, 4500);
    put16(&q, 10);
    put8(&q, 7);
    put_bytes(&q, "Printer", 7);
    put16(&q, 0xC00C);
    CHECK(ask(&m, &q, 5353, t + 2000, &d) > 0);
}

TEST(only_the_own_network) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 2000;
    pshb_mdns_dest_t d;
    pkt_t q = query(HOST, T_A, 1);
    static const uint8_t outside[4] = {10, 0, 0, 5};
    CHECK_EQ_U(ask_from(&m, &q, outside, 5353, t, &d), 0);
    CHECK_EQ_U(ask_from(&m, &q, outside, 40000, t, &d), 0);
    memset(i.mask, 0, 4);  // no check (the simulator)
    t = start_ready(&m, &i) + 2000;
    CHECK(ask_from(&m, &q, outside, 5353, t, &d) > 0);
}

TEST(other_opcodes_and_errors) {
    pshb_mdns_t m;
    pshb_mdns_info_t i = info();
    uint64_t t = start_ready(&m, &i) + 2000;
    pshb_mdns_dest_t d;
    pkt_t q = query(HOST, T_A, 1);
    q.b[2] = 0x28;  // opcode 5 (update)
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);
    q = query(HOST, T_A, 1);
    q.b[3] = 0x03;  // rcode 3
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);
    q = query(HOST, T_A, 1);
    CHECK(ask(&m, &q, 5353, t, &d) > 0);
}

TEST(small_buffers) {
    pshb_mdns_t m;
    memset(&m, 0, sizeof(m));
    pshb_mdns_info_t i = info();
    uint64_t t = 0;
    pshb_mdns_reset(&m, &i, t);
    uint8_t buf[200];
    // Probes need about 150 bytes; with less nothing is sent, nothing written past.
    for (size_t cap = 0; cap < 120; cap += 7) {
        memset(buf, 0xAA, sizeof(buf));
        pshb_mdns_t copy = m;
        size_t n = 0;
        for (uint64_t k = 0; k < 300 && !n; k++) n = pshb_mdns_poll(&copy, k, buf, cap);
        CHECK_EQ_U(n, 0);
        for (size_t k = cap; k < sizeof(buf); k++) CHECK(buf[k] == 0xAA);
    }
    t = until_ready(&m, 0) + 2000;
    // An answer that fits only partly: the answers that fit, well formed.
    pkt_t q = query("_http._tcp.local", T_PTR, 1);
    pshb_mdns_dest_t d;
    memset(buf, 0xAA, sizeof(buf));
    size_t n = pshb_mdns_input(&m, q.b, q.n, k_peer, 5353, t, buf, 80, &d);
    tmsg_t p = parse(buf, n);
    CHECK(n > 0 && n <= 80 && p.ok && p.an == 1 && p.ar < 4);
    for (size_t k = 80; k < sizeof(buf); k++) CHECK(buf[k] == 0xAA);
    // Not even the answer fits.
    CHECK_EQ_U(pshb_mdns_input(&m, q.b, q.n, k_peer, 5353, t + 2000, buf, 20, &d), 0);
}

TEST(silent_without_address) {
    pshb_mdns_t m;
    memset(&m, 0, sizeof(m));
    pshb_mdns_info_t i = info();
    memset(i.ip, 0, 4);
    pshb_mdns_reset(&m, &i, 0);
    CHECK(m.phase == PSHB_MDNS_OFF);
    uint64_t t = 0;
    CHECK_EQ_U(next_packet(&m, &t, 5000), 0);
    pkt_t q = query(HOST, T_A, 1);
    pshb_mdns_dest_t d;
    CHECK_EQ_U(ask(&m, &q, 5353, t, &d), 0);
    CHECK(pshb_mdns_name(&m) == NULL);
    // An announced board losing its address takes nothing back (it cannot send).
    i = info();
    t = start_ready(&m, &i);
    memset(i.ip, 0, 4);
    pshb_mdns_reset(&m, &i, t);
    CHECK_EQ_U(next_packet(&m, &t, 5000), 0);
    // Longest names: still well-formed packets.
    i = info();
    memset(i.host, 'h', sizeof(i.host) - 1);
    memset(i.instance, 'i', sizeof(i.instance) - 1);
    memset(&m, 0, sizeof(m));
    pshb_mdns_reset(&m, &i, 0);
    t = 0;
    size_t n = next_packet(&m, &t, 300);
    CHECK(n > 0 && parse(g_out, n).ok);
    t = until_ready(&m, t);
    m.phase = PSHB_MDNS_ANNOUNCING;
    m.step = 0;
    n = pshb_mdns_poll(&m, t, g_out, sizeof(g_out));
    CHECK(n > 0 && n <= PSHB_MDNS_PACKET_MIN && parse(g_out, n).ok);
}

int main(void) {
    RUN(probes_then_announces);
    RUN(silent_while_probing);
    RUN(answers_an_a_query);
    RUN(one_multicast_per_record_and_second);
    RUN(qu_questions);
    RUN(legacy_unicast);
    RUN(any_and_missing_types);
    RUN(names_in_any_case);
    RUN(compressed_queries);
    RUN(malformed_names);
    RUN(truncated_packets);
    RUN(fuzz);
    RUN(dns_sd);
    RUN(conflict_while_probing_takes_the_default_name);
    RUN(conflict_without_a_name_of_its_own);
    RUN(no_conflict);
    RUN(conflict_when_announced_probes_again);
    RUN(new_name_takes_the_old_one_back);
    RUN(simultaneous_probes);
    RUN(known_answers);
    RUN(only_the_own_network);
    RUN(other_opcodes_and_errors);
    RUN(small_buffers);
    RUN(silent_without_address);
    return TEST_SUMMARY();
}
