# WIZnet ioLibrary_Driver (W5500 driver, socket API, DHCP client).
#
# We compile generated copies of two of its files:
#
# dhcp.c
#  * the DHCP host name is pshb_dhcp_hostname (core1.c: the board's network
#    name, "ps-hid-bridge-" + last 3 MAC bytes or one derived from the name
#    the user gave it) instead of "WIZnet" + MAC bytes, so the board is easy
#    to find in the router's client list;
#  * after an address conflict, dhcp.c waits for its 1 s tick in a busy
#    loop -- but that tick is advanced by our own loop on core 1, which is
#    stuck in there: core 1 would hang forever. It sleeps 1 s instead
#    (pshb_dhcp_decline_wait, core1.c);
#  * two functions are added for core1.c: whether the client holds an
#    acknowledged lease (to follow when it ends: the library itself forgets
#    the lease after a failed renewal) and "renew now" (after a link drop).
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
           "extern char pshb_dhcp_hostname[]; /* PS HID Bridge: core1.c */"
           "HOST_NAME definition")
# Both host name options (DISCOVER and REQUEST): the whole name from
# pshb_dhcp_hostname, no MAC digits appended.
pshb_patch(PSHB_DHCP_SRC dhcp.c
           [=[	for(i = 0 ; HOST_NAME[i] != 0; i++)
   	pDHCPMSG->OPT[k++] = HOST_NAME[i];
	pDHCPMSG->OPT[k++] = NibbleToHex(DHCP_CHADDR[3] >> 4); 
	pDHCPMSG->OPT[k++] = NibbleToHex(DHCP_CHADDR[3]);
	pDHCPMSG->OPT[k++] = NibbleToHex(DHCP_CHADDR[4] >> 4); 
	pDHCPMSG->OPT[k++] = NibbleToHex(DHCP_CHADDR[4]);
	pDHCPMSG->OPT[k++] = NibbleToHex(DHCP_CHADDR[5] >> 4); 
	pDHCPMSG->OPT[k++] = NibbleToHex(DHCP_CHADDR[5]);
	pDHCPMSG->OPT[k - (i+6+1)] = i+6; // length of hostname]=]
           [=[	for(i = 0 ; pshb_dhcp_hostname[i] != 0; i++)
   	pDHCPMSG->OPT[k++] = pshb_dhcp_hostname[i];
	pDHCPMSG->OPT[k - (i+1)] = i; // length of hostname]=]
           "host name option")
string(FIND "${PSHB_DHCP_SRC}" "HOST_NAME[i]" pshb_left)
if(NOT pshb_left EQUAL -1)
    message(FATAL_ERROR "ioLibrary dhcp.c changed: a host name option still uses HOST_NAME")
endif()
pshb_patch(PSHB_DHCP_SRC dhcp.c
           "while((dhcp_tick_1s - ret) < 2) ;"
           "{ extern void pshb_dhcp_decline_wait(void); pshb_dhcp_decline_wait(); }"
           "busy wait after DHCP DECLINE")
string(APPEND PSHB_DHCP_SRC "
/* ---- PS HID Bridge (firmware/cmake/iolibrary.cmake). */
/* An acknowledged lease, not being renewed right now. Every ACK (first one
 * or renewal) enters this state. */
uint8_t pshb_dhcp_bound(void)
{
   return dhcp_state == STATE_DHCP_LEASED;
}

/* Renew the lease at the next DHCP_run (a REQUEST for the same address,
 * as at half its time): after a link drop, the board may be on another
 * network, whose server then answers NAK and the client starts over. */
void pshb_dhcp_renew_now(void)
{
   if (dhcp_state == STATE_DHCP_LEASED && dhcp_lease_time != INFINITE_LEASETIME)
      dhcp_tick_1s = dhcp_lease_time / 2 + 1;
}
")
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
