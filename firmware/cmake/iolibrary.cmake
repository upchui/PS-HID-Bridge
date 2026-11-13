# WIZnet ioLibrary_Driver (W5500 driver, socket API, DHCP client).
#
# We compile a generated copy of dhcp.c with two changes:
#  * the DHCP host name is "ps-hid-bridge-" + last 3 MAC bytes instead of
#    "WIZnet...", so the board is easy to find in the router's client list;
#  * after an address conflict, dhcp.c waits for its 1 s tick in a busy
#    loop -- but that tick is advanced by our own loop on core 1, which is
#    stuck in there: core 1 would hang forever. It sleeps 1 s instead
#    (pshb_dhcp_decline_wait, core1.c).

set(PSHB_IOLIB ${PSHB_LIB}/ioLibrary_Driver)

file(READ ${PSHB_IOLIB}/Internet/DHCP/dhcp.c PSHB_DHCP_SRC)
function(pshb_patch_dhcp from to what)
    string(FIND "${PSHB_DHCP_SRC}" "${from}" pos)
    if(pos EQUAL -1)
        message(FATAL_ERROR "ioLibrary dhcp.c changed: ${what} not found")
    endif()
    string(REPLACE "${from}" "${to}" src "${PSHB_DHCP_SRC}")
    set(PSHB_DHCP_SRC "${src}" PARENT_SCOPE)
endfunction()
pshb_patch_dhcp("uint8_t HOST_NAME[] = DCHP_HOST_NAME;"
                "uint8_t HOST_NAME[] = \"ps-hid-bridge-\";"
                "HOST_NAME definition")
pshb_patch_dhcp("while((dhcp_tick_1s - ret) < 2) ;"
                "{ extern void pshb_dhcp_decline_wait(void); pshb_dhcp_decline_wait(); }"
                "busy wait after DHCP DECLINE")
# Written via a temporary file: configure_file leaves dhcp.c (and with it
# the object and all images) untouched when nothing changed.
file(WRITE ${CMAKE_BINARY_DIR}/gen/iolibrary/dhcp.c.new "${PSHB_DHCP_SRC}")
configure_file(${CMAKE_BINARY_DIR}/gen/iolibrary/dhcp.c.new ${CMAKE_BINARY_DIR}/gen/iolibrary/dhcp.c COPYONLY)

add_library(iolibrary STATIC
    ${PSHB_IOLIB}/Ethernet/wizchip_conf.c
    ${PSHB_IOLIB}/Ethernet/socket.c
    ${PSHB_IOLIB}/Ethernet/W5500/w5500.c
    ${CMAKE_BINARY_DIR}/gen/iolibrary/dhcp.c
)
target_include_directories(iolibrary PUBLIC
    ${PSHB_IOLIB}/Ethernet
    ${PSHB_IOLIB}/Ethernet/W5500
    ${PSHB_IOLIB}/Internet/DHCP
)
target_compile_definitions(iolibrary PUBLIC _WIZCHIP_=W5500)
# Third-party code: keep its warnings out of our build output.
target_compile_options(iolibrary PRIVATE -w)
