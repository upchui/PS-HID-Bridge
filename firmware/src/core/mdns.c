#include "mdns.h"

#include <string.h>

#define T_A 1
#define T_NS 2
#define T_CNAME 5
#define T_PTR 12
#define T_TXT 16
#define T_SRV 33
#define T_NSEC 47
#define T_ANY 255
#define C_IN 1
#define C_ANY 255
#define C_TOP 0x8000u  // questions: a unicast answer is wanted (QU); records: cache flush

#define F_QR 0x8000u
#define F_AA 0x0400u
#define F_OPCODE 0x7800u
#define F_RCODE 0x000Fu

#define HDR 12
#define NAME_MAX_WIRE 255  // a name in wire format, its final zero included
#define MAX_JUMPS 64       // compression pointers in one name
#define THEIRS_MAX 4       // another device's probe records compared per name

enum { R_A, R_NSEC, R_SERVICES, R_PTR, R_SRV, R_TXT };
#define BIT(r) (1u << (r))
#define UNIQUE (BIT(R_A) | BIT(R_NSEC) | BIT(R_SRV) | BIT(R_TXT))
#define ALL_RECORDS ((1u << PSHB_MDNS_RECORDS) - 1u)

// Names in wire format; the string's own NUL is the final (root) label.
static const uint8_t k_local[] = "\x05local";
static const uint8_t k_http[] = "\x05_http\x04_tcp\x05local";
static const uint8_t k_services[] = "\x09_services\x07_dns-sd\x04_udp\x05local";
static const uint8_t k_txt[] = "\x06path=/";
// NSEC type bitmap: window 0, one byte, only type 1 (A).
static const uint8_t k_nsec_bitmap[] = {0x00, 0x01, 0x40};

static uint16_t rd16(const uint8_t *p) {
    return (uint16_t)((p[0] << 8) | p[1]);
}

static uint32_t rd32(const uint8_t *p) {
    return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) | ((uint32_t)p[2] << 8) | p[3];
}

static void wr16(uint8_t *p, uint16_t v) {
    p[0] = (uint8_t)(v >> 8);
    p[1] = (uint8_t)v;
}

// ---------------------------------------------------------------- names

// Length of an uncompressed, well-formed name, its final zero included.
static size_t wire_len(const uint8_t *n) {
    size_t i = 0;
    while (n[i]) i += 1u + n[i];
    return i + 1;
}

static uint8_t fold(uint8_t c) {
    return (c >= 'A' && c <= 'Z') ? (uint8_t)(c + ('a' - 'A')) : c;
}

// Two uncompressed, well-formed names; ASCII letters in any case (RFC 6762
// section 16), other bytes as they are.
static bool name_eq(const uint8_t *a, const uint8_t *b) {
    for (;;) {
        if (*a != *b) return false;
        uint8_t n = *a;
        if (n == 0) return true;
        for (uint8_t i = 1; i <= n; i++) {
            if (fold(a[i]) != fold(b[i])) return false;
        }
        a += 1u + n;
        b += 1u + n;
    }
}

// The name at `*off` in `p[0..len)`, compression pointers followed, into
// `out` uncompressed; `*off` moves past it. False if it is malformed:
// outside the packet, a label type other than length or pointer, longer
// than 255 bytes, or a pointer not strictly before the previous one (so a
// pointer loop ends at once).
static bool read_name(const uint8_t *p, size_t len, size_t *off, uint8_t out[NAME_MAX_WIRE]) {
    size_t pos = *off, n = 0, end = 0, limit = *off;
    bool jumped = false;
    unsigned jumps = 0;
    for (;;) {
        if (pos >= len) return false;
        uint8_t b = p[pos];
        if ((b & 0xC0) == 0xC0) {
            if (pos + 1 >= len || ++jumps > MAX_JUMPS) return false;
            size_t to = ((size_t)(b & 0x3F) << 8) | p[pos + 1];
            if (to >= limit) return false;
            if (!jumped) end = pos + 2;
            jumped = true;
            limit = to;
            pos = to;
            continue;
        }
        if (b & 0xC0) return false;  // 0x40, 0x80: no such labels in mDNS
        if (b == 0) {
            out[n] = 0;
            *off = jumped ? end : pos + 1;
            return true;
        }
        if (pos + 1u + b > len || n + 2u + b > NAME_MAX_WIRE) return false;
        memcpy(out + n, p + pos, 1u + b);
        n += 1u + b;
        pos += 1u + b;
    }
}

