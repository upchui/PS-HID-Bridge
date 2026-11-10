// Test pattern for the hidtest image: the mouse slowly traces a square.
// One step every PSHB_TP_STEP_MS; each side has PSHB_TP_STEPS_PER_SIDE steps.
#ifndef PSHB_TEST_PATTERN_H
#define PSHB_TEST_PATTERN_H

#include <stdint.h>

#define PSHB_TP_START_DELAY_MS 5000
#define PSHB_TP_STEP_MS        10
#define PSHB_TP_STEPS_PER_SIDE 100
#define PSHB_TP_STEP_PX        2

// Motion of step number `step` (0, 1, 2, ...): right, down, left, up.
void test_pattern_delta(uint32_t step, int16_t *dx, int16_t *dy);

#endif
