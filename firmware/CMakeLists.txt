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

add_library(pshb_core STATIC ${PSHB_CORE_SOURCES})
target_include_directories(pshb_core PUBLIC ${PSHB_CORE_INCLUDES})
target_compile_options(pshb_core PRIVATE -Wall -Wextra)

# ---------------------------------------------------------------------------
# Main firmware image -> ps5-hid-bridge.uf2
# ---------------------------------------------------------------------------
add_executable(ps5-hid-bridge
    src/pico/main.c
)
target_include_directories(ps5-hid-bridge PRIVATE include)
target_compile_options(ps5-hid-bridge PRIVATE -Wall -Wextra)
target_link_libraries(ps5-hid-bridge PRIVATE pshb_core pico_stdlib)

pico_enable_stdio_uart(ps5-hid-bridge 1)
pico_enable_stdio_usb(ps5-hid-bridge 0)
pico_add_extra_outputs(ps5-hid-bridge)