static bool read_question(const uint8_t *p, size_t len, size_t *off, uint8_t name[NAME_MAX_WIRE], uint16_t *type,
                          uint16_t *cls) {
    if (!read_name(p, len, off, name) || len - *off < 4) return false;
    *type = rd16(p + *off);
    *cls = rd16(p + *off + 2);
    *off += 4;
    return true;
}

typedef struct {
    uint8_t name[NAME_MAX_WIRE];
    uint16_t type, cls;
    uint32_t ttl;
    size_t rd;  // offset of the rdata in the packet
    size_t rdlen;
} rr_t;

static bool read_rr(const uint8_t *p, size_t len, size_t *off, rr_t *rr) {
    if (!read_name(p, len, off, rr->name) || len - *off < 10) return false;
    const uint8_t *h = p + *off;
    rr->type = rd16(h);
    rr->cls = rd16(h + 2);
    rr->ttl = rd32(h + 4);
    rr->rdlen = rd16(h + 8);
    rr->rd = *off + 10;
    if (rr->rdlen > len - rr->rd) return false;
    *off = rr->rd + rr->rdlen;
    return true;
}

// A name `at` bytes into the rdata of `rr` (it may point back into the
// packet); `*next`: where the rdata goes on after it.
static bool rd_name(const uint8_t *p, const rr_t *rr, size_t at, uint8_t out[NAME_MAX_WIRE], size_t *next) {
    if (at >= rr->rdlen) return false;
    size_t off = rr->rd + at;
    if (!read_name(p, rr->rd + rr->rdlen, &off, out)) return false;
    *next = off - rr->rd;
    return true;
}

// ---------------------------------------------------------------- our records

static uint16_t rec_type(int r) {
    static const uint16_t t[PSHB_MDNS_RECORDS] = {T_A, T_NSEC, T_PTR, T_PTR, T_SRV, T_TXT};
    return t[r];
}

static uint32_t rec_ttl(int r) {
    return (BIT(r) & (BIT(R_A) | BIT(R_NSEC) | BIT(R_SRV))) ? PSHB_MDNS_HOST_TTL : PSHB_MDNS_OTHER_TTL;
}

// Which of our records `rr` is (same name, type and data; any TTL), -1 if
// none.
static int our_record(const pshb_mdns_set_t *s, const uint8_t *p, const rr_t *rr) {
    if ((rr->cls & 0x7FFF) != C_IN) return -1;
    uint8_t n[NAME_MAX_WIRE];
    size_t next;
    if (name_eq(rr->name, s->host)) {
        if (rr->type == T_A) return rr->rdlen == 4 && memcmp(p + rr->rd, s->ip, 4) == 0 ? R_A : -1;
        if (rr->type == T_NSEC && rd_name(p, rr, 0, n, &next) && name_eq(n, s->host) &&
            rr->rdlen - next == sizeof(k_nsec_bitmap) &&
            memcmp(p + rr->rd + next, k_nsec_bitmap, sizeof(k_nsec_bitmap)) == 0) {
            return R_NSEC;
        }
        return -1;
    }
    if (name_eq(rr->name, k_services)) {
        return rr->type == T_PTR && rd_name(p, rr, 0, n, &next) && next == rr->rdlen && name_eq(n, k_http)
                   ? R_SERVICES
                   : -1;
    }
    if (name_eq(rr->name, k_http)) {
        return rr->type == T_PTR && rd_name(p, rr, 0, n, &next) && next == rr->rdlen && name_eq(n, s->inst) ? R_PTR
                                                                                                           : -1;
    }
    if (name_eq(rr->name, s->inst)) {
        if (rr->type == T_SRV) {
            return rr->rdlen > 6 && rd16(p + rr->rd + 4) == s->port && rd_name(p, rr, 6, n, &next) &&
                           next == rr->rdlen && name_eq(n, s->host)
                       ? R_SRV
                       : -1;
        }
        if (rr->type == T_TXT) {
            return rr->rdlen == sizeof(k_txt) - 1 && memcmp(p + rr->rd, k_txt, sizeof(k_txt) - 1) == 0 ? R_TXT : -1;
        }
    }
    return -1;
}

