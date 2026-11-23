/*
 * ev3lib: C programs for the LEGO EV3 brick running the original firmware.
 *
 * Linking a program with libev3.a sets up the following before main() runs:
 *   - stdout writes line by line and stderr writes every call, so output
 *     printed before a crash or a stop still reaches the run folder;
 *   - a normal end of the program (return from main() or exit()) stops all
 *     motors.
 *
 * Environment variables
 *   EV3C_PROJECT_DIR  The program's project folder on the brick. The
 *                     supervisor sets it; ev3_button_take_back() needs it.
 *   EV3C_RUN_DIR      The folder of the current run, where the supervisor
 *                     stores the program's output and result.
 *   EV3C_PWM_DEVICE   Motor command device, default /dev/lms_pwm.
 *   EV3C_MOTOR_DEVICE Motor measurement device, default /dev/lms_motor.
 *   EV3C_UI_DEVICE    Button and LED device, default /dev/lms_ui.
 *   EV3C_SOUND_DEVICE Sound device, default /dev/lms_sound.
 * The device variables exist for tests on a PC. All of them are read once,
 * when the program starts.
 *
 * Functions that can fail return -1 and set errno.
 */
#ifndef EV3_H
#define EV3_H

#include <stdio.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Motor ports, combinable with |. */
#define EV3_PORT_A   0x01
#define EV3_PORT_B   0x02
#define EV3_PORT_C   0x04
#define EV3_PORT_D   0x08
#define EV3_PORT_ALL 0x0F

/* --- core ---------------------------------------------------------------- */

/*
 * Sets up output buffering, reads the environment variables above, and
 * arranges the motor stop at exit. ev3lib calls it before main(), so a
 * program does not need to; later calls do nothing.
 */
void ev3_core_init(void);

/*
 * Stops all motors. ev3lib calls it when the program ends normally; a
 * program may call it earlier.
 */
void ev3_core_exit(void);

/* Project folder and run folder from the environment, or NULL if unset. */
const char *ev3_core_project_dir(void);
const char *ev3_core_run_dir(void);

/* Waits for the given number of milliseconds. */
void ev3_core_sleep_ms(unsigned int ms);

/* --- motors -------------------------------------------------------------- */

/*
 * Runs the motors on the given ports at a power from -100 (full backwards)
 * to 100 (full forwards); values outside that range are limited to it.
 * Returns 0, or -1 if the motor device cannot be written.
 */
int ev3_motor_set_power(int ports, int power);

/*
 * Stops the motors on the given ports. With brake nonzero the motors hold
 * their position; otherwise they coast.
 * Returns 0, or -1 if the motor device cannot be written.
 */
int ev3_motor_stop(int ports, int brake);

/*
 * Runs the motors at a regulated speed from -100 to 100 percent of full
 * speed: unlike power, the firmware holds the speed under load. Values
 * outside the range are limited to it. Returns 0 or -1.
 */
int ev3_motor_set_speed(int ports, int speed);

/*
 * Sets which way the motors turn for a positive power or speed: 1 as built,
 * -1 reversed, 0 the other way than now. Returns 0 or -1 (EINVAL for any
 * other value).
 */
int ev3_motor_set_polarity(int ports, int polarity);

/*
 * Turns the motors by the given number of degrees at a regulated speed, and
 * returns at once; the sign of the speed gives the direction. When done the
 * motors brake (brake nonzero) or coast. ev3_motor_wait() waits for the end.
 * Returns 0 or -1.
 */
int ev3_motor_run_degrees(int ports, int speed, unsigned int degrees, int brake);

/* As ev3_motor_run_degrees(), for a time in milliseconds instead of degrees. */
int ev3_motor_run_ms(int ports, int speed, unsigned int ms, int brake);

/*
 * Returns 1 while any of the motors still runs a ev3_motor_run_degrees() or
 * ev3_motor_run_ms() command, 0 when none does, -1 on error.
 */
int ev3_motor_busy(int ports);

