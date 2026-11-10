// Core 1: network management (link, DHCP/static IP, socket lifecycle) and,
// from step 6, the web interface. Never on the latency-critical path.
#ifndef PSHB_CORE1_H
#define PSHB_CORE1_H

void core1_main(void);

#endif
