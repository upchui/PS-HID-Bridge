// Log of the controller-mode test images (usbhosttest, dspassthrough).
//
// Every printf() line gets a timestamp and goes into a RAM ring buffer that
// survives a watchdog restart. From there it goes to UART0 (115200 baud, as
// far as the UART keeps up -- nothing waits for it) and over UDP to whoever
// subscribed (tools/boardlog.py): a datagram "sub" to port NETLOG_PORT makes
// the board send everything it still has since boot, and then every new line,
// to the sender; "ka" keeps that up (sent every few seconds; after 30 s
// without one the board stops sending). Any other text is a command for the
// image (netlog_on_command).
//
// Core 0 runs netlog_service(); printf() may be called on both cores, but
// not from an interrupt handler.
#ifndef PSHB_NETLOG_H
#define PSHB_NETLOG_H

#include <stdbool.h>
#include <stdint.h>

#include "settings.h"

#define NETLOG_PORT 45046

// First thing in main(): the ring buffer, the stdio driver and UART0.
void netlog_init(void);

// After w5500_port_init(): the network as stored in the settings (DHCP or
// static address), the log socket once the link is up.
void netlog_start_network(const pshb_settings_t *s, bool chip_ok);

// Core 0 main loop.
void netlog_service(void);

// Text of a command datagram (trimmed), called from netlog_service().
typedef void (*netlog_cmd_fn)(const char *cmd);
void netlog_on_command(netlog_cmd_fn fn);

// printf() `len` bytes as hex, 32 per line, each line prefixed with `what`.
void netlog_hex(const char *what, const void *data, uint32_t len);

#endif