// Record `r` is the same in both sets (byte for byte: a name that only
// changed case is taken back and announced anew).
static bool same_record(const pshb_mdns_set_t *a, const pshb_mdns_set_t *b, int r) {
    bool host = memcmp(a->host, b->host, wire_len(a->host)) == 0;
    bool inst = memcmp(a->inst, b->inst, wire_len(a->inst)) == 0;
    switch (r) {
    case R_A: return host && memcmp(a->ip, b->ip, 4) == 0;
    case R_NSEC: return host;
    case R_SERVICES: return true;
    case R_PTR: return inst;
    case R_SRV: return inst && host && a->port == b->port;
    default: return inst;
    }
}

// ---------------------------------------------------------------- writing

enum { N_LOCAL, N_HOST, N_HTTP, N_INST, N_SERVICES, N_COUNT };

typedef struct {
    uint8_t *p;
    size_t cap, len;
    bool full;             // something did not fit: everything after it is dropped
    uint16_t at[N_COUNT];  // where each name was written, for compression (0 = not yet)
} wr_t;

static void w_bytes(wr_t *w, const void *d, size_t n) {
    if (w->full || n > w->cap - w->len) {
        w->full = true;
        return;
    }
    memcpy(w->p + w->len, d, n);
    w->len += n;
}

static void w_u16(wr_t *w, uint16_t v) {
    uint8_t b[2] = {(uint8_t)(v >> 8), (uint8_t)v};
    w_bytes(w, b, 2);
}

static void w_u32(wr_t *w, uint32_t v) {
    uint8_t b[4] = {(uint8_t)(v >> 24), (uint8_t)(v >> 16), (uint8_t)(v >> 8), (uint8_t)v};
    w_bytes(w, b, 4);
}

// One of our names, compressed against the ones already in the packet.
static void w_name(wr_t *w, const pshb_mdns_set_t *s, int id) {
    if (w->at[id]) {
        w_u16(w, (uint16_t)(0xC000u | w->at[id]));
        return;
    }
    size_t here = w->len;
    switch (id) {
    case N_LOCAL: w_bytes(w, k_local, sizeof(k_local)); break;
    case N_HOST:
        w_bytes(w, s->host, 1u + s->host[0]);
        w_name(w, s, N_LOCAL);
        break;
    case N_HTTP:
        w_bytes(w, k_http, sizeof(k_http) - sizeof(k_local));
        w_name(w, s, N_LOCAL);
        break;
    case N_INST:
        w_bytes(w, s->inst, 1u + s->inst[0]);
        w_name(w, s, N_HTTP);
        break;
    default:
        w_bytes(w, k_services, sizeof(k_services) - sizeof(k_local));
        w_name(w, s, N_LOCAL);
        break;
    }
    if (!w->full && here <= 0x3FFF) w->at[id] = (uint16_t)here;
}

// Record `r`; false (and nothing written) if it does not fit.
static bool w_record(wr_t *w, const pshb_mdns_set_t *s, int r, uint32_t ttl, bool flush) {
    static const uint8_t owner[PSHB_MDNS_RECORDS] = {
        [R_A] = N_HOST, [R_NSEC] = N_HOST, [R_SERVICES] = N_SERVICES,
        [R_PTR] = N_HTTP, [R_SRV] = N_INST, [R_TXT] = N_INST,
    };
    size_t mark = w->len;
    uint16_t at[N_COUNT];
    memcpy(at, w->at, sizeof(at));
    w_name(w, s, owner[r]);
    w_u16(w, rec_type(r));
    w_u16(w, (uint16_t)(C_IN | (flush ? C_TOP : 0)));
    w_u32(w, ttl);
    size_t rdlen_at = w->len;
    w_u16(w, 0);
    size_t rd = w->len;
    switch (r) {
    case R_A: w_bytes(w, s->ip, 4); break;
    case R_NSEC:
        // The next name is the record's own (RFC 6762 section 6.1), written out.
        w_bytes(w, s->host, wire_len(s->host));
        w_bytes(w, k_nsec_bitmap, sizeof(k_nsec_bitmap));
        break;
    case R_SERVICES: w_name(w, s, N_HTTP); break;
    case R_PTR: w_name(w, s, N_INST); break;
    case R_SRV:
        w_u16(w, 0);  // priority
        w_u16(w, 0);  // weight
        w_u16(w, s->port);
        w_name(w, s, N_HOST);
        break;
    default: w_bytes(w, k_txt, sizeof(k_txt) - 1); break;
    }
    if (w->full) {
        w->len = mark;
        w->full = false;
        memcpy(w->at, at, sizeof(at));
        return false;
    }
    wr16(w->p + rdlen_at, (uint16_t)(w->len - rd));
    return true;
}

