#include "bootsel.h"

#include "hardware/gpio.h"
#include "hardware/structs/ioqspi.h"
#include "hardware/structs/sio.h"
#include "hardware/sync.h"

// Same technique as pico-examples/picoboard/button: BOOTSEL pulls QSPI_SS
// low. Disconnect the output driver for a moment and read the pin level.
bool __no_inline_not_in_flash_func(bootsel_pressed)(void) {
    const unsigned cs_index = 1;
    uint32_t irq = save_and_disable_interrupts();
    hw_write_masked(&ioqspi_hw->io[cs_index].ctrl, GPIO_OVERRIDE_LOW << IO_QSPI_GPIO_QSPI_SS_CTRL_OEOVER_LSB,
                    IO_QSPI_GPIO_QSPI_SS_CTRL_OEOVER_BITS);
    for (volatile int i = 0; i < 1000; i++) {
    }
    bool pressed = !(sio_hw->gpio_hi_in & SIO_GPIO_HI_IN_QSPI_CSN_BITS);
    hw_write_masked(&ioqspi_hw->io[cs_index].ctrl, GPIO_OVERRIDE_NORMAL << IO_QSPI_GPIO_QSPI_SS_CTRL_OEOVER_LSB,
                    IO_QSPI_GPIO_QSPI_SS_CTRL_OEOVER_BITS);
    restore_interrupts(irq);
    return pressed;
}
