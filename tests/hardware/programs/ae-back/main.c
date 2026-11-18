/*
 * Acceptance: a program that takes Back over receives short presses and keeps running;
 * holding Back still stops it. Each short press toggles motor A.
 */
#include <stdio.h>

#include "ev3.h"

int main(void)
{
    int was_pressed = 0;
    int motor_on = 0;

    ev3_button_take_back();
    printf("Back taken over: a short press toggles motor A, holding Back stops the program\n");
    for (;;) {
        int pressed = ev3_button_back_pressed();
        if (pressed && !was_pressed) {
            motor_on = !motor_on;
            if (motor_on) {
                ev3_motor_set_power(EV3_PORT_A, 40);
            } else {
                ev3_motor_stop(EV3_PORT_A, 1);
            }
            printf("Back pressed, motor %s\n", motor_on ? "on" : "off");
        }
        was_pressed = pressed;
        ev3_core_sleep_ms(20);
    }
}
