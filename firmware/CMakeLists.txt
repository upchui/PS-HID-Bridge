cmake_minimum_required(VERSION 3.13...3.31)

# ---------------------------------------------------------------------------
# SDK and library locations (all pinned git submodules, see versions.env)
# ---------------------------------------------------------------------------
set(PSHB_LIB ${CMAKE_CURRENT_LIST_DIR}/lib)
set(PICO_SDK_PATH ${PSHB_LIB}/pico-sdk)
set(PICO_TINYUSB_PATH ${PSHB_LIB}/tinyusb)
list(APPEND PICO_BOARD_HEADER_DIRS ${CMAKE_CURRENT_LIST_DIR}/boards)
set(PICO_BOARD wiznet_w5500_evb_pico2)
set(PICO_PLATFORM rp2350-arm-s)

include(${PICO_SDK_PATH}/pico_sdk_init.cmake)

project(ps5_hid_bridge VERSION 0.9.0 LANGUAGES C CXX ASM)
set(CMAKE_C_STANDARD 11)

# Build number = number of commits; it is the image's binary version, which
# the bootrom uses to choose between firmware slots A and B (newer wins).
execute_process(
    COMMAND git rev-list --count HEAD
    WORKING_DIRECTORY ${CMAKE_CURRENT_LIST_DIR}
    OUTPUT_VARIABLE PSHB_BUILD
    OUTPUT_STRIP_TRAILING_WHITESPACE
    ERROR_QUIET
)
if(NOT PSHB_BUILD)
    set(PSHB_BUILD 0)
endif()
math(EXPR PSHB_FW_VERSION_HEX "(${PROJECT_VERSION_MAJOR} << 16) | (${PROJECT_VERSION_MINOR} << 8) | ${PROJECT_VERSION_PATCH}" OUTPUT_FORMAT HEXADECIMAL)
message(STATUS "PS5 HID Bridge firmware ${PROJECT_VERSION}, build ${PSHB_BUILD}")

pico_sdk_init()

# Portable core shared with the host build (firmware/host).
include(${CMAKE_CURRENT_LIST_DIR}/cmake/core_sources.cmake)
include(${CMAKE_CURRENT_LIST_DIR}/cmake/iolibrary.cmake)

set(PSHB_PICO_SOURCES
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/main.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/bootsel.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/core1.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/flash_store.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/hid_out.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/ota.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/usb_descriptors.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/w5500_port.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/web_server.c
)

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
    target_compile_definitions(${name} PRIVATE ${ARG_DEFINES}
        PSHB_FW_VERSION=${PSHB_FW_VERSION_HEX}
        PSHB_FW_BUILD=${PSHB_BUILD}
    )
    target_compile_options(${name} PRIVATE -Wall -Wextra)
    target_link_libraries(${name} PRIVATE
        pico_stdlib
        pico_multicore
        pico_rand
        pico_sync
        pico_unique_id
        hardware_flash
        hardware_spi
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
        target_compile_definitions(${name} PRIVATE PICO_CRT0_IMAGE_TYPE_TBYB=1)
        target_link_libraries(${name} PRIVATE boot_picobin_headers boot_picoboot_headers)
        pico_hash_binary(${name})
        pico_set_binary_version(${name} MAJOR ${PROJECT_VERSION_MAJOR} MINOR ${PSHB_BUILD})
    endif()
    pico_add_extra_outputs(${name})
endfunction()

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

pshb_add_firmware(ps5-hid-bridge OTA)

# Partition table: A/B firmware slots + settings (see partitions/pt.json).
# Flash it once before the first firmware image (README, "Erstinstallation").
add_custom_command(
    OUTPUT ${CMAKE_BINARY_DIR}/ps5-hid-bridge-pt.uf2
    COMMAND picotool partition create ${CMAKE_CURRENT_LIST_DIR}/partitions/pt.json
            ${CMAKE_BINARY_DIR}/ps5-hid-bridge-pt.uf2
    DEPENDS ${CMAKE_CURRENT_LIST_DIR}/partitions/pt.json
    COMMENT "Creating partition table UF2"
)
add_custom_target(partition_table ALL DEPENDS ${CMAKE_BINARY_DIR}/ps5-hid-bridge-pt.uf2)
pshb_add_firmware(ps5-hid-bridge-plain OTA DEFINES PSHB_PLAINTEXT_DEBUG=1)
pshb_add_firmware(ps5-hid-bridge-rollbacktest OTA DEFINES PSHB_OTA_NO_CONFIRM=1)

# Recovery: erase everything (RAM-only image).
add_executable(ps5-hid-bridge-nuke src/tools/nuke.c)
target_link_libraries(ps5-hid-bridge-nuke PRIVATE pico_stdlib hardware_flash)
pico_set_binary_type(ps5-hid-bridge-nuke no_flash)
pico_add_extra_outputs(ps5-hid-bridge-nuke)
pshb_add_firmware(ps5-hid-bridge-hidtest DEFINES PSHB_TEST_PATTERN=1)
pshb_add_firmware(ps5-hid-bridge-hidtest-mouse8 DEFINES PSHB_TEST_PATTERN=1 PSHB_MOUSE_8BIT=1)