static void w_header(wr_t *w, uint16_t id, uint16_t flags) {
    w_u16(w, id);
    w_u16(w, flags);
    for (int i = 0; i < 4; i++) w_u16(w, 0);
}

static void set_counts(uint8_t *p, uint16_t qd, uint16_t an, uint16_t ns, uint16_t ar) {
    wr16(p + 4, qd);
    wr16(p + 6, an);
    wr16(p + 8, ns);
    wr16(p + 10, ar);
}

typedef struct {
    bool legacy;  // a plain DNS resolver: its ID and question, TTL at most 10 s, no cache flush
    uint16_t id;
    const uint8_t *q;  // the query, whose questions a legacy answer repeats
    size_t q_len;
    uint16_t q_count;
    bool goodbye;  // TTL 0
} resp_t;

static uint32_t resp_ttl(const resp_t *o, int r) {
    if (o->goodbye) return 0;
    uint32_t t = rec_ttl(r);
    return o->legacy && t > PSHB_MDNS_LEGACY_TTL ? PSHB_MDNS_LEGACY_TTL : t;
}

// A response with the answers `ans` and the additional records `add`; its
// length (0 if not even one answer fits) and in `*sent` the records in it.
static size_t w_response(const pshb_mdns_set_t *s, unsigned ans, unsigned add, const resp_t *o, uint8_t *out,
                         size_t cap, unsigned *sent) {
    wr_t w = {.p = out, .cap = cap};
    *sent = 0;
    w_header(&w, o->legacy ? o->id : 0, F_QR | F_AA);
    uint16_t qd = 0, an = 0, ar = 0;
    if (o->legacy) {
        size_t off = HDR;
        uint8_t name[NAME_MAX_WIRE];
        uint16_t type, cls;
        for (uint16_t i = 0; i < o->q_count && read_question(o->q, o->q_len, &off, name, &type, &cls); i++) {
            w_bytes(&w, name, wire_len(name));
            w_u16(&w, type);
            w_u16(&w, cls & 0x7FFF);
            qd++;
        }
    }
    if (w.full) return 0;
    bool flush = !o->legacy && !o->goodbye;
    for (int r = 0; r < PSHB_MDNS_RECORDS; r++) {
        if ((ans & BIT(r)) && w_record(&w, s, r, resp_ttl(o, r), flush && (UNIQUE & BIT(r)))) {
            an++;
            *sent |= BIT(r);
        }
    }
    if (!an) return 0;
    for (int r = 0; r < PSHB_MDNS_RECORDS; r++) {
        if ((add & BIT(r)) && w_record(&w, s, r, resp_ttl(o, r), flush && (UNIQUE & BIT(r)))) {
            ar++;
            *sent |= BIT(r);
        }
    }
    set_counts(out, qd, an, 0, ar);
    return w.len;
}

// A probe: "who has these names?" (asking for unicast answers), with the
// records we want to use in the authority section.
static size_t w_probe(const pshb_mdns_set_t *s, uint8_t *out, size_t cap) {
    wr_t w = {.p = out, .cap = cap};
    w_header(&w, 0, 0);
    w_name(&w, s, N_HOST);
    w_u16(&w, T_ANY);
    w_u16(&w, C_IN | C_TOP);
    w_name(&w, s, N_INST);
    w_u16(&w, T_ANY);
    w_u16(&w, C_IN | C_TOP);
    if (w.full) return 0;
    if (!w_record(&w, s, R_A, PSHB_MDNS_HOST_TTL, false) || !w_record(&w, s, R_SRV, PSHB_MDNS_HOST_TTL, false) ||
        !w_record(&w, s, R_TXT, PSHB_MDNS_OTHER_TTL, false)) {
        return 0;
    }
    set_counts(out, 2, 0, 3, 0);
    return w.len;
}

// ---------------------------------------------------------------- state

static void put_name(uint8_t *out, const char *label, const uint8_t *suffix, size_t suffix_len) {
    size_t n = strlen(label);
    out[0] = (uint8_t)n;
    memcpy(out + 1, label, n);
    memcpy(out + 1 + n, suffix, suffix_len);
}

