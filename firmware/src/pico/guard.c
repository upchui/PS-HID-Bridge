#include "guard.h"

#include "hardware/sync.h"
#include "hardware/watchdog.h"
#include "pico/time.h"

#define FEED_EVERY_US 50000

static volatile bool s_stop;              // a restart on purpose is under way
static bool s_core1_launched;
static uint32_t s_launch_us;
static volatile bool s_core1_started;
static volatile uint32_t s_core1_seen_us; // core 1's last sign of life (time_us_32)
static bool s_unconfirmed;
static bool s_watchdog_reset;
static uint32_t s_start_us, s_fed_us;

void guard_start(bool trial_boot) {
    // Before watchdog_enable, which writes the scratch register this reads.
    s_watchdog_reset = watchdog_enable_caused_reboot();
    s_unconfirmed = trial_boot;
    s_start_us = s_fed_us = time_us_32();
    watchdog_enable(GUARD_TIMEOUT_MS, true);  // paused while a debugger halts the cores
}

void guard_service(void) {
    uint32_t now = time_us_32();
    if (s_stop || now - s_fed_us < FEED_EVERY_US) return;
    if (s_unconfirmed && now - s_start_us > GUARD_TRIAL_MS * 1000u) return;
    if (s_core1_started) {
        if (now - s_core1_seen_us > GUARD_CORE1_SILENCE_MS * 1000u) return;
    } else if (s_core1_launched && now - s_launch_us > GUARD_CORE1_START_MS * 1000u) {
        return;  // core 1 never got to its loop
    }
    watchdog_update();
    s_fed_us = now;
}

void guard_core1_launched(void) {
    s_launch_us = time_us_32();
    s_core1_launched = true;
}

void guard_core1_alive(void) {
    s_core1_seen_us = time_us_32();
    __dmb();  // core 0 must not see "started" before the time
    s_core1_started = true;
}

void guard_confirmed(void) {
    s_unconfirmed = false;
}

void guard_before_reboot(void) {
    s_stop = true;
}

bool guard_watchdog_reset(void) {
    return s_watchdog_reset;
}
