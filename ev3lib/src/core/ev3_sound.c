/*
 * ev3lib sound: tones on the brick's speaker, written to the sound device as
 * the LEGO firmware sources (1.09D developer edition) define it:
 *   - lms2012/source/bytecodes.h:936-940: the commands, BREAK (0) and
 *     TONE (1) among them;
 *   - d_sound/Linuxmod_AM1808/d_sound.c:633-692: TONE reads the volume in
 *     percent, then the frequency in Hz and the duration in milliseconds as
 *     16-bit little-endian values; it limits the volume to 100 and the
 *     frequency to what the speaker plays, and a duration of 0 sounds until
 *     stopped;
 *   - d_sound.c:694: BREAK stops the sound.
 */
#include "ev3.h"
#include "ev3_core_internal.h"

#define EV3_SOUND_BREAK 0
#define EV3_SOUND_TONE  1

int ev3_sound_tone(int volume, unsigned int frequency, unsigned int ms)
{
    unsigned char command[6];

    if (volume < 0) {
        volume = 0;
    } else if (volume > 100) {
        volume = 100;
    }
    if (frequency > 0xFFFF) {
        frequency = 0xFFFF;
    }
    if (ms > 0xFFFF) {
        ms = 0xFFFF;
    }
    command[0] = EV3_SOUND_TONE;
    command[1] = (unsigned char)volume;
    command[2] = (unsigned char)(frequency & 0xFF);
    command[3] = (unsigned char)(frequency >> 8);
    command[4] = (unsigned char)(ms & 0xFF);
    command[5] = (unsigned char)(ms >> 8);
    return ev3_core_device_write(ev3_core_sound_device(), command, sizeof command);
}

int ev3_sound_stop(void)
{
    static const unsigned char command[] = { EV3_SOUND_BREAK };

    return ev3_core_device_write(ev3_core_sound_device(), command, sizeof command);
}