// Names for the next attempt (the user's, or the default ones after a
// conflict); false if there is nothing to answer with.
static bool use_names(pshb_mdns_t *m) {
    const pshb_mdns_info_t *i = &m->info;
    const char *host = m->use_fallback || !i->host[0] ? i->fallback : i->host;
    const char *inst = m->use_fallback ? i->fallback : i->instance[0] ? i->instance : host;
    static const uint8_t none[4] = {0};
    if (!host[0] || memcmp(i->ip, none, 4) == 0) return false;
    memcpy(m->host, host, PSHB_NAME_SIZE);
    memcpy(m->inst, inst, PSHB_NAME_SIZE);
    memcpy(m->cur.ip, i->ip, 4);
    m->cur.port = i->http_port;
    put_name(m->cur.host, host, k_local, sizeof(k_local));
    put_name(m->cur.inst, inst, k_http, sizeof(k_http));
    return true;
}

static bool has_alternative(const pshb_mdns_t *m) {
    const char *f = m->info.fallback;
    return !m->use_fallback && f[0] && (strcmp(m->host, f) != 0 || strcmp(m->inst, f) != 0);
}

// Spreads the first probe of boards that start together over 0..249 ms
// (RFC 6762 section 8.1); the same for one board and address.
static uint64_t jitter_ms(const pshb_mdns_info_t *i) {
    uint32_t h = 2166136261u;
    for (const char *c = i->fallback; *c; c++) h = (h ^ (uint8_t)*c) * 16777619u;
    for (int k = 0; k < 4; k++) h = (h ^ i->ip[k]) * 16777619u;
    return h % PSHB_MDNS_PROBE_MS;
}

static void start_probe(pshb_mdns_t *m, uint64_t at_ms) {
    m->phase = PSHB_MDNS_PROBING;
    m->step = 0;
    m->next_ms = at_ms;
}

// Records that went to the group less than `gap_ms` ago.
static unsigned sent_within(const pshb_mdns_t *m, uint64_t gap_ms, uint64_t now_ms) {
    unsigned bits = 0;
    for (int r = 0; r < PSHB_MDNS_RECORDS; r++) {
        if ((m->mcast_known & BIT(r)) && now_ms - m->mcast_ms[r] < gap_ms) bits |= BIT(r);
    }
    return bits;
}

// Every record in `bits` went to the group within a quarter of its TTL.
static bool recently_sent(const pshb_mdns_t *m, unsigned bits, uint64_t now_ms) {
    for (int r = 0; r < PSHB_MDNS_RECORDS; r++) {
        if (!(bits & BIT(r))) continue;
        if (!(m->mcast_known & BIT(r)) || now_ms - m->mcast_ms[r] >= rec_ttl(r) * 1000u / 4) return false;
    }
    return true;
}

static void mark_sent(pshb_mdns_t *m, unsigned bits, uint64_t now_ms) {
    for (int r = 0; r < PSHB_MDNS_RECORDS; r++) {
        if (!(bits & BIT(r))) continue;
        m->mcast_ms[r] = now_ms;
        m->mcast_known |= (uint8_t)BIT(r);
    }
}

static bool unicast_allow(pshb_mdns_t *m, uint64_t now_ms) {
    if (now_ms - m->ucast_window_ms >= 1000) {
        m->ucast_window_ms = now_ms;
        m->ucast_sent = 0;
    }
    if (m->ucast_sent >= PSHB_MDNS_MAX_UNICAST_PER_S) return false;
    m->ucast_sent++;
    return true;
}

void pshb_mdns_reset(pshb_mdns_t *m, const pshb_mdns_info_t *info, uint64_t now_ms) {
    bool announced = m->phase == PSHB_MDNS_ANNOUNCING || m->phase == PSHB_MDNS_READY;
    pshb_mdns_set_t before = m->cur;
    m->info = *info;
    m->info.host[PSHB_NAME_SIZE - 1] = '\0';
    m->info.fallback[PSHB_NAME_SIZE - 1] = '\0';
    m->info.instance[PSHB_NAME_SIZE - 1] = '\0';
    m->use_fallback = false;
    m->mcast_known = 0;
    m->goodbye = 0;
    if (!use_names(m)) {
        m->phase = PSHB_MDNS_OFF;
        return;
    }
    if (announced) {
        for (int r = 0; r < PSHB_MDNS_RECORDS; r++) {
            if (!same_record(&before, &m->cur, r)) m->goodbye |= (uint8_t)BIT(r);
        }
        m->old = before;
    }
    start_probe(m, now_ms + jitter_ms(&m->info));
}

const char *pshb_mdns_name(const pshb_mdns_t *m) {
    return m->phase == PSHB_MDNS_ANNOUNCING || m->phase == PSHB_MDNS_READY ? m->host : NULL;
}

