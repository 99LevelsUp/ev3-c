/* Acceptance: a crash stops the motor and the run result shows the printed line and the signal. */
#include <stdio.h>

#include "ev3.h"

int main(void)
{
    volatile int *nowhere = NULL;

    ev3_motor_set_power(EV3_PORT_A, 40);
    printf("motor A started, crashing in 2 s\n");
    ev3_sleep_ms(2000);
    return *nowhere;
}
