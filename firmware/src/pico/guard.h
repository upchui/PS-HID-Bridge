// Hardware watchdog for both cores.
//
// Core 0 feeds the watchdog from its main loop, but only while core 1 (DHCP,
// web interface, settings, updates) shows signs of life: a core that hangs
// or crashed restarts the board within a few seconds instead of leaving it
// without web interface (or, if core 1 held the SPI bus, core 0 without
// input) until the power is cut.
//
// It also guards the start at 200 MHz (clock.c: the trial mark in a
// watchdog scratch register) and keeps the boot ROM's trial of new firmware:
// an update that is not confirmed (ota_confirm_if_trial) gets no feeding
// after GUARD_TRIAL_MS, so the board returns to the previous firmware as the
// boot ROM's own trial watchdog would do.
#ifndef PSHB_GUARD_H
#define PSHB_GUARD_H

#include <stdbool.h>

#define GUARD_TIMEOUT_MS 2000         // watchdog load: core 0 must feed within this
#define GUARD_CORE1_SILENCE_MS 1500   // core 1 counts as hung after this without a sign of life
#define GUARD_CORE1_START_MS 5000     // core 1 must reach its loop within this after its launch
#define GUARD_TRIAL_MS 15000          // unconfirmed new firmware: no feeding after this

// First thing at start (before the clock change). `trial_boot`: this start
// runs new firmware on trial (ota.c).
void guard_start(bool trial_boot);

// Core 0's loop.
void guard_service(void);

// Core 0, right after starting core 1.
void guard_core1_launched(void);

// Core 1's loop and its longer waits.
void guard_core1_alive(void);

// The new firmware works and is confirmed (core 0, at start).
void guard_confirmed(void);

// Before a restart on purpose (either core): no more feeding, so the
// restart that follows is not postponed by core 0.
void guard_before_reboot(void);

// The previous start ended because the watchdog was not fed (a core hung).
bool guard_watchdog_reset(void);

#endif
