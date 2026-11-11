# Native host build of the portable firmware core: unit tests and (from step 3)
# the board simulator. Built with the system compiler inside the firmware image.
cmake_minimum_required(VERSION 3.20)
project(pshb_host C)

set(CMAKE_C_STANDARD 11)
set(CMAKE_C_STANDARD_REQUIRED ON)

option(PSHB_SANITIZE "Build with AddressSanitizer and UBSan" ON)
if(PSHB_SANITIZE)
    add_compile_options(-fsanitize=address,undefined -fno-omit-frame-pointer -fno-sanitize-recover=all)
    add_link_options(-fsanitize=address,undefined)
endif()

add_compile_options(-Wall -Wextra -Werror)

# The simulator reports the same version as the board images.
include(${CMAKE_CURRENT_LIST_DIR}/../cmake/version.cmake)
add_compile_definitions(PSHB_FW_VERSION=${PSHB_FW_VERSION_HEX})

include(${CMAKE_CURRENT_LIST_DIR}/../cmake/core_sources.cmake)

# Core library variants: default (16-bit mouse) and the 8-bit fallback.
add_library(pshb_core STATIC ${PSHB_CORE_SOURCES})
target_include_directories(pshb_core PUBLIC ${PSHB_CORE_INCLUDES})

add_library(pshb_core_mouse8 STATIC ${PSHB_CORE_SOURCES})
target_include_directories(pshb_core_mouse8 PUBLIC ${PSHB_CORE_INCLUDES})
target_compile_definitions(pshb_core_mouse8 PUBLIC PSHB_MOUSE_8BIT=1)

enable_testing()

# Shared protocol test vectors -> C header (protocol/vectors/v1.json).
set(PSHB_PROTOCOL_DIR ${PSHB_FW_DIR}/../protocol)
find_package(Python3 REQUIRED COMPONENTS Interpreter)
add_custom_command(
    OUTPUT ${CMAKE_BINARY_DIR}/gen/vectors_v1.h
    COMMAND ${CMAKE_COMMAND} -E make_directory ${CMAKE_BINARY_DIR}/gen
    COMMAND Python3::Interpreter ${PSHB_PROTOCOL_DIR}/tools/gen_c_vectors.py
            ${PSHB_PROTOCOL_DIR}/vectors/v1.json ${CMAKE_BINARY_DIR}/gen/vectors_v1.h
    DEPENDS ${PSHB_PROTOCOL_DIR}/vectors/v1.json ${PSHB_PROTOCOL_DIR}/tools/gen_c_vectors.py
)
add_custom_target(pshb_vectors DEPENDS ${CMAKE_BINARY_DIR}/gen/vectors_v1.h)

# Every tests/test_<name>.c becomes one ctest case against the default core.
file(GLOB PSHB_TEST_SOURCES CONFIGURE_DEPENDS ${PSHB_FW_DIR}/tests/test_*.c)
foreach(src ${PSHB_TEST_SOURCES})
    get_filename_component(name ${src} NAME_WE)
    add_executable(${name} ${src})
    target_include_directories(${name} PRIVATE ${PSHB_FW_DIR}/tests ${CMAKE_BINARY_DIR}/gen)
    target_link_libraries(${name} PRIVATE pshb_core)
    add_dependencies(${name} pshb_vectors)
    add_test(NAME ${name} COMMAND ${name})
endforeach()

# Tests that depend on the mouse resolution also run against the fallback.
foreach(name test_hid_desc test_input_state)
    add_executable(${name}_mouse8 ${PSHB_FW_DIR}/tests/${name}.c)
    target_include_directories(${name}_mouse8 PRIVATE ${PSHB_FW_DIR}/tests)
    target_link_libraries(${name}_mouse8 PRIVATE pshb_core_mouse8)
    add_test(NAME ${name}_mouse8 COMMAND ${name}_mouse8)
endforeach()

# Board simulator (step 3): the core behind a UDP socket, see host/sim_main.c.
add_executable(pshb-sim ${CMAKE_CURRENT_LIST_DIR}/sim_main.c)
target_link_libraries(pshb-sim PRIVATE pshb_core)
