#include <stdio.h>

#include "ev3.h"

int main(void)
{
    printf("Hello from the EV3!\n");

    /* Run the motor on port A at half speed for one second, then stop it. */
    ev3_motor_start(EV3_PORT_A, 50);
    ev3_sleep_ms(1000);
    ev3_motor_stop(EV3_PORT_A, EV3_STOP_BRAKE);

    printf("Done.\n");
    return 0;
}
