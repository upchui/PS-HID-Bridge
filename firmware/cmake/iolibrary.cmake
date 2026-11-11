# WIZnet ioLibrary_Driver (W5500 driver, socket API, DHCP client).
#
# dhcp.c hard-codes the DHCP host name ("WIZnet" + last 3 MAC bytes). We
# compile a generated copy that uses "ps-hid-bridge-" instead, so the board
# is easy to find in the router's client list.

set(PSHB_IOLIB ${PSHB_LIB}/ioLibrary_Driver)

file(READ ${PSHB_IOLIB}/Internet/DHCP/dhcp.c PSHB_DHCP_SRC)
set(PSHB_DHCP_FROM "uint8_t HOST_NAME[] = DCHP_HOST_NAME;")
set(PSHB_DHCP_TO "uint8_t HOST_NAME[] = \"ps-hid-bridge-\";")
string(FIND "${PSHB_DHCP_SRC}" "${PSHB_DHCP_FROM}" PSHB_DHCP_POS)
if(PSHB_DHCP_POS EQUAL -1)
    message(FATAL_ERROR "ioLibrary dhcp.c changed: HOST_NAME definition not found")
endif()
string(REPLACE "${PSHB_DHCP_FROM}" "${PSHB_DHCP_TO}" PSHB_DHCP_SRC "${PSHB_DHCP_SRC}")
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
