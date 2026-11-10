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

include(${CMAKE_CURRENT_LIST_DIR}/../cmake/core_sources.cmake)

add_library(pshb_core STATIC ${PSHB_CORE_SOURCES})
target_include_directories(pshb_core PUBLIC ${PSHB_CORE_INCLUDES})

enable_testing()

# Every tests/test_<name>.c becomes one ctest case.
file(GLOB PSHB_TEST_SOURCES CONFIGURE_DEPENDS ${PSHB_FW_DIR}/tests/test_*.c)
foreach(src ${PSHB_TEST_SOURCES})
    get_filename_component(name ${src} NAME_WE)
    add_executable(${name} ${src})
    target_include_directories(${name} PRIVATE ${PSHB_FW_DIR}/tests)
    target_link_libraries(${name} PRIVATE pshb_core)
    add_test(NAME ${name} COMMAND ${name})
endforeach()
