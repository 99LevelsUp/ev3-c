/*
 * Diagnostic: does one power + start command turn motor A, or only repeated ones?
 * Phase 1 sends the pair once and waits 2 s; phase 2 repeats it every 100 ms for 2 s.
 * Every failed call's result is printed: this uses the try variants.
 */
#include <stdio.h>

#include "ev3.h"

static void report(const char *what, Ev3Result result)
{
    if (result != EV3_OK) {
        printf("  %s: %s (%s)\n", what, ev3_error_string(result), ev3_error_message());
    }
}

int main(void)
{
    printf("phase 1: one power+start\n");
    report("set_power", ev3_motor_try_set_power(EV3_PORT_A, 50));
    ev3_sleep_ms(2000);

    printf("phase 2: power+start every 100 ms for 2 s\n");
    for (int i = 0; i < 20; i++) {
        report("set_power", ev3_motor_try_set_power(EV3_PORT_A, 50));
        ev3_sleep_ms(100);
    }

    report("stop", ev3_motor_try_stop(EV3_PORT_A, EV3_STOP_BRAKE));
    printf("done\n");
    return 0;
}
