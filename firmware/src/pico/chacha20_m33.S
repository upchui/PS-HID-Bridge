/*
 * ChaCha20 block function for the Cortex-M33 (RP2350), the same as
 * pshb_chacha20_block_c in core/aead.c: out = 20 rounds of in, plus in.
 *
 *   void pshb_chacha20_block_m33(uint32_t out[16], const uint32_t in[16]);
 *
 * `out` and `in` must not overlap. The firmware compares it with the C
 * version at start and uses it only if both agree (main.c).
 *
 * Two things make it about 2.5 times faster than the compiled C:
 *  - 14 of the 16 state words stay in registers (r0-r12, lr); two of
 *    x8..x11 live on the stack and are swapped in pairs twice per double
 *    round (x8/x9 for columns 0 and 1 and diagonals 2 and 3, x10/x11 for
 *    the others).
 *  - Lazy rotation: b and d are kept rotated (b = ror(B, 25), d = ror(D, 24))
 *    and every rotation folds into the barrel shifter of the next add/eor,
 *    so a quarter round is 8 instead of 12 instructions.
 */
    .syntax unified
    .thumb
    .text

/* Stack frame below the saved registers. */
#define OUT  0
#define IN   4
#define X8   16
#define X9   20
#define X10  24
#define X11  28
#define FRAME 36

/* Quarter round on (a, b, c, d) with b = ror(B, 25), d = ror(D, 24) before
 * and after:  a += b; d ^= a; d <<<= 16; c += d; b ^= c; b <<<= 12;
 *             a += b; d ^= a; d <<<= 8;  c += d; b ^= c; b <<<= 7. */
.macro QR a, b, c, d
    add  \a, \a, \b, ror #25
    eor  \d, \a, \d, ror #24
    add  \c, \c, \d, ror #16
    eor  \b, \c, \b, ror #25
    add  \a, \a, \b, ror #20
    eor  \d, \a, \d, ror #16
    add  \c, \c, \d, ror #24
    eor  \b, \c, \b, ror #20
.endm

    .align 2
    .global pshb_chacha20_block_m33
    .type pshb_chacha20_block_m33, %function
    .thumb_func
pshb_chacha20_block_m33:
    push  {r4-r11, lr}
    sub   sp, sp, #FRAME
    str   r0, [sp, #OUT]
    str   r1, [sp, #IN]

    /* x10, x11 to the stack; x8, x9 and x12..x15 into registers. */
    ldrd  r2, r3, [r1, #40]
    strd  r2, r3, [sp, #X10]
    ldrd  r8, r9, [r1, #32]
    ldrd  r10, r11, [r1, #48]
    ldr   r12, [r1, #56]
    ldr   lr, [r1, #60]
    ldm   r1, {r0-r7}            /* x0..x7 (r1 itself last, no writeback) */

    /* Enter the rotated representation: B = ror(b, 7), D = ror(d, 8). */
    ror   r4, r4, #7
    ror   r5, r5, #7
    ror   r6, r6, #7
    ror   r7, r7, #7
    ror   r10, r10, #8
    ror   r11, r11, #8
    ror   r12, r12, #8
    ror   lr, lr, #8

    .rept 10                      /* 20 rounds, unrolled */
    QR    r0, r4, r8, r10         /* column 0: x0 x4 x8  x12 */
    QR    r1, r5, r9, r11         /* column 1: x1 x5 x9  x13 */
    strd  r8, r9, [sp, #X8]
    ldrd  r8, r9, [sp, #X10]
    QR    r2, r6, r8, r12         /* column 2: x2 x6 x10 x14 */
    QR    r3, r7, r9, lr          /* column 3: x3 x7 x11 x15 */
    QR    r0, r5, r8, lr          /* diagonal 0: x0 x5 x10 x15 */
    QR    r1, r6, r9, r10         /* diagonal 1: x1 x6 x11 x12 */
    strd  r8, r9, [sp, #X10]
    ldrd  r8, r9, [sp, #X8]
    QR    r2, r7, r8, r11         /* diagonal 2: x2 x7 x8  x13 */
    QR    r3, r4, r9, r12         /* diagonal 3: x3 x4 x9  x14 */
    .endr

    /* Leave the rotated representation. */
    ror   r4, r4, #25
    ror   r5, r5, #25
    ror   r6, r6, #25
    ror   r7, r7, #25
    ror   r10, r10, #24
    ror   r11, r11, #24
    ror   r12, r12, #24
    ror   lr, lr, #24

    /* out = state ... */
    strd  r8, r9, [sp, #X8]
    ldr   r8, [sp, #OUT]
    stmia r8!, {r0-r7}
    ldrd  r0, r1, [sp, #X8]
    ldrd  r2, r3, [sp, #X10]
    stmia r8!, {r0-r3}
    stmia r8, {r10, r11, r12, lr}

    /* ... + in */
    ldr   r8, [sp, #OUT]
    ldr   r9, [sp, #IN]
    .rept 4
    ldmia r9!, {r0-r3}
    ldmia r8, {r4-r7}
    add   r4, r4, r0
    add   r5, r5, r1
    add   r6, r6, r2
    add   r7, r7, r3
    stmia r8!, {r4-r7}
    .endr

    /* No state words left on the stack. */
    movs  r0, #0
    movs  r1, #0
    strd  r0, r1, [sp, #X8]
    strd  r0, r1, [sp, #X10]
    add   sp, sp, #FRAME
    pop   {r4-r11, pc}
    .size pshb_chacha20_block_m33, . - pshb_chacha20_block_m33
