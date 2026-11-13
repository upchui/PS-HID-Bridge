// Web interface on the W5500: TCP port 80, two connections (sockets 2-3).
// Core 1 only.
#ifndef PSHB_WEB_SERVER_H
#define PSHB_WEB_SERVER_H

#include "web.h"

void web_server_init(const web_backend_t *be, pshb_settings_t *settings);

// Serve pending connections; never blocks for long (1 KiB SPI bursts).
void web_server_service(void);

// A connection is open: call web_server_service() without pause.
bool web_server_busy(void);

// Before a reboot: wait (at most timeout_ms) until the connections being
// closed are closed, so the last answer reaches the browser or the app.
void web_server_drain(uint32_t timeout_ms);

#endif
