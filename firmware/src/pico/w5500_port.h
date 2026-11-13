// W5500 glue for the RP2350: SPI0 wiring, ioLibrary callbacks and a
// non-blocking UDP fast path for core 0.
//
// Threading rules (see also core1.c):
//  * Every SPI transaction is serialised by a recursive mutex registered as
//    ioLibrary critical section (never a spinlock: it would mask USB IRQs).
//  * Core 1 performs all socket()/close() calls (socket.c has globals).
//  * Core 0 touches socket 1 only through w5500_udp_recv()/w5500_udp_send_nb(),
//    which use raw register/buffer access and no socket.c state.
#ifndef PSHB_W5500_PORT_H
#define PSHB_W5500_PORT_H

#include <stdbool.h>
#include <stdint.h>

#define W5500_PIN_MISO 16
#define W5500_PIN_CS   17
#define W5500_PIN_SCK  18
#define W5500_PIN_MOSI 19
#define W5500_PIN_RST  20
#define W5500_PIN_INT  21

#define W5500_SOCK_DHCP 0
// Receive buffer of the DHCP socket. The DHCP client copies a whole
// datagram (up to this size) into its message buffer, see core1.c.
#define W5500_DHCP_RX_KB 2
#define W5500_SOCK_UDP  1
#define W5500_SOCK_HTTP0 2
#define W5500_SOCK_HTTP1 3

#define W5500_RX_EMPTY    (-1)
#define W5500_RX_OVERSIZE (-2)

// Reset and initialise the chip (buffers, MAC, retry timing).
// Call once on core 0 before core 1 is launched. Returns false if the chip
// does not answer (VERSIONR != 0x04).
bool w5500_port_init(void);

void w5500_get_mac(uint8_t mac[6]);

// INTn is active low; asserted while socket 1 has unacknowledged RECV.
bool w5500_int_asserted(void);

// Hold the SPI bus across several calls (recursive; the ioLibrary calls
// inside still work). Core 0 takes it once per receive round, so it waits
// for core 1 at most once instead of before every register access.
void w5500_lock(void);
void w5500_unlock(void);

// Read one datagram from a UDP socket without blocking.
// Returns its length, W5500_RX_EMPTY or W5500_RX_OVERSIZE (dropped).
// `*left`: bytes known to be waiting in the socket (0 = ask the chip);
// updated to what is left afterwards, so a round of reads does not ask the
// chip again (0 afterwards = nothing known to be left; new datagrams raise
// INTn again).
int w5500_udp_recv(uint8_t sn, uint8_t *buf, uint16_t cap, uint8_t ip[4], uint16_t *port, uint16_t *left);

// Queue one datagram without waiting for completion. Returns false if the
// previous send is still in flight or the TX buffer is full (datagram dropped).
bool w5500_udp_send_nb(uint8_t sn, const uint8_t *buf, uint16_t len, const uint8_t ip[4],
                       uint16_t port);

// Acknowledge completed sends; call regularly.
void w5500_udp_tx_poll(uint8_t sn);

#endif
