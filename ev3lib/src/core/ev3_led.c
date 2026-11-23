/*
 * ev3lib LEDs: the pattern of the two LEDs below the screen, written to the
 * UI device as the LEGO firmware sources (1.09D developer edition) define it:
 *   - d_ui/Linuxmod_AM1808/d_ui.c:435-461: a write of at least two bytes
 *     sets the pattern named by the first byte, as a digit '0'..'9';
 *   - lms2012/source/bytecodes.h:1354-1363: the patterns, LED_BLACK (0)
 *     through LED_ORANGE_PULSE (9), which EV3_LED_* follow.
 */
#include <errno.h>

#include "ev3.h"
#include "ev3_core_internal.h"

int ev3_led_set(int pattern)
{
    char command[2];

    if (pattern < EV3_LED_OFF || pattern > EV3_LED_ORANGE_PULSE) {
        errno = EINVAL;
        return -1;
    }
    command[0] = (char)('0' + pattern);
    command[1] = '\0';
    return ev3_core_device_write(ev3_core_ui_device(), command, sizeof command);
}
