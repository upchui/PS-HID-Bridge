# Portable C sources that build both for the RP2350 and natively on the host
# (unit tests, board simulator). They must not include any pico-sdk header.

set(PSHB_FW_DIR ${CMAKE_CURRENT_LIST_DIR}/..)

set(PSHB_CORE_SOURCES
    ${PSHB_FW_DIR}/src/core/crc32.c
    ${PSHB_FW_DIR}/src/core/hid_desc.c
    ${PSHB_FW_DIR}/src/core/input_state.c
    ${PSHB_FW_DIR}/src/core/test_pattern.c
)

set(PSHB_CORE_INCLUDES
    ${PSHB_FW_DIR}/src/core
    ${PSHB_FW_DIR}/include
)