// Another device uses one of our names.
static void conflict(pshb_mdns_t *m, uint64_t now_ms) {
    if (m->phase != PSHB_MDNS_PROBING) {
        // Announced already: make sure again (RFC 6762 section 9).
        start_probe(m, now_ms);
    } else if (has_alternative(m)) {
        m->use_fallback = true;
        use_names(m);
        start_probe(m, now_ms);
    } else {
        m->phase = PSHB_MDNS_WAITING;
        m->next_ms = now_ms + PSHB_MDNS_RETRY_MS;
    }
}

// ---------------------------------------------------------------- input

// A record under our names that is not ours, from another device: a
// conflict (a goodbye, TTL 0, gives a name up and is none).
static void on_response(pshb_mdns_t *m, const uint8_t *p, size_t len, uint64_t now_ms) {
    if (m->phase != PSHB_MDNS_PROBING && m->phase != PSHB_MDNS_ANNOUNCING && m->phase != PSHB_MDNS_READY) return;
    size_t off = HDR;
    uint8_t name[NAME_MAX_WIRE];
    uint16_t type, cls;
    for (uint16_t i = rd16(p + 4); i > 0; i--) {
        if (!read_question(p, len, &off, name, &type, &cls)) return;
    }
    rr_t rr;
    for (uint32_t i = (uint32_t)rd16(p + 6) + rd16(p + 8) + rd16(p + 10); i > 0; i--) {
        if (!read_rr(p, len, &off, &rr)) return;
        if (rr.ttl == 0 || (rr.cls & 0x7FFF) != C_IN) continue;
        if (!name_eq(rr.name, m->cur.host) && !name_eq(rr.name, m->cur.inst)) continue;
        if (our_record(&m->cur, p, &rr) >= 0) continue;
        conflict(m, now_ms);
        return;
    }
}

// The comparable form of a record (RFC 6762 section 8.2): class, type,
// rdata with names written out.
static bool canon_rr(const uint8_t *p, const rr_t *rr, pshb_mdns_canon_t *c) {
    c->cls = rr->cls & 0x7FFF;
    c->type = rr->type;
    size_t pre;
    if (rr->type == T_SRV) {
        pre = 6;
    } else if (rr->type == T_PTR || rr->type == T_NSEC || rr->type == T_CNAME || rr->type == T_NS) {
        pre = 0;
    } else {
        size_t n = rr->rdlen < sizeof(c->data) ? rr->rdlen : sizeof(c->data);
        memcpy(c->data, p + rr->rd, n);
        c->len = (uint16_t)n;
        return true;
    }
    uint8_t name[NAME_MAX_WIRE];
    size_t next;
    if (rr->rdlen < pre || !rd_name(p, rr, pre, name, &next)) return false;
    size_t nl = wire_len(name);
    memcpy(c->data, p + rr->rd, pre);
    memcpy(c->data + pre, name, nl);
    size_t rest = rr->rdlen - next, room = sizeof(c->data) - pre - nl;
    if (rest > room) rest = room;
    memcpy(c->data + pre + nl, p + rr->rd + next, rest);
    c->len = (uint16_t)(pre + nl + rest);
    return true;
}

static int canon_cmp(const pshb_mdns_canon_t *a, const pshb_mdns_canon_t *b) {
    if (a->cls != b->cls) return a->cls < b->cls ? -1 : 1;
    if (a->type != b->type) return a->type < b->type ? -1 : 1;
    size_t n = a->len < b->len ? a->len : b->len;
    int c = memcmp(a->data, b->data, n);
    if (c) return c < 0 ? -1 : 1;
    return a->len == b->len ? 0 : a->len < b->len ? -1 : 1;
}

// Our probe records under the host name (`host`) or the instance name, in
// the order they are compared.
static size_t our_canon(const pshb_mdns_set_t *s, bool host, pshb_mdns_canon_t *out) {
    if (host) {
        out[0] = (pshb_mdns_canon_t){.cls = C_IN, .type = T_A, .len = 4};
        memcpy(out[0].data, s->ip, 4);
        return 1;
    }
    out[0] = (pshb_mdns_canon_t){.cls = C_IN, .type = T_TXT, .len = sizeof(k_txt) - 1};
    memcpy(out[0].data, k_txt, sizeof(k_txt) - 1);
    size_t hl = wire_len(s->host);
    out[1] = (pshb_mdns_canon_t){.cls = C_IN, .type = T_SRV, .len = (uint16_t)(6 + hl)};
    wr16(out[1].data + 4, s->port);
    memcpy(out[1].data + 6, s->host, hl);
    return 2;
}

