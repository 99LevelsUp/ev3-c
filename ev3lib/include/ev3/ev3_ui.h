/*
 * ev3lib buttons, LEDs and sound.
 *
 * Each function comes as a try variant, which returns an Ev3Result, and as a
 * plain variant, which stops the program on an error (see ev3_error.h).
 */
#ifndef EV3_UI_H
#define EV3_UI_H

#include <stdbool.h>

#include "ev3_error.h"

#ifdef __cplusplus
extern "C" {
#endif

/* --- buttons ------------------------------------------------------------------ */

/* The brick's buttons, combinable with |. */
typedef enum {
    EV3_BUTTON_NONE  = 0x00,
    EV3_BUTTON_UP    = 0x01,
    EV3_BUTTON_ENTER = 0x02,
    EV3_BUTTON_DOWN  = 0x04,
    EV3_BUTTON_RIGHT = 0x08,
    EV3_BUTTON_LEFT  = 0x10,
    EV3_BUTTON_BACK  = 0x20,
    EV3_BUTTON_ANY   = 0x3F
} Ev3Button;

/* Whether the button is held down; for several, whether any of them is. */
Ev3Result ev3_button_try_is_pressed(Ev3Button buttons, bool *pressed);
bool ev3_button_is_pressed_at_(const char *file, int line, Ev3Button buttons);
#define ev3_button_is_pressed(buttons) ev3_button_is_pressed_at_(__FILE__, __LINE__, (buttons))

/*
 * Waits until one of the buttons is pressed, for at most timeout_ms (0: no
 * limit), and gives the one pressed. The try variant returns EV3_ERR_TIMEOUT
 * when time ran out; the plain one returns EV3_BUTTON_NONE then.
 */
Ev3Result ev3_button_try_wait_press(Ev3Button buttons, int timeout_ms, Ev3Button *pressed);
Ev3Button ev3_button_wait_press_at_(const char *file, int line, Ev3Button buttons, int timeout_ms);
#define ev3_button_wait_press(buttons, timeout_ms) \
    ev3_button_wait_press_at_(__FILE__, __LINE__, (buttons), (timeout_ms))

/* Waits until none of the buttons is held down. */
Ev3Result ev3_button_try_wait_release(Ev3Button buttons);
void ev3_button_wait_release_at_(const char *file, int line, Ev3Button buttons);
#define ev3_button_wait_release(buttons) ev3_button_wait_release_at_(__FILE__, __LINE__, (buttons))

/*
 * By default a press of the brick's Back button stops the program. After
 * ev3_button_take_back() a short press no longer stops it, and the program
 * can watch the button with ev3_button_is_pressed(EV3_BUTTON_BACK) instead.
 * A long press of Back always stops the program. EV3C_PROJECT_DIR must be set
 * (EVƎ C's supervisor sets it).
 */
Ev3Result ev3_button_try_take_back(void);
void ev3_button_take_back_at_(const char *file, int line);
#define ev3_button_take_back() ev3_button_take_back_at_(__FILE__, __LINE__)

Ev3Result ev3_button_try_release_back(void);
void ev3_button_release_back_at_(const char *file, int line);
#define ev3_button_release_back() ev3_button_release_back_at_(__FILE__, __LINE__)

/* --- LEDs --------------------------------------------------------------------- */

/* The patterns of the two LEDs below the screen. */
typedef enum {
    EV3_LED_OFF          = 0,
    EV3_LED_GREEN        = 1,
    EV3_LED_RED          = 2,
    EV3_LED_ORANGE       = 3,
    EV3_LED_GREEN_FLASH  = 4,
    EV3_LED_RED_FLASH    = 5,
    EV3_LED_ORANGE_FLASH = 6,
    EV3_LED_GREEN_PULSE  = 7,
    EV3_LED_RED_PULSE    = 8,
    EV3_LED_ORANGE_PULSE = 9
} Ev3LedPattern;

/*
 * Shows a pattern on the LEDs. The firmware sets them too, for example when a
 * program starts, so the last one to set them wins.
 */
Ev3Result ev3_led_try_set(Ev3LedPattern pattern);
void ev3_led_set_at_(const char *file, int line, Ev3LedPattern pattern);
#define ev3_led_set(pattern) ev3_led_set_at_(__FILE__, __LINE__, (pattern))

/* --- sound -------------------------------------------------------------------- */

/* The volume of ev3_sound_play_tone(). */
#define EV3_SOUND_VOLUME 50

/*
 * Plays a tone and returns at once: frequency 20 to 20000 Hz (the speaker
 * plays what it can of it), for ms milliseconds (0: until ev3_sound_stop()).
 */
Ev3Result ev3_sound_try_play_tone(int frequency, int ms);
void ev3_sound_play_tone_at_(const char *file, int line, int frequency, int ms);
#define ev3_sound_play_tone(frequency, ms) \
    ev3_sound_play_tone_at_(__FILE__, __LINE__, (frequency), (ms))

/* As ev3_sound_play_tone(), at a volume of 0 to 100. */
Ev3Result ev3_sound_try_play_tone_volume(int frequency, int ms, int volume);
void ev3_sound_play_tone_volume_at_(const char *file, int line, int frequency, int ms, int volume);
#define ev3_sound_play_tone_volume(frequency, ms, volume) \
    ev3_sound_play_tone_volume_at_(__FILE__, __LINE__, (frequency), (ms), (volume))

/* Stops the sound playing. */
Ev3Result ev3_sound_try_stop(void);
void ev3_sound_stop_at_(const char *file, int line);
#define ev3_sound_stop() ev3_sound_stop_at_(__FILE__, __LINE__)

#ifdef __cplusplus
}
#endif

#endif /* EV3_UI_H */
