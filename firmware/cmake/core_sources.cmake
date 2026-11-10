# Portable C sources that build both for the RP2350 and natively on the host
# (unit tests, board simulator). They must not include any pico-sdk header.

set(PSHB_FW_DIR ${CMAKE_CURRENT_LIST_DIR}/..)

set(PSHB_CORE_SOURCES
    ${PSHB_FW_DIR}/src/core/bridge.c
    ${PSHB_FW_DIR}/src/core/crc32.c
    ${PSHB_FW_DIR}/src/core/hexutil.c
    ${PSHB_FW_DIR}/src/core/hid_desc.c
    ${PSHB_FW_DIR}/src/core/http.c
    ${PSHB_FW_DIR}/src/core/input_state.c
    ${PSHB_FW_DIR}/src/core/json.c
    ${PSHB_FW_DIR}/src/core/packet.c
    ${PSHB_FW_DIR}/src/core/password.c
    ${PSHB_FW_DIR}/src/core/proto.c
    ${PSHB_FW_DIR}/src/core/settings.c
    ${PSHB_FW_DIR}/src/core/test_pattern.c
    ${PSHB_FW_DIR}/src/core/web.c
    ${PSHB_FW_DIR}/lib/monocypher/src/monocypher.c
)

include(${CMAKE_CURRENT_LIST_DIR}/embed_web.cmake)
list(APPEND PSHB_CORE_SOURCES ${PSHB_WEB_SOURCES})

set(PSHB_CORE_INCLUDES
    ${PSHB_FW_DIR}/src/core
    ${PSHB_FW_DIR}/include
    ${PSHB_FW_DIR}/lib/monocypher/src
    ${PSHB_WEB_GEN_DIR}
)