// Another device probes for one of our names while we do (RFC 6762 section
// 8.2): the one whose records compare later wins; the loser waits a second
// and probes again (and then meets the winner's answer). `off` points at
// the query's answer section.
static void tiebreak(pshb_mdns_t *m, const uint8_t *p, size_t len, size_t off, uint16_t an, uint16_t ns,
                     uint64_t now_ms) {
    rr_t rr;
    for (; an > 0; an--) {
        if (!read_rr(p, len, &off, &rr)) return;
    }
    pshb_mdns_canon_t *theirs = m->scratch, *ours = m->scratch + THEIRS_MAX;
    for (int which = 0; which < 2; which++) {
        const uint8_t *name = which == 0 ? m->cur.host : m->cur.inst;
        size_t nt = 0, at = off;
        uint8_t order[THEIRS_MAX];
        for (uint16_t i = 0; i < ns && nt < THEIRS_MAX; i++) {
            if (!read_rr(p, len, &at, &rr)) return;
            if (!name_eq(rr.name, name)) continue;
            if (!canon_rr(p, &rr, &theirs[nt])) return;
            // Sorted as it comes in.
            size_t k = nt;
            while (k > 0 && canon_cmp(&theirs[order[k - 1]], &theirs[nt]) > 0) {
                order[k] = order[k - 1];
                k--;
            }
            order[k] = (uint8_t)nt;
            nt++;
        }
        if (nt == 0) continue;
        size_t no = our_canon(&m->cur, which == 0, ours);
        int c = 0;
        for (size_t k = 0; c == 0 && (k < no || k < nt); k++) {
            c = k >= no ? -1 : k >= nt ? 1 : canon_cmp(&ours[k], &theirs[order[k]]);
        }
        if (c < 0) {
            start_probe(m, now_ms + PSHB_MDNS_DEFER_MS);
            return;
        }
    }
}

static size_t on_query(pshb_mdns_t *m, const uint8_t *p, size_t len, uint16_t src_port, uint64_t now_ms,
                       uint8_t *out, size_t cap, pshb_mdns_dest_t *dest) {
    bool answering = m->phase == PSHB_MDNS_ANNOUNCING || m->phase == PSHB_MDNS_READY;
    if (!answering && m->phase != PSHB_MDNS_PROBING) return 0;
    uint16_t qd = rd16(p + 4), an = rd16(p + 6), ns = rd16(p + 8);
    size_t off = HDR;
    unsigned ans = 0, add = 0;
    bool all_qu = true;
    bool ours = false;  // a question for one of our own (unique) names
    uint8_t name[NAME_MAX_WIRE];
    for (uint16_t i = 0; i < qd; i++) {
        uint16_t type, cls;
        if (!read_question(p, len, &off, name, &type, &cls)) return 0;
        uint16_t k = cls & 0x7FFF;
        if (k != C_IN && k != C_ANY) continue;
        unsigned a = 0, x = 0;
        if (name_eq(name, m->cur.host)) {
            ours = true;
            if (type == T_A || type == T_ANY) {
                a = BIT(R_A);
                x = BIT(R_NSEC);
            } else {
                a = BIT(R_NSEC);  // no such record (AAAA, ...): say which there are
            }
        } else if (name_eq(name, m->cur.inst)) {
            ours = true;
            if (type == T_SRV || type == T_ANY) {
                a |= BIT(R_SRV);
                x = BIT(R_A) | BIT(R_NSEC);
            }
            if (type == T_TXT || type == T_ANY) a |= BIT(R_TXT);
        } else if (name_eq(name, k_http)) {
            if (type == T_PTR || type == T_ANY) {
                a = BIT(R_PTR);
                x = BIT(R_SRV) | BIT(R_TXT) | BIT(R_A) | BIT(R_NSEC);
            }
        } else if (name_eq(name, k_services)) {
            if (type == T_PTR || type == T_ANY) a = BIT(R_SERVICES);
        }
        if (!a) continue;
        ans |= a;
        add |= x;
        if (!(cls & C_TOP)) all_qu = false;
    }
    bool probe = ns > 0 && ours;
    if (!answering) {
        if (probe) tiebreak(m, p, len, off, an, ns, now_ms);
        return 0;
    }
    if (!ans) return 0;

    // Known answers: what the asker holds with at least half its TTL left is
    // not sent again (RFC 6762 section 7.1).
    rr_t rr;
    for (; an > 0; an--) {
        if (!read_rr(p, len, &off, &rr)) break;
        int r = our_record(&m->cur, p, &rr);
        if (r >= 0 && rr.ttl >= rec_ttl(r) / 2) {
            ans &= ~BIT(r);
            add &= ~BIT(r);
        }
    }
    add &= ~ans;
    if (!ans) return 0;

    // A plain DNS resolver (not from port 5353) gets its answer directly; so
    // does an mDNS querier asking for unicast when the records went to the
    // group recently (else everyone may as well hear them, RFC 6762 5.4).
    bool legacy = src_port != PSHB_MDNS_PORT;
    bool unicast = legacy || (all_qu && !probe && recently_sent(m, ans, now_ms));
    if (unicast) {
        if (!unicast_allow(m, now_ms)) return 0;
    } else {
        unsigned fresh = sent_within(m, probe ? PSHB_MDNS_DEFEND_GAP_MS : PSHB_MDNS_MCAST_GAP_MS, now_ms);
        ans &= ~fresh;
        add &= ~fresh;
        if (!ans) return 0;
    }
    resp_t o = {.legacy = legacy, .id = rd16(p), .q = p, .q_len = len, .q_count = qd};
    unsigned sent;
    size_t n = w_response(&m->cur, ans, add, &o, out, cap, &sent);
    if (n && !unicast) mark_sent(m, sent, now_ms);
    *dest = unicast ? PSHB_MDNS_TO_SENDER : PSHB_MDNS_TO_GROUP;
    return n;
}

