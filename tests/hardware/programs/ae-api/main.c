/*
 * Acceptance: ev3lib's regulated motor commands, motor readings, buttons,
 * LEDs and tones on a brick. A motor on port A.
 */
#include <stdio.h>

#include "ev3.h"

static void show(const char *what)
{
    int speed = 0, degrees = 0;

    if (ev3_motor_read(EV3_PORT_A, &speed, &degrees) != 0) {
        perror("ev3_motor_read");
        return;
    }
    printf("%s: speed %d, %d degrees\n", what, speed, degrees);
}

int main(void)
{
    ev3_led_set(EV3_LED_ORANGE);
    ev3_sound_tone(30, 440, 200);

    ev3_motor_reset_count(EV3_PORT_A);
    show("after reset");

    /* One turn at half speed, braking at the end; the program waits for it. */
    ev3_motor_run_degrees(EV3_PORT_A, 50, 360, 1);
    printf("busy right after the start: %d\n", ev3_motor_busy(EV3_PORT_A));
    ev3_motor_wait(EV3_PORT_A);
    show("after 360 degrees");

    /* Two seconds backwards at a regulated speed, read halfway. */
    ev3_motor_run_ms(EV3_PORT_A, -30, 2000, 0);
    ev3_core_sleep_ms(1000);
    show("halfway back");
    ev3_motor_wait(EV3_PORT_A);

    /* Reversed polarity: a positive speed turns the other way. */
    ev3_motor_set_polarity(EV3_PORT_A, -1);
    ev3_motor_set_speed(EV3_PORT_A, 20);
    ev3_core_sleep_ms(1000);
    ev3_motor_stop(EV3_PORT_A, 1);
    ev3_motor_set_polarity(EV3_PORT_A, 1);
    show("after a second reversed");

    ev3_led_set(EV3_LED_GREEN_FLASH);
    printf("press Enter on the brick\n");
    while (ev3_button_pressed(EV3_BUTTON_ENTER) == 0) {
        ev3_core_sleep_ms(20);
    }
    ev3_sound_tone(30, 880, 200);
    ev3_led_set(EV3_LED_GREEN);
    printf("Enter pressed\n");
    return 0;
}
