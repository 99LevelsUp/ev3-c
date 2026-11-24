/*
 * Diagnostic: the IR remote's buttons on all four channels, from the infrared
 * sensor on port 3. After a beep, press Enter; press the remote's buttons one
 * at a time, slowly; press Enter again. It prints a line whenever the
 * readings change.
 */
#include <stdio.h>

#include "ev3.h"

int main(void)
{
    int last[4] = { -1, -1, -1, -1 };

    ev3_sound_play_tone(880, 150);
    ev3_button_wait_press(EV3_BUTTON_ENTER, 0);
    ev3_button_wait_release(EV3_BUTTON_ENTER);
    ev3_led_set(EV3_LED_GREEN_FLASH);
    while (!ev3_button_is_pressed(EV3_BUTTON_ENTER)) {
        int now[4];
        bool changed = false;
        for (int channel = 1; channel <= 4; channel++) {
            now[channel - 1] = ev3_sensor_read_ir_remote(EV3_PORT_3, channel);
            changed = changed || now[channel - 1] != last[channel - 1];
        }
        if (changed) {
            printf("channels 1-4: %d %d %d %d\n", now[0], now[1], now[2], now[3]);
            for (int i = 0; i < 4; i++) {
                last[i] = now[i];
            }
        }
        ev3_sleep_ms(50);
    }
    ev3_led_set(EV3_LED_OFF);
    ev3_sound_play_tone(440, 150);
    return 0;
}