/* Waits until ev3_motor_busy() is 0 for the motors. Returns 0 or -1. */
int ev3_motor_wait(int ports);

/*
 * Reads one motor: its speed (-100..100) and the degrees it turned since its
 * count was last reset, as the firmware's rotation sensor counts them. Pass
 * one port. Either pointer may be NULL. Returns 0, or -1 (EINVAL for no port
 * or several).
 */
int ev3_motor_read(int port, int *speed, int *degrees);

/* Sets the degree count of the motors to 0. Returns 0 or -1. */
int ev3_motor_reset_count(int ports);

/* --- buttons ------------------------------------------------------------- */

/*
 * By default a press of the brick's Back button stops the program. After
 * ev3_button_take_back() a short press no longer stops it, and the program
 * can watch the button with ev3_button_back_pressed() instead. A long press
 * of Back always stops the program.
 *
 * Both calls return 0, or -1 if EV3C_PROJECT_DIR is unset (errno ENOENT) or
 * the request cannot be written or removed.
 */
int ev3_button_take_back(void);
int ev3_button_release_back(void);

/* Returns 1 while Back is held down, 0 when it is not, -1 on error. */
int ev3_button_back_pressed(void);

/* The brick's buttons, for ev3_button_pressed(). */
#define EV3_BUTTON_UP    0
#define EV3_BUTTON_ENTER 1
#define EV3_BUTTON_DOWN  2
#define EV3_BUTTON_RIGHT 3
#define EV3_BUTTON_LEFT  4
#define EV3_BUTTON_BACK  5

/* Returns 1 while the button is held down, 0 when it is not, -1 on error (EINVAL for no such button). */
int ev3_button_pressed(int button);

/* --- LEDs and sound ------------------------------------------------------ */

/* The patterns of the two LEDs below the screen, for ev3_led_set(). */
#define EV3_LED_OFF          0
#define EV3_LED_GREEN        1
#define EV3_LED_RED          2
#define EV3_LED_ORANGE       3
#define EV3_LED_GREEN_FLASH  4
#define EV3_LED_RED_FLASH    5
#define EV3_LED_ORANGE_FLASH 6
#define EV3_LED_GREEN_PULSE  7
#define EV3_LED_RED_PULSE    8
#define EV3_LED_ORANGE_PULSE 9

/*
 * Shows a pattern on the LEDs. The firmware sets them too, for example when
 * a program starts, so the last one to set them wins. Returns 0, or -1
 * (EINVAL for no such pattern).
 */
int ev3_led_set(int pattern);

/*
 * Plays a tone and returns at once: volume 0..100 (higher is limited to
 * 100), frequency in Hz (the firmware keeps it within what the speaker can
 * play), for the given milliseconds, or until ev3_sound_stop() for 0.
 * Returns 0 or -1.
 */
int ev3_sound_tone(int volume, unsigned int frequency, unsigned int ms);

/* Stops the sound playing. Returns 0 or -1. */
int ev3_sound_stop(void);

/* --- returning files ----------------------------------------------------- */

/*
 * A program can hand files back to its author: EVƎ C downloads the files a
 * run marks into the author's workspace after the run ends. Marking writes
 * the file's full path to the run folder's list at once, so the list
 * survives a crash; what the file holds is read when the run has ended.
 * A relative path is taken from the current folder, which is the project
 * folder when EVƎ C's supervisor starts the program. Paths may not contain
 * a line break.
 */

/*
 * Opens a file like fopen() and marks it for return. Returns the open file,
 * or NULL with errno set if it cannot be opened (EINVAL for a line break).
 * Outside a run (no EV3C_RUN_DIR) the file opens and nothing is marked.
 */
FILE *ev3_return_fopen(const char *path, const char *mode);

/*
 * Marks a file the program already created for return. Returns 0, or -1
 * with errno set: ENOENT if the file does not exist or the program runs
 * outside a run, EINVAL for a line break, or why the list cannot be written.
 */
int ev3_return_file(const char *path);

#ifdef __cplusplus
}
#endif

#endif /* EV3_H */
