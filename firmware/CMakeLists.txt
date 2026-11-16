cmake_minimum_required(VERSION 3.13...3.31)

# ---------------------------------------------------------------------------
# SDK and library locations (all pinned git submodules, see versions.env)
# ---------------------------------------------------------------------------
set(PSHB_LIB ${CMAKE_CURRENT_LIST_DIR}/lib)
set(PICO_SDK_PATH ${PSHB_LIB}/pico-sdk)
set(PICO_TINYUSB_PATH ${PSHB_LIB}/tinyusb)
# USB host on GPIO pins (controller-mode test images); TinyUSB's rp2040
# family support builds the target tinyusb_pico_pio_usb from it. We build a
# patched copy: Pico-PIO-USB reads the line state around RP2350-E9 (an
# undriven input with pull-down floats up to ~2.2 V) only on chips with
# revision 2 (A2). A3 chips have the erratum as well and report revision 3,
# like A4, which does not need it but loses nothing by it.
set(PSHB_PIO_USB_GEN ${CMAKE_BINARY_DIR}/gen/pico-pio-usb)
file(COPY ${PSHB_LIB}/pico-pio-usb/src DESTINATION ${PSHB_PIO_USB_GEN})
file(READ ${PSHB_PIO_USB_GEN}/src/pio_usb_ll.h PSHB_PIO_USB_LL)
string(FIND "${PSHB_PIO_USB_LL}" "if (chip_version <= 2) {" PSHB_POS)
if(PSHB_POS EQUAL -1)
    message(FATAL_ERROR "Pico-PIO-USB pio_usb_ll.h changed: the E9 chip check is gone")
endif()
string(REPLACE "if (chip_version <= 2) {" "if (chip_version <= 3) {" PSHB_PIO_USB_LL "${PSHB_PIO_USB_LL}")
file(WRITE ${PSHB_PIO_USB_GEN}/src/pio_usb_ll.h "${PSHB_PIO_USB_LL}")
set(PICO_PIO_USB_PATH ${PSHB_PIO_USB_GEN})
list(APPEND PICO_BOARD_HEADER_DIRS ${CMAKE_CURRENT_LIST_DIR}/boards)
set(PICO_BOARD wiznet_w5500_evb_pico2)
set(PICO_PLATFORM rp2350-arm-s)

include(${PICO_SDK_PATH}/pico_sdk_init.cmake)

# Version MAJOR.MINOR.PATCH (tools/version.sh); it is also the binary version
# of the OTA images (PSHB_BIN_VERSION_MAJOR / PSHB_BUILD).
include(${CMAKE_CURRENT_LIST_DIR}/cmake/version.cmake)

project(ps_hid_bridge VERSION ${PSHB_VERSION} LANGUAGES C CXX ASM)
set(CMAKE_C_STANDARD 11)
message(STATUS "PS HID Bridge firmware ${PROJECT_VERSION}")

pico_sdk_init()

# Portable core shared with the host build (firmware/host).
include(${CMAKE_CURRENT_LIST_DIR}/cmake/core_sources.cmake)
list(APPEND PSHB_CORE_INCLUDES ${PSHB_VERSION_DIR})
include(${CMAKE_CURRENT_LIST_DIR}/cmake/iolibrary.cmake)

set(PSHB_PICO_SOURCES
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/main.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/bootsel.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/chacha20_m33.S
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/clock.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/guard.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/core1.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/crypto_help.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/flash_store.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/hid_out.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/ota.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/usb_descriptors.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/w5500_port.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/web_server.c
)

# pshb_make_ota(<target>): the image can be installed over the network --
# booted "on trial" (TBYB) after an update and confirmed by the firmware
# itself; hashed so the bootrom rejects corrupted images; versioned so the
# bootrom picks the newer slot.
function(pshb_make_ota name)
    target_compile_definitions(${name} PRIVATE PICO_CRT0_IMAGE_TYPE_TBYB=1)
    target_link_libraries(${name} PRIVATE boot_picobin_headers boot_picoboot_headers)
    pico_hash_binary(${name})
    # What pico_set_binary_version(MAJOR .. MINOR ..) does, but only for
    # assembler sources: crt0.S embeds the version, and a define on every
    # C file would rebuild the whole image on each commit. The boot ROM
    # only knows two 16-bit parts, so MAJOR.MINOR becomes the first one
    # (major * 256 + minor) and PATCH the second.
    target_compile_definitions(${name} PRIVATE
        $<$<COMPILE_LANGUAGE:ASM>:PICO_CRT0_VERSION_MAJOR=${PSHB_BIN_VERSION_MAJOR}>
        $<$<COMPILE_LANGUAGE:ASM>:PICO_CRT0_VERSION_MINOR=${PSHB_BUILD}>)
