// CPU clock: 150 MHz (the RP2350's rating) or 200 MHz (setting cpu_200mhz).
//
// 200 MHz is beyond the rating, so a start at 200 MHz is on trial: for its
// first CLOCK_TRIAL_MS the hardware watchdog checks that core 0's loop keeps
// running, and a mark in a watchdog scratch register (kept across the
// watchdog's reset, cleared by power-on) says "a 200 MHz start is on
// trial". A start that finds the mark -- the previous one hung or crashed
// -- runs at 150 MHz and reports it (clock_fell_back); core 1 then stores
// 150 MHz in the settings.
#ifndef PSHB_CLOCK_H
#define PSHB_CLOCK_H

#include <stdbool.h>
#include <stdint.h>

#include "settings.h"

#define CLOCK_TRIAL_MS 10000

// First thing at start (before the UART, USB and SPI, which all depend on
// the clock): set the CPU clock the settings ask for.
void clock_init(const pshb_settings_t *s);

// Core 0's loop: feeds the watchdog during the trial and ends it.
void clock_service(void);

// Before a restart on purpose: that is not a failed trial.
void clock_before_reboot(void);

// At 200 MHz the flash gets a clock divider of at least 3 (67 MHz). Every
// flash write or erase ends in the boot ROM's XIP set-up, which restores the
// divider found at boot: call this right after each one, before reading the
// flash again.
void clock_flash_divider(void);

uint32_t clock_cpu_hz(void);
// The settings ask for 200 MHz, but the previous start at 200 MHz did not
// get through its trial: running at 150 MHz.
bool clock_fell_back(void);

#endif
