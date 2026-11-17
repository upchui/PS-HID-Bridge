// Multicast DNS (RFC 6762) and DNS-SD (RFC 6763) responder: the board
// answers as "<network name>.local" and announces its web interface as an
// "_http._tcp" service. Portable and without sockets: the board (core1.c)
// and the simulator move the datagrams.
//
// Records (TTL in seconds; "unique" ones carry the cache-flush bit):
//   <host>.local                   A     <ip>                    120  unique
//   <host>.local                   NSEC  only A exists           120  unique
//   _services._dns-sd._udp.local   PTR   _http._tcp.local        4500 shared
//   _http._tcp.local               PTR   <instance>._http._tcp   4500 shared
//   <instance>._http._tcp.local    SRV   0 0 <port> <host>.local 120  unique
//   <instance>._http._tcp.local    TXT   "path=/"                4500 unique
//
// Start: 3 probes 250 ms apart for both unique names, then 2 announcements
// 1 s apart. Another device using either name (it answers a probe, or
// announces it later) makes the board probe again, and on a conflict while
// probing it takes its default name for both (no "-2": the web server
// only accepts its own names as Host). When the default name is taken too,
// it stays silent and tries again from the start after a minute.
#ifndef PSHB_MDNS_H
#define PSHB_MDNS_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "board_name.h"

#define PSHB_MDNS_PORT 5353
// 224.0.0.251 and its Ethernet address.
#define PSHB_MDNS_GROUP_INIT {224, 0, 0, 251}
#define PSHB_MDNS_GROUP_MAC_INIT {0x01, 0x00, 0x5e, 0x00, 0x00, 0xfb}

#define PSHB_MDNS_HOST_TTL 120    // records with the host name: A, NSEC, SRV
#define PSHB_MDNS_OTHER_TTL 4500  // PTR, TXT
#define PSHB_MDNS_LEGACY_TTL 10   // at most, in answers to plain DNS resolvers

#define PSHB_MDNS_PROBES 3
#define PSHB_MDNS_PROBE_MS 250
#define PSHB_MDNS_ANNOUNCES 2
#define PSHB_MDNS_ANNOUNCE_MS 1000
// The same record goes to the group at most once per second (a quarter of
// that when it defends the name against another device's probe).
#define PSHB_MDNS_MCAST_GAP_MS 1000
#define PSHB_MDNS_DEFEND_GAP_MS 250
// Both names taken: silent this long, then everything from the start.
#define PSHB_MDNS_RETRY_MS 60000
// Lost a tie with another device probing for the same name at the same time.
#define PSHB_MDNS_DEFER_MS 1000
// Unicast answers per second at most (each may cost core 1 an ARP wait).
#define PSHB_MDNS_MAX_UNICAST_PER_S 8

// Room for any packet built here (the largest, an announcement with the
// longest names, takes about 260 bytes; a legacy answer repeats the
// question, so more room is used if given).
#define PSHB_MDNS_PACKET_MIN 512

typedef struct {
    uint8_t ip[4];    // 0.0.0.0 = no address: silent
    uint8_t mask[4];  // queries from outside ip/mask are ignored; 0.0.0.0 = no check
    char host[PSHB_NAME_SIZE];      // network name ("wohnzimmer-ps5"), as sent by DHCP
    char fallback[PSHB_NAME_SIZE];  // default name ("ps-hid-bridge-96c33d"), after a conflict
    char instance[PSHB_NAME_SIZE];  // service instance: the name shown ("Wohnzimmer PS5")
    uint16_t http_port;
} pshb_mdns_info_t;

typedef enum {
    PSHB_MDNS_OFF = 0,     // no address or name
    PSHB_MDNS_PROBING,
    PSHB_MDNS_ANNOUNCING,  // the names are ours: answering, announcing
    PSHB_MDNS_READY,       // answering
    PSHB_MDNS_WAITING,     // both names taken; probing again later
} pshb_mdns_phase_t;

typedef enum {
    PSHB_MDNS_TO_GROUP,   // 224.0.0.251:5353
    PSHB_MDNS_TO_SENDER,  // unicast to the query's source address and port
} pshb_mdns_dest_t;

enum { PSHB_MDNS_RECORDS = 6 };

// The records' names and data in DNS wire format (internal).
typedef struct {
    uint8_t ip[4];
    uint16_t port;
    uint8_t host[PSHB_NAME_SIZE + 8];   // <host>.local
    uint8_t inst[PSHB_NAME_SIZE + 19];  // <instance>._http._tcp.local
} pshb_mdns_set_t;

// One record compared when two devices probe at the same time (internal).
typedef struct {
    uint16_t cls, type, len;
    uint8_t data[6 + 256];
} pshb_mdns_canon_t;

// Start from a zeroed state (static or memset), then pshb_mdns_reset.
typedef struct {
    pshb_mdns_info_t info;
    pshb_mdns_phase_t phase;
    bool use_fallback;
    char host[PSHB_NAME_SIZE];  // host name in use (or being probed)
    char inst[PSHB_NAME_SIZE];  // instance name in use (or being probed)
    pshb_mdns_set_t cur;
    uint8_t step;  // probes or announcements sent in this phase
    uint64_t next_ms;
    // When each record last went to the group (valid if its bit is set).
    uint64_t mcast_ms[PSHB_MDNS_RECORDS];
    uint8_t mcast_known;
    uint64_t ucast_window_ms;
    uint32_t ucast_sent;
    // Records announced before the last reset that are gone now: sent once
    // with TTL 0 ("goodbye") by the next pshb_mdns_poll.
    pshb_mdns_set_t old;
    uint8_t goodbye;
    pshb_mdns_canon_t scratch[6];
} pshb_mdns_t;

// (Re)start with this address and these names: probe and announce anew (a
// new address, link up, the chip set up again, a new name). Records that
// were announced and changed are taken back with the next poll.
void pshb_mdns_reset(pshb_mdns_t *m, const pshb_mdns_info_t *info, uint64_t now_ms);

// A datagram that arrived on port 5353 (to the group or to the board).
// Returns the length of the answer in `out` (0 = none) and where it goes:
// a plain DNS query (source port not 5353) is answered to its sender only;
// a query asking for a unicast answer ("QU") gets one if the records went
// to the group recently, else the answer goes to the group. `out` should
// hold PSHB_MDNS_PACKET_MIN bytes at least. Malformed packets are ignored.
size_t pshb_mdns_input(pshb_mdns_t *m, const uint8_t *pkt, size_t len, const uint8_t src_ip[4],
                       uint16_t src_port, uint64_t now_ms, uint8_t *out, size_t cap,
                       pshb_mdns_dest_t *dest);

// Call regularly (every few ms to some tens of ms): a probe, announcement
// or goodbye that is due, to be sent to the group; 0 = nothing now.
size_t pshb_mdns_poll(pshb_mdns_t *m, uint64_t now_ms, uint8_t *out, size_t cap);

// The host name answered under .local right now (without ".local"), NULL
// while probing or silent.
const char *pshb_mdns_name(const pshb_mdns_t *m);

#endif