endfunction()

# pshb_add_firmware(<name> [OTA] [DEFINES d1 d2 ...]) -> <name>.uf2
# Each image compiles the core itself so build-time options apply everywhere.
# OTA: image can be installed over the network -- booted "on trial" (TBYB)
# after an update and confirmed by the firmware itself; hashed so the bootrom
# rejects corrupted images; versioned so the bootrom picks the newer slot.
function(pshb_add_firmware name)
    cmake_parse_arguments(ARG "OTA" "" "DEFINES" ${ARGN})
    add_executable(${name} ${PSHB_CORE_SOURCES} ${PSHB_PICO_SOURCES})
    target_include_directories(${name} PRIVATE
        ${PSHB_CORE_INCLUDES}
        ${CMAKE_CURRENT_LIST_DIR}/src/pico
    )
    target_compile_definitions(${name} PRIVATE ${ARG_DEFINES})
    target_compile_options(${name} PRIVATE -Wall -Wextra)
    target_link_libraries(${name} PRIVATE
        pico_stdlib
        pico_multicore
        pico_rand
        pico_sync
        pico_unique_id
        hardware_dma
        hardware_flash
        hardware_spi
        hardware_vreg
        hardware_watchdog
        tinyusb_device
        iolibrary
    )
    # Run entirely from RAM: flash writes (settings, OTA) and BOOTSEL reads
    # disable XIP, which must never stall the input path on core 0.
    pico_set_binary_type(${name} copy_to_ram)
    pico_enable_stdio_uart(${name} 1)
    pico_enable_stdio_usb(${name} 0)
    if(ARG_OTA)
        pshb_make_ota(${name})
    endif()
    pico_add_extra_outputs(${name})
endfunction()

# Mouse reports: armed shortly before the PlayStation fetches them (ON) or
# right after the previous fetch (OFF, for comparison),
# e.g. make firmware FW_CMAKE_ARGS=-DPSHB_MOUSE_LATE_ARM=OFF
option(PSHB_MOUSE_LATE_ARM "Arm mouse reports shortly before the host fetches them" ON)
if(PSHB_MOUSE_LATE_ARM)
    add_compile_definitions(PSHB_MOUSE_LATE_ARM=1)
else()
    add_compile_definitions(PSHB_MOUSE_LATE_ARM=0)
endif()

# UDP receive path: burst reads (ON) or the plain ioLibrary functions (OFF),
# e.g. make firmware FW_CMAKE_ARGS=-DPSHB_W5500_FAST_RX=OFF
option(PSHB_W5500_FAST_RX "Receive UDP with a few SPI burst reads" ON)
if(PSHB_W5500_FAST_RX)
    add_compile_definitions(PSHB_W5500_FAST_RX=1)
else()
    add_compile_definitions(PSHB_W5500_FAST_RX=0)
endif()

# Development PSK until settings exist: make firmware DEV_PSK=<64 hex digits>
set(PSHB_DEV_PSK "" CACHE STRING "Development pre-shared key (64 hex digits)")
if(PSHB_DEV_PSK)
    add_compile_definitions(PSHB_DEV_PSK="${PSHB_DEV_PSK}")
endif()

pshb_add_firmware(ps-hid-bridge OTA)

# Partition table: A/B firmware slots + settings (see partitions/pt.json).
# Flash it once before the first firmware image (README, "Quick start").
add_custom_command(
    OUTPUT ${CMAKE_BINARY_DIR}/ps-hid-bridge-pt.uf2
    COMMAND picotool partition create ${CMAKE_CURRENT_LIST_DIR}/partitions/pt.json
            ${CMAKE_BINARY_DIR}/ps-hid-bridge-pt.uf2
    DEPENDS ${CMAKE_CURRENT_LIST_DIR}/partitions/pt.json
    COMMENT "Creating partition table UF2"
)
add_custom_target(partition_table ALL DEPENDS ${CMAKE_BINARY_DIR}/ps-hid-bridge-pt.uf2)
pshb_add_firmware(ps-hid-bridge-plain OTA DEFINES PSHB_PLAINTEXT_DEBUG=1)
pshb_add_firmware(ps-hid-bridge-rollbacktest OTA DEFINES PSHB_OTA_NO_CONFIRM=1)
# Measurements (docs/latency.md): the normal firmware plus measuring pins
# (GP6, GP7), a summary on the debug UART and the upload sink for load tests.
pshb_add_firmware(ps-hid-bridge-timing OTA DEFINES PSHB_TIMING_GPIO=1 PSHB_TIMING_UART=1 PSHB_BENCH=1)

