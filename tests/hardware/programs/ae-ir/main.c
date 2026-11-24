/*
 * Acceptance: the infrared sensor on port 3, with the IR remote/beacon on
 * channel 1. Three windows, each started and ended with Enter after a beep:
 *   1 beep   proximity: move a hand towards and away from the sensor
 *   2 beeps  remote: press the remote's buttons, one and two at a time
 *   3 beeps  beacon: switch the beacon on (the top button) and move it around
 * It prints a reading every 200 ms.
 */
#include <stdio.h>

#include "ev3.h"

static void begin(int beeps)
{
    for (int i = 0; i < beeps; i++) {
        ev3_sound_play_tone(880, 120);
        ev3_sleep_ms(250);
    }
    ev3_button_wait_press(EV3_BUTTON_ENTER, 0);
    ev3_button_wait_release(EV3_BUTTON_ENTER);
    ev3_led_set(EV3_LED_GREEN_FLASH);
}

static bool done(void)
{
    if (!ev3_button_is_pressed(EV3_BUTTON_ENTER)) {
        return false;
    }
    ev3_button_wait_release(EV3_BUTTON_ENTER);
    ev3_led_set(EV3_LED_OFF);
    ev3_sound_play_tone(440, 150);
    return true;
}

static void report(Ev3Result result)
{
    printf("  %s (%s)\n", ev3_error_string(result), ev3_error_message());
}

int main(void)
{
    Ev3Result result;

    printf("proximity\n");
    begin(1);
    while (!done()) {
        int proximity = 0;
        result = ev3_sensor_try_read_ir_proximity(EV3_PORT_3, &proximity);
        result == EV3_OK ? (void)printf("  proximity=%d\n", proximity) : report(result);
        ev3_sleep_ms(200);
    }

    printf("remote, channel 1\n");
    begin(2);
    while (!done()) {
        int buttons = 0;
        result = ev3_sensor_try_read_ir_remote(EV3_PORT_3, 1, &buttons);
        result == EV3_OK ? (void)printf("  buttons=%d\n", buttons) : report(result);
        ev3_sleep_ms(200);
    }

    printf("beacon, channel 1\n");
    begin(3);
    while (!done()) {
        Ev3IrSeek seek;
        result = ev3_sensor_try_read_ir_seek(EV3_PORT_3, 1, &seek);
        result == EV3_OK ? (void)printf("  found=%d heading=%d distance=%d\n", seek.found, seek.heading, seek.distance)
                         : report(result);
        ev3_sleep_ms(200);
    }
    return 0;
}
