/*
 * Acceptance: the gyro on port 4. After a beep, press Enter; turn the gyro a
 * quarter turn clockwise, wait, a quarter turn back, then press Enter again.
 * It prints the angle and rate every 200 ms, then resets the gyro.
 */
#include <stdio.h>

#include "ev3.h"

int main(void)
{
    int tick = 0;

    ev3_sound_play_tone(880, 150);
    ev3_button_wait_press(EV3_BUTTON_ENTER, 0);
    ev3_button_wait_release(EV3_BUTTON_ENTER);
    ev3_led_set(EV3_LED_GREEN_FLASH);
    while (!ev3_button_is_pressed(EV3_BUTTON_ENTER)) {
        int angle = 0, rate = 0;
        Ev3Result result = ev3_sensor_try_read_gyro_angle(EV3_PORT_4, &angle);
        if (result == EV3_OK) {
            result = ev3_sensor_try_read_gyro_rate(EV3_PORT_4, &rate);
        }
        if (result != EV3_OK) {
            printf("t=%.1f %s (%s)\n", tick / 5.0, ev3_error_string(result), ev3_error_message());
        } else {
            printf("t=%.1f angle=%d rate=%d\n", tick / 5.0, angle, rate);
        }
        tick++;
        ev3_sleep_ms(200);
    }
    ev3_led_set(EV3_LED_OFF);
    ev3_sound_play_tone(440, 150);
    ev3_sensor_gyro_reset(EV3_PORT_4);
    ev3_sleep_ms(500);
    printf("after the reset: angle=%d\n", ev3_sensor_read_gyro_angle(EV3_PORT_4));
    return 0;
}
