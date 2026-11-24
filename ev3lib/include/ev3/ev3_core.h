/*
 * ev3lib core: start-up, exit, the run's folders, and waiting.
 *
 * Linking a program with libev3.a sets up the following before main() runs:
 *   - stdout writes line by line and stderr writes every call, so output
 *     printed before a crash or a stop still reaches the run folder;
 *   - a normal end of the program (return from main(), exit(), or a plain
 *     variant stopping on an error) stops all motors.
 *
 * Environment variables
 *   EV3C_PROJECT_DIR  The program's project folder on the brick. The
 *                     supervisor sets it; ev3_button_take_back() needs it.
 *   EV3C_RUN_DIR      The folder of the current run, where the supervisor
 *                     stores the program's output and result.
 *   EV3C_PWM_DEVICE, EV3C_MOTOR_DEVICE, EV3C_UI_DEVICE, EV3C_SOUND_DEVICE,
 *   EV3C_ANALOG_DEVICE, EV3C_UART_DEVICE, EV3C_IIC_DEVICE
 *                     The firmware's devices, for tests on a PC; the brick's
 *                     /dev/lms_* by default.
 * All of them are read once, when the program starts.
 *
 * Functions that cannot fail exist only once, without a try variant.
 */
#ifndef EV3_CORE_H
#define EV3_CORE_H

#include "ev3_error.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Sets up output buffering, reads the environment variables above, and
 * arranges the motor stop at exit. ev3lib calls it before main(), so a
 * program does not need to; later calls do nothing.
 */
void ev3_core_init(void);

/* Stops all motors. ev3lib calls it when the program ends; a program may call it earlier. */
void ev3_core_exit(void);

/* Project folder and run folder from the environment, or NULL if unset. */
const char *ev3_core_project_dir(void);
const char *ev3_core_run_dir(void);

/* Waits for a number of milliseconds. */
Ev3Result ev3_try_sleep_ms(int ms);
void ev3_sleep_ms_at_(const char *file, int line, int ms);
#define ev3_sleep_ms(ms) ev3_sleep_ms_at_(__FILE__, __LINE__, (ms))

/* Waits for a number of seconds: 0.25 is a quarter of a second. */
Ev3Result ev3_try_sleep_sec(float seconds);
void ev3_sleep_sec_at_(const char *file, int line, float seconds);
#define ev3_sleep_sec(seconds) ev3_sleep_sec_at_(__FILE__, __LINE__, (seconds))

#ifdef __cplusplus
}
#endif

#endif /* EV3_CORE_H */
