// The board's names: the name the user gives it ("Wohnzimmer PS5", shown in
// the app, the search and the web interface) and the network name derived
// from it ("wohnzimmer-ps5", sent to the router via DHCP and accepted as
// Host). Without a name of its own the board is "ps-hid-bridge-" and the
// last three bytes of its MAC address in both places.
#ifndef PSHB_BOARD_NAME_H
#define PSHB_BOARD_NAME_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

// Room for a name or network name, NUL included.
#define PSHB_NAME_SIZE 32

// The default name from the MAC address: "ps-hid-bridge-96c33d".
void pshb_board_name(const uint8_t mac[6], char *out, size_t cap);

// A name the user may give: at most PSHB_NAME_SIZE - 1 bytes of valid
// UTF-8, no control characters, no spaces at either end. Empty = no name of
// its own (the default name).
bool pshb_name_valid(const char *name);

// The network name for `name`: lower-case letters and digits, ä ö ü ß as
// ae oe ue ss, everything else one "-", no "-" at either end, at most
// PSHB_NAME_SIZE - 1 characters. `fallback` if nothing is left (or `name`
// is empty).
void pshb_hostname_from_name(const char *name, const char *fallback, char *out, size_t cap);

#endif
