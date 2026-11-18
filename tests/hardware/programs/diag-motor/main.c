/*
 * Diagnostic: does one power + start command turn motor A, or only repeated ones?
 * Phase 1 sends the pair once and waits 2 s; phase 2 repeats it every 100 ms for 2 s.
 * Every call's return value and errno are printed.
 */
#include <errno.h>
#include <stdio.h>
#include <string.h>

#include "ev3.h"

int main(void)
{
    int rc;

    errno = 0;
    rc = ev3_motor_set_power(EV3_PORT_A, 50);
    printf("phase 1: one power+start, rc=%d errno=%d (%s)\n", rc, errno, strerror(errno));
    ev3_core_sleep_ms(2000);

    printf("phase 2: power+start every 100 ms for 2 s\n");
    for (int i = 0; i < 20; i++) {
        errno = 0;
        rc = ev3_motor_set_power(EV3_PORT_A, 50);
        if (rc != 0) {
            printf("  call %d: rc=%d errno=%d (%s)\n", i, rc, errno, strerror(errno));
        }
        ev3_core_sleep_ms(100);
    }

    rc = ev3_motor_stop(EV3_PORT_A, 1);
    printf("stop: rc=%d\n", rc);
    return 0;
}
