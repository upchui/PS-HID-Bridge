# Version MAJOR.MINOR.<commit count> from tools/version.sh (the file VERSION
# plus the number of commits), shared by the board and the host build. Sets
#   PSHB_VERSION         e.g. 0.9.16
#   PSHB_VERSION_MAJOR / _MINOR / _PATCH
#   PSHB_BUILD           = PSHB_VERSION_PATCH, the commit count; the image's
#                        binary version, which the boot ROM uses to choose
#                        between firmware slots A and B (newer wins)
#   PSHB_FW_VERSION_HEX  0xMMmmpppp as reported in PONG and the web interface
execute_process(
    COMMAND bash ${CMAKE_CURRENT_LIST_DIR}/../../tools/version.sh
    OUTPUT_VARIABLE PSHB_VERSION
    OUTPUT_STRIP_TRAILING_WHITESPACE
    RESULT_VARIABLE _pshb_version_result
)
if(NOT _pshb_version_result EQUAL 0 OR NOT PSHB_VERSION MATCHES "^([0-9]+)\\.([0-9]+)\\.([0-9]+)$")
    message(FATAL_ERROR "tools/version.sh failed or printed '${PSHB_VERSION}'")
endif()
set(PSHB_VERSION_MAJOR ${CMAKE_MATCH_1})
set(PSHB_VERSION_MINOR ${CMAKE_MATCH_2})
set(PSHB_VERSION_PATCH ${CMAKE_MATCH_3})
if(PSHB_VERSION_MAJOR GREATER 255 OR PSHB_VERSION_MINOR GREATER 255 OR PSHB_VERSION_PATCH GREATER 65535)
    message(FATAL_ERROR "Version ${PSHB_VERSION} does not fit 0xMMmmpppp")
endif()
set(PSHB_BUILD ${PSHB_VERSION_PATCH})
math(EXPR PSHB_FW_VERSION_HEX
    "(${PSHB_VERSION_MAJOR} << 24) | (${PSHB_VERSION_MINOR} << 16) | ${PSHB_VERSION_PATCH}"
    OUTPUT_FORMAT HEXADECIMAL)
