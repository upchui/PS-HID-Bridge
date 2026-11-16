#include "clock.h"

#include "hardware/clocks.h"
#include "hardware/structs/qmi.h"
#include "hardware/structs/watchdog.h"
#include "hardware/vreg.h"
#include "hardware/watchdog.h"
#include "pico/time.h"

#define TRIAL_MARK 0x50534843u  // "PSHC" in watchdog scratch 0 (the boot ROM uses 4..7)
#define TRIAL_WATCHDOG_MS 2000

static uint32_t s_cpu_hz;
static bool s_fell_back;
static bool s_trial;
static absolute_time_t s_trial_end;

void clock_init(const pshb_settings_t *s) {
    bool failed = watchdog_hw->scratch[0] == TRIAL_MARK;
    watchdog_hw->scratch[0] = 0;
    if (s->cpu_200mhz && failed) {
        s_fell_back = true;
    } else if (s->cpu_200mhz) {
        // The flash (only read for settings and updates; the firmware runs
        // from RAM) gets a slower clock divider first, so it stays at most
        // 200 / 3 = 67 MHz. Then, as the SDK does for 200 MHz on the RP2040,
        // a little more core voltage before the faster clock.
        uint32_t t = qmi_hw->m[0].timing;
        if ((t & QMI_M0_TIMING_CLKDIV_BITS) < 3) {
            qmi_hw->m[0].timing = (t & ~QMI_M0_TIMING_CLKDIV_BITS) | (3u << QMI_M0_TIMING_CLKDIV_LSB);
        }
        vreg_set_voltage(VREG_VOLTAGE_1_15);
        busy_wait_us(1000);
        if (set_sys_clock_khz(200000, false)) {
            // The SDK moves clk_peri to the 48 MHz USB PLL on a clock change;
            // keep it at the system clock, so the SPI divider gives 25 and
            // 33.3 MHz (and the UART is set up after this anyway).
            clock_configure_undivided(clk_peri, 0, CLOCKS_CLK_PERI_CTRL_AUXSRC_VALUE_CLKSRC_PLL_SYS,
                                      200 * 1000 * 1000);
            watchdog_hw->scratch[0] = TRIAL_MARK;
            watchdog_enable(TRIAL_WATCHDOG_MS, true);
            s_trial = true;
            s_trial_end = make_timeout_time_ms(CLOCK_TRIAL_MS);
        }
    }
    s_cpu_hz = clock_get_hz(clk_sys);
}

void clock_service(void) {
    if (!s_trial) return;
    watchdog_update();
    if (time_reached(s_trial_end)) {
        watchdog_hw->scratch[0] = 0;  // got through: the next start may use 200 MHz again
        watchdog_disable();
        s_trial = false;
    }
}

void clock_before_reboot(void) {
    watchdog_hw->scratch[0] = 0;
}

uint32_t clock_cpu_hz(void) {
    return s_cpu_hz;
}

bool clock_fell_back(void) {
    return s_fell_back;
}
