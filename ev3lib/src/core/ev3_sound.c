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

static Ev3Result write_sound(const unsigned char *command, size_t length)
{
    if (ev3_core_device_write(ev3_core_sound_device(), command, length) != 0) {
        return ev3_fail_errno(EV3_ERR_IO, "the sound device cannot be written");
    }
    return EV3_OK;
}

Ev3Result ev3_sound_try_play_tone_volume(int frequency, int ms, int volume)
{
    unsigned char command[6];

    if (frequency < 20 || frequency > 20000) {
        return ev3_fail(EV3_ERR_PARAM, "frequency %d Hz is outside 20 to 20000", frequency);
    }
    if (ms < 0 || ms > 0xFFFF) {
        return ev3_fail(EV3_ERR_PARAM, "%d ms is outside 0 to 65535", ms);
    }
    if (volume < 0 || volume > 100) {
        return ev3_fail(EV3_ERR_PARAM, "volume %d is outside 0 to 100", volume);
    }
    command[0] = EV3_SOUND_TONE;
    command[1] = (unsigned char)volume;
    command[2] = (unsigned char)(frequency & 0xFF);
    command[3] = (unsigned char)(frequency >> 8);
    command[4] = (unsigned char)(ms & 0xFF);
    command[5] = (unsigned char)(ms >> 8);
    return write_sound(command, sizeof command);
}

Ev3Result ev3_sound_try_play_tone(int frequency, int ms)
{
    return ev3_sound_try_play_tone_volume(frequency, ms, EV3_SOUND_VOLUME);
}

Ev3Result ev3_sound_try_stop(void)
{
    static const unsigned char command[] = { EV3_SOUND_BREAK };

    return write_sound(command, sizeof command);
}

void ev3_sound_play_tone_at_(EV3_AT, int frequency, int ms)
{
    EV3_MUST("ev3_sound_play_tone", ev3_sound_try_play_tone(frequency, ms));
}

void ev3_sound_play_tone_volume_at_(EV3_AT, int frequency, int ms, int volume)
{
    EV3_MUST("ev3_sound_play_tone_volume", ev3_sound_try_play_tone_volume(frequency, ms, volume));
}

void ev3_sound_stop_at_(EV3_AT)
{
    EV3_MUST("ev3_sound_stop", ev3_sound_try_stop());
}
