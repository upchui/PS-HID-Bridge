#include "mini_test.h"
#include "test_pattern.h"

TEST(square_closes) {
    long x = 0, y = 0, max_x = 0, max_y = 0;
    for (uint32_t step = 0; step < 4 * PSHB_TP_STEPS_PER_SIDE; step++) {
        int16_t dx, dy;
        test_pattern_delta(step, &dx, &dy);
        CHECK(dx == 0 || dy == 0);
        x += dx;
        y += dy;
        if (x > max_x) max_x = x;
        if (y > max_y) max_y = y;
    }
    CHECK_EQ_I(x, 0);
    CHECK_EQ_I(y, 0);
    CHECK_EQ_I(max_x, PSHB_TP_STEPS_PER_SIDE * PSHB_TP_STEP_PX);
    CHECK_EQ_I(max_y, PSHB_TP_STEPS_PER_SIDE * PSHB_TP_STEP_PX);
}

TEST(directions) {
    int16_t dx, dy;
    test_pattern_delta(0, &dx, &dy);
    CHECK(dx > 0 && dy == 0);  // right first
    test_pattern_delta(PSHB_TP_STEPS_PER_SIDE, &dx, &dy);
    CHECK(dx == 0 && dy > 0);  // then down (screen coordinates)
}

int main(void) {
    RUN(square_closes);
    RUN(directions);
    return TEST_SUMMARY();
}
