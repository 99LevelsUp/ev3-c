/*
 * Acceptance: ev3lib's regulated motor commands, motor readings, buttons,
 * LEDs and tones on a brick. A motor on port A.
 */
#include <stdio.h>

#include "ev3.h"

static void show(const char *what)
{
    printf("%s: speed %d, %d degrees\n", what, ev3_motor_get_speed(EV3_PORT_A),
           ev3_motor_get_position(EV3_PORT_A));
}

int main(void)
{
    ev3_led_set(EV3_LED_ORANGE);
    ev3_sound_play_tone(440, 200);

    ev3_motor_reset_position(EV3_PORT_A);
    show("after reset");

    /* One turn at half speed, braking at the end; the call returns when it is done. */
    ev3_motor_run_degrees(EV3_PORT_A, 50, 360);
    show("after 360 degrees");

    /* Two seconds backwards at a regulated speed, read halfway. */
    ev3_motor_start_seconds(EV3_PORT_A, -30, 2.0f);
    printf("busy right after the start: %d\n", ev3_motor_is_busy(EV3_PORT_A));
    ev3_sleep_ms(1000);
    show("halfway back");
    ev3_motor_wait(EV3_PORT_A, 0);

    /* Reversed polarity: a positive speed turns the other way. */
    ev3_motor_set_polarity(EV3_PORT_A, -1);
    ev3_motor_start(EV3_PORT_A, 20);
    ev3_sleep_ms(1000);
    ev3_motor_stop(EV3_PORT_A, EV3_STOP_BRAKE);
    ev3_motor_set_polarity(EV3_PORT_A, 1);
    show("after a second reversed");

    ev3_led_set(EV3_LED_GREEN_FLASH);
    printf("press Enter on the brick\n");
    ev3_button_wait_press(EV3_BUTTON_ENTER, 0);
    ev3_sound_play_tone(880, 200);
    ev3_led_set(EV3_LED_GREEN);
    printf("Enter pressed\n");

    /* A sensor port passed to a motor: the plain variant stops the program with where and why. */
    printf("now a mistake on purpose\n");
    ev3_motor_start(EV3_PORT_1, 50);
    return 0;
}
