/*
 * Acceptance: a program that takes Back over receives short presses and keeps running;
 * holding Back still stops it. Each short press toggles motor A.
 */
#include <stdio.h>

#include "ev3.h"

int main(void)
{
    bool motor_on = false;

    ev3_button_take_back();
    printf("Back taken over: a short press toggles motor A, holding Back stops the program\n");
    for (;;) {
        ev3_button_wait_press(EV3_BUTTON_BACK, 0);
        motor_on = !motor_on;
        if (motor_on) {
            ev3_motor_set_power(EV3_PORT_A, 40);
        } else {
            ev3_motor_stop(EV3_PORT_A, EV3_STOP_BRAKE);
        }
        printf("Back pressed, motor %s\n", motor_on ? "on" : "off");
        ev3_button_wait_release(EV3_BUTTON_BACK);
    }
}
