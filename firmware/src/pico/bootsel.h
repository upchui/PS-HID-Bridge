// BOOTSEL button at runtime (factory reset when held for 5 s).
#ifndef PSHB_BOOTSEL_H
#define PSHB_BOOTSEL_H

#include <stdbool.h>

// Reads the button by briefly floating the flash chip select. Safe only
// because both cores run from RAM; call from core 1.
bool bootsel_pressed(void);

#endif
