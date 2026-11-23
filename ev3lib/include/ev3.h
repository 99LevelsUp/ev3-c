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
 *   EV3C_PWM_DEVICE   Motor device, default /dev/lms_pwm.
 *   EV3C_UI_DEVICE    Button device, default /dev/lms_ui.
 * The two device variables exist for tests on a PC. All four are read once,
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

/* --- returning files ----------------------------------------------------- */

/*
 * A program can hand files back to its author: EVƎ C downloads the files a
 * run marks into the author's workspace after the run ends. Marking writes
 * the file's full path to the run folder's list at once, so the list
 * survives a crash; what the file holds is read when the run has ended.
 * A relative path is taken from the current folder. Paths may not contain
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
