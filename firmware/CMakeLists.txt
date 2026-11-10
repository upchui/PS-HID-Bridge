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

project(ps5_hid_bridge C CXX ASM)
set(CMAKE_C_STANDARD 11)

pico_sdk_init()

# Portable core shared with the host build (firmware/host).
include(${CMAKE_CURRENT_LIST_DIR}/cmake/core_sources.cmake)

set(PSHB_PICO_SOURCES
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/main.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/hid_out.c
    ${CMAKE_CURRENT_LIST_DIR}/src/pico/usb_descriptors.c
)

# pshb_add_firmware(<name> [DEFINES d1 d2 ...]) -> <name>.uf2
# Each image compiles the core itself so build-time options apply everywhere.
function(pshb_add_firmware name)
    cmake_parse_arguments(ARG "" "" "DEFINES" ${ARGN})
    add_executable(${name} ${PSHB_CORE_SOURCES} ${PSHB_PICO_SOURCES})
    target_include_directories(${name} PRIVATE
        ${PSHB_CORE_INCLUDES}
        ${CMAKE_CURRENT_LIST_DIR}/src/pico
    )
    target_compile_definitions(${name} PRIVATE ${ARG_DEFINES})
    target_compile_options(${name} PRIVATE -Wall -Wextra)
    target_link_libraries(${name} PRIVATE
        pico_stdlib
        pico_unique_id
        tinyusb_device
    )
    # Run entirely from RAM: flash writes (settings, OTA) and BOOTSEL reads
    # disable XIP, which must never stall the input path on core 0.
    pico_set_binary_type(${name} copy_to_ram)
    pico_enable_stdio_uart(${name} 1)
    pico_enable_stdio_usb(${name} 0)
    pico_add_extra_outputs(${name})
endfunction()

pshb_add_firmware(ps5-hid-bridge)
pshb_add_firmware(ps5-hid-bridge-hidtest DEFINES PSHB_TEST_PATTERN=1)
pshb_add_firmware(ps5-hid-bridge-hidtest-mouse8 DEFINES PSHB_TEST_PATTERN=1 PSHB_MOUSE_8BIT=1)