static bool on_link(const pshb_mdns_info_t *i, const uint8_t src[4]) {
    for (int k = 0; k < 4; k++) {
        if ((src[k] & i->mask[k]) != (i->ip[k] & i->mask[k])) return false;
    }
    return true;
}

size_t pshb_mdns_input(pshb_mdns_t *m, const uint8_t *pkt, size_t len, const uint8_t src_ip[4],
                       uint16_t src_port, uint64_t now_ms, uint8_t *out, size_t cap,
                       pshb_mdns_dest_t *dest) {
    *dest = PSHB_MDNS_TO_GROUP;
    if (len < HDR || m->phase == PSHB_MDNS_OFF) return 0;
    uint16_t flags = rd16(pkt + 2);
    if (flags & (F_OPCODE | F_RCODE)) return 0;  // RFC 6762 section 18.3, 18.11
    if (flags & F_QR) {
        // Responses come from port 5353 or are ignored (section 6).
        if (src_port == PSHB_MDNS_PORT) on_response(m, pkt, len, now_ms);
        return 0;
    }
    // Queries from outside the board's network are not answered (section 5.5).
    if (!on_link(&m->info, src_ip)) return 0;
    return on_query(m, pkt, len, src_port, now_ms, out, cap, dest);
}

size_t pshb_mdns_poll(pshb_mdns_t *m, uint64_t now_ms, uint8_t *out, size_t cap) {
    unsigned sent;
    if (m->goodbye) {
        resp_t o = {.goodbye = true};
        unsigned bits = m->goodbye;
        m->goodbye = 0;
        size_t n = w_response(&m->old, bits, 0, &o, out, cap, &sent);
        if (n) return n;
    }
    if (m->phase == PSHB_MDNS_WAITING && now_ms >= m->next_ms) {
        m->use_fallback = false;
        use_names(m);
        start_probe(m, now_ms);
    }
    if ((m->phase != PSHB_MDNS_PROBING && m->phase != PSHB_MDNS_ANNOUNCING) || now_ms < m->next_ms) return 0;
    if (m->phase == PSHB_MDNS_PROBING) {
        if (m->step < PSHB_MDNS_PROBES) {
            m->step++;
            m->next_ms = now_ms + PSHB_MDNS_PROBE_MS;
            return w_probe(&m->cur, out, cap);
        }
        // Nobody objected within 250 ms of the last probe: the names are ours.
        m->phase = PSHB_MDNS_ANNOUNCING;
        m->step = 0;
    }
    resp_t o = {0};
    size_t n = w_response(&m->cur, ALL_RECORDS, 0, &o, out, cap, &sent);
    mark_sent(m, sent, now_ms);
    if (++m->step >= PSHB_MDNS_ANNOUNCES) {
        m->phase = PSHB_MDNS_READY;
    } else {
        m->next_ms = now_ms + PSHB_MDNS_ANNOUNCE_MS;
    }
    return n;
}