# Recovery: erase everything (RAM-only image).
add_executable(ps-hid-bridge-nuke src/tools/nuke.c)
target_link_libraries(ps-hid-bridge-nuke PRIVATE pico_stdlib hardware_flash)
pico_set_binary_type(ps-hid-bridge-nuke no_flash)
pico_add_extra_outputs(ps-hid-bridge-nuke)
pshb_add_firmware(ps-hid-bridge-hidtest DEFINES PSHB_TEST_PATTERN=1)
pshb_add_firmware(ps-hid-bridge-hidtest-mouse8 DEFINES PSHB_TEST_PATTERN=1 PSHB_MOUSE_8BIT=1)

# Controller-mode test images (README, "Controller mode"): the USB host port
# (Pico-PIO-USB on core 1, D+ GP2, D- GP3) and a log over the network
# (src/pico/netlog.c, tools/boardlog.py); no input from the app. Installed
# over the network like a firmware update, they never confirm themselves:
# the next reset (or power cut) returns to the normal firmware.
set(PSHB_USBHOST_SOURCES
    ${CMAKE_CURRENT_LIST_DIR}/src/core/crc32.c
    ${CMAKE_CURRENT_LIST_DIR}/src/core/logbuf.c
    ${CMAKE_CURRENT_LIST_DIR}/src/core/settings.c
    ${CMAKE_CURRENT_LIST_DIR}/src/core/uf2_writer.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/clock.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/flash_store.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/netlog.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/ota.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/w5500_port.c
)
function(pshb_add_usbhost_image name)
    cmake_parse_arguments(ARG "" "" "SOURCES;DEFINES;LIBS" ${ARGN})
    add_executable(${name} ${PSHB_USBHOST_SOURCES} ${ARG_SOURCES})
    target_include_directories(${name} PRIVATE
        ${PSHB_CORE_INCLUDES}
        ${CMAKE_CURRENT_LIST_DIR}/src/pico
    )
    target_compile_definitions(${name} PRIVATE ${ARG_DEFINES})
    target_compile_options(${name} PRIVATE -Wall -Wextra)
    target_link_libraries(${name} PRIVATE
        pico_stdlib
        pico_multicore
        pico_unique_id
        hardware_adc
        hardware_dma
        hardware_flash
        hardware_spi
        hardware_vreg
        hardware_watchdog
        tinyusb_host
        tinyusb_pico_pio_usb
        iolibrary
        ${ARG_LIBS}
    )
    pico_set_binary_type(${name} copy_to_ram)
    pshb_make_ota(${name})
    # The log is its own stdio driver (netlog.c), which also feeds UART0.
    pico_enable_stdio_uart(${name} 0)
    pico_enable_stdio_usb(${name} 0)
    pico_add_extra_outputs(${name})
endfunction()
pshb_add_usbhost_image(ps-hid-bridge-usbhosttest
    SOURCES ${CMAKE_CURRENT_LIST_DIR}/src/pico/usbhost_test.c
    DEFINES PSHB_USBHOST_TEST=1)
# A real DualSense on the host port passed through to the PlayStation (its
# authentication included): the go/no-go test of the controller mode.
pshb_add_usbhost_image(ps-hid-bridge-dspassthrough
    SOURCES ${CMAKE_CURRENT_LIST_DIR}/src/pico/dspass.c ${CMAKE_CURRENT_LIST_DIR}/src/core/ds_desc.c
    DEFINES PSHB_DS_PASSTHROUGH=1
    LIBS tinyusb_device)

# The images of this build: the Makefile copies exactly these to out/ (not
# leftovers of renamed or removed targets in the build directory).
set(PSHB_IMAGES ps-hid-bridge ps-hid-bridge-pt ps-hid-bridge-plain ps-hid-bridge-rollbacktest
    ps-hid-bridge-timing ps-hid-bridge-nuke ps-hid-bridge-hidtest ps-hid-bridge-hidtest-mouse8
    ps-hid-bridge-usbhosttest ps-hid-bridge-dspassthrough)
list(TRANSFORM PSHB_IMAGES APPEND ".uf2")
list(JOIN PSHB_IMAGES "\n" PSHB_IMAGES_TEXT)
file(CONFIGURE OUTPUT ${CMAKE_BINARY_DIR}/images.txt CONTENT "${PSHB_IMAGES_TEXT}\n")
