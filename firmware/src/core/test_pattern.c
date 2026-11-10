#include "test_pattern.h"

void test_pattern_delta(uint32_t step, int16_t *dx, int16_t *dy) {
    static const int8_t dir[4][2] = {{1, 0}, {0, 1}, {-1, 0}, {0, -1}};
    uint32_t side = (step / PSHB_TP_STEPS_PER_SIDE) % 4;
    *dx = (int16_t)(dir[side][0] * PSHB_TP_STEP_PX);
    *dy = (int16_t)(dir[side][1] * PSHB_TP_STEP_PX);
}
