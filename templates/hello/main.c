#include <stdio.h>

#include "ev3.h"

int main(void)
{
    printf("Hello from the EV3!\n");

    /* Run the motor on port A at half power for one second, then stop it. */
    ev3_motor_set_power(EV3_PORT_A, 50);
    ev3_core_sleep_ms(1000);
    ev3_motor_stop(EV3_PORT_A, 1);

    printf("Done.\n");
    return 0;
}
