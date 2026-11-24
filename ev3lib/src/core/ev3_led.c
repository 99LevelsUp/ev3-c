/*
 * ev3lib LEDs: the pattern of the two LEDs below the screen, written to the
 * UI device as the LEGO firmware sources (1.09D developer edition) define it:
 *   - d_ui/Linuxmod_AM1808/d_ui.c:435-461: a write of at least two bytes
 *     sets the pattern named by the first byte, as a digit '0'..'9';
 *   - lms2012/source/bytecodes.h:1354-1363: the patterns, LED_BLACK (0)
 *     through LED_ORANGE_PULSE (9), which EV3_LED_* follow.
 */
#include "ev3.h"
#include "ev3_core_internal.h"

Ev3Result ev3_led_try_set(Ev3LedPattern pattern)
{
    char command[2];

    if ((int)pattern < EV3_LED_OFF || (int)pattern > EV3_LED_ORANGE_PULSE) {
        return ev3_fail(EV3_ERR_PARAM, "LED pattern %d is not one of EV3_LED_OFF to EV3_LED_ORANGE_PULSE", (int)pattern);
    }
    command[0] = (char)('0' + (int)pattern);
    command[1] = '\0';
    if (ev3_core_device_write(ev3_core_ui_device(), command, sizeof command) != 0) {
        return ev3_fail_errno(EV3_ERR_IO, "the LED device cannot be written");
    }
    return EV3_OK;
}

void ev3_led_set_at_(EV3_AT, Ev3LedPattern pattern)
{
    EV3_MUST("ev3_led_set", ev3_led_try_set(pattern));
}
