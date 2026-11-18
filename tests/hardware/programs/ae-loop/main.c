/* Acceptance: an endless loop that keeps setting motor power stops for good on Stop. */
#include "ev3.h"

int main(void)
{
    for (;;) {
        ev3_motor_set_power(EV3_PORT_A, 40);
        ev3_core_sleep_ms(50);
    }
}
