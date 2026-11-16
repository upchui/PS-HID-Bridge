# WIZnet ioLibrary_Driver (W5500 driver, socket API, DHCP client).
#
# We compile generated copies of two of its files:
#
# dhcp.c
#  * the DHCP host name is "ps-hid-bridge-" + last 3 MAC bytes instead of
#    "WIZnet...", so the board is easy to find in the router's client list;
#  * after an address conflict, dhcp.c waits for its 1 s tick in a busy
#    loop -- but that tick is advanced by our own loop on core 1, which is
#    stuck in there: core 1 would hang forever. It sleeps 1 s instead
#    (pshb_dhcp_decline_wait, core1.c).
#
# w5500.c
#  * WIZCHIP_READ_BUF / WIZCHIP_WRITE_BUF move long transfers (HTTP up to
#    1 KiB, DHCP up to 2 KiB) in pieces of PSHB_W5500_PIECE bytes and give
#    the SPI bus free in between: core 0 (the input path) then waits at most
#    one piece for it (w5500_port.c gives it priority).

set(PSHB_IOLIB ${PSHB_LIB}/ioLibrary_Driver)
set(PSHB_IOLIB_GEN ${CMAKE_BINARY_DIR}/gen/iolibrary)

# Replace `from` by `to` in the variable `var` (the text of `file`); fails
# the build if the library changed and `from` is gone.
function(pshb_patch var file from to what)
    string(FIND "${${var}}" "${from}" pos)
    if(pos EQUAL -1)
        message(FATAL_ERROR "ioLibrary ${file} changed: ${what} not found")
    endif()
    string(REPLACE "${from}" "${to}" src "${${var}}")
    set(${var} "${src}" PARENT_SCOPE)
endfunction()

# Written via a temporary file: configure_file leaves the copy (and with it
# the object and all images) untouched when nothing changed.
function(pshb_write_patched name text)
    file(WRITE ${PSHB_IOLIB_GEN}/${name}.new "${text}")
    configure_file(${PSHB_IOLIB_GEN}/${name}.new ${PSHB_IOLIB_GEN}/${name} COPYONLY)
endfunction()

file(READ ${PSHB_IOLIB}/Internet/DHCP/dhcp.c PSHB_DHCP_SRC)
pshb_patch(PSHB_DHCP_SRC dhcp.c
           "uint8_t HOST_NAME[] = DCHP_HOST_NAME;"
           "uint8_t HOST_NAME[] = \"ps-hid-bridge-\";"
           "HOST_NAME definition")
pshb_patch(PSHB_DHCP_SRC dhcp.c
           "while((dhcp_tick_1s - ret) < 2) ;"
           "{ extern void pshb_dhcp_decline_wait(void); pshb_dhcp_decline_wait(); }"
           "busy wait after DHCP DECLINE")
pshb_write_patched(dhcp.c "${PSHB_DHCP_SRC}")

file(READ ${PSHB_IOLIB}/Ethernet/W5500/w5500.c PSHB_W5500_SRC)
pshb_patch(PSHB_W5500_SRC w5500.c
           "void     WIZCHIP_READ_BUF (uint32_t AddrSel, uint8_t* pBuf, uint16_t len)"
           "static void pshb_read_piece(uint32_t AddrSel, uint8_t* pBuf, uint16_t len)"
           "WIZCHIP_READ_BUF definition")
pshb_patch(PSHB_W5500_SRC w5500.c
           "void     WIZCHIP_WRITE_BUF(uint32_t AddrSel, uint8_t* pBuf, uint16_t len)"
           "static void pshb_write_piece(uint32_t AddrSel, uint8_t* pBuf, uint16_t len)"
           "WIZCHIP_WRITE_BUF definition")
string(APPEND PSHB_W5500_SRC "
/* ---- PS HID Bridge (firmware/cmake/iolibrary.cmake): transfers in pieces.
 * Each piece is a transaction of its own inside its own critical section;
 * the next one continues at the following address (the 16-bit offset in
 * bits 8..23; buffer offsets wrap inside the socket buffer by themselves). */
#ifndef PSHB_W5500_PIECE
#define PSHB_W5500_PIECE 128
#endif

static uint32_t pshb_next_addr(uint32_t addr, uint16_t n)
{
   return ((((addr >> 8) + n) & 0xFFFFu) << 8) | (addr & 0xFFu);
}

void WIZCHIP_READ_BUF(uint32_t AddrSel, uint8_t* pBuf, uint16_t len)
{
   while (len > PSHB_W5500_PIECE) {
      pshb_read_piece(AddrSel, pBuf, PSHB_W5500_PIECE);
      AddrSel = pshb_next_addr(AddrSel, PSHB_W5500_PIECE);
      pBuf += PSHB_W5500_PIECE;
      len -= PSHB_W5500_PIECE;
   }
   pshb_read_piece(AddrSel, pBuf, len);
}

void WIZCHIP_WRITE_BUF(uint32_t AddrSel, uint8_t* pBuf, uint16_t len)
{
   while (len > PSHB_W5500_PIECE) {
      pshb_write_piece(AddrSel, pBuf, PSHB_W5500_PIECE);
      AddrSel = pshb_next_addr(AddrSel, PSHB_W5500_PIECE);
      pBuf += PSHB_W5500_PIECE;
      len -= PSHB_W5500_PIECE;
   }
   pshb_write_piece(AddrSel, pBuf, len);
}
")
pshb_write_patched(w5500.c "${PSHB_W5500_SRC}")

add_library(iolibrary STATIC
    ${PSHB_IOLIB}/Ethernet/wizchip_conf.c
    ${PSHB_IOLIB}/Ethernet/socket.c
    ${PSHB_IOLIB_GEN}/w5500.c
    ${PSHB_IOLIB_GEN}/dhcp.c
)
target_include_directories(iolibrary PUBLIC
    ${PSHB_IOLIB}/Ethernet
    ${PSHB_IOLIB}/Ethernet/W5500
    ${PSHB_IOLIB}/Internet/DHCP
)
target_compile_definitions(iolibrary PUBLIC _WIZCHIP_=W5500)
# Third-party code: keep its warnings out of our build output.
target_compile_options(iolibrary PRIVATE -w)
