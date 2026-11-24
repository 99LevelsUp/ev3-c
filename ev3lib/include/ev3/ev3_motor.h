/*
 * ev3lib motors on ports A to D.
 *
 * Commands that drive motors take one port or several combined with |, for
 * example EV3_PORT_A | EV3_PORT_B. Readings take exactly one port.
 *
 * Speed and power go from -100 (full backwards) to 100 (full forwards). Speed is
 * regulated: the firmware holds it under load. Power is not.
 *
 * Each function comes as a try variant, which returns an Ev3Result, and as a
 * plain variant, which stops the program on an error (see ev3_error.h).
 */
#ifndef EV3_MOTOR_H
#define EV3_MOTOR_H

#include <stdbool.h>

#include "ev3_error.h"

#ifdef __cplusplus
extern "C" {
#endif

/* How a motor stops. */
typedef enum {
    EV3_STOP_COAST = 0, /* the motor runs out freely */
    EV3_STOP_BRAKE = 1, /* the motor brakes and holds its position */
} Ev3StopMode;

/* --- running ------------------------------------------------------------------ */

/* Starts the motors at a regulated speed; they run until stopped. */
Ev3Result ev3_motor_try_start(Ev3Port ports, int speed);
void ev3_motor_start_at_(const char *file, int line, Ev3Port ports, int speed);
#define ev3_motor_start(ports, speed) ev3_motor_start_at_(__FILE__, __LINE__, (ports), (speed))

/* Runs the motors at a power without regulation; they run until stopped. */
Ev3Result ev3_motor_try_set_power(Ev3Port ports, int power);
void ev3_motor_set_power_at_(const char *file, int line, Ev3Port ports, int power);
#define ev3_motor_set_power(ports, power) ev3_motor_set_power_at_(__FILE__, __LINE__, (ports), (power))

/* Stops the motors. */
Ev3Result ev3_motor_try_stop(Ev3Port ports, Ev3StopMode mode);
void ev3_motor_stop_at_(const char *file, int line, Ev3Port ports, Ev3StopMode mode);
#define ev3_motor_stop(ports, mode) ev3_motor_stop_at_(__FILE__, __LINE__, (ports), (mode))

/*
 * Turns the motors by a number of degrees at a regulated speed and returns when
 * they are done; then they brake. The sign of the speed or of the degrees gives
 * the direction: both negative turn forwards again.
 */
Ev3Result ev3_motor_try_run_degrees(Ev3Port ports, int speed, int degrees);
void ev3_motor_run_degrees_at_(const char *file, int line, Ev3Port ports, int speed, int degrees);
#define ev3_motor_run_degrees(ports, speed, degrees) \
    ev3_motor_run_degrees_at_(__FILE__, __LINE__, (ports), (speed), (degrees))

/* As ev3_motor_run_degrees(), in whole turns: 1.5 is 540 degrees. */
Ev3Result ev3_motor_try_run_rotations(Ev3Port ports, int speed, float rotations);
void ev3_motor_run_rotations_at_(const char *file, int line, Ev3Port ports, int speed, float rotations);
#define ev3_motor_run_rotations(ports, speed, rotations) \
    ev3_motor_run_rotations_at_(__FILE__, __LINE__, (ports), (speed), (rotations))

/* Runs the motors for a time at a regulated speed and returns when it is over; then they brake. */
Ev3Result ev3_motor_try_run_seconds(Ev3Port ports, int speed, float seconds);
void ev3_motor_run_seconds_at_(const char *file, int line, Ev3Port ports, int speed, float seconds);
#define ev3_motor_run_seconds(ports, speed, seconds) \
    ev3_motor_run_seconds_at_(__FILE__, __LINE__, (ports), (speed), (seconds))

/*
 * Sets which way the motors turn for a positive speed or power: 1 as built,
 * -1 reversed, 0 the other way than now.
 */
Ev3Result ev3_motor_try_set_polarity(Ev3Port ports, int polarity);
void ev3_motor_set_polarity_at_(const char *file, int line, Ev3Port ports, int polarity);
#define ev3_motor_set_polarity(ports, polarity) \
    ev3_motor_set_polarity_at_(__FILE__, __LINE__, (ports), (polarity))

/* --- reading ------------------------------------------------------------------ */

/* The degrees the motor has turned since its position was last reset. */
Ev3Result ev3_motor_try_get_position(Ev3Port port, int *degrees);
int ev3_motor_get_position_at_(const char *file, int line, Ev3Port port);
#define ev3_motor_get_position(port) ev3_motor_get_position_at_(__FILE__, __LINE__, (port))

/* The motor's speed now, -100 to 100. */
Ev3Result ev3_motor_try_get_speed(Ev3Port port, int *speed);
int ev3_motor_get_speed_at_(const char *file, int line, Ev3Port port);
#define ev3_motor_get_speed(port) ev3_motor_get_speed_at_(__FILE__, __LINE__, (port))

/* Sets the position of the motors to 0. */
Ev3Result ev3_motor_try_reset_position(Ev3Port ports);
void ev3_motor_reset_position_at_(const char *file, int line, Ev3Port ports);
#define ev3_motor_reset_position(ports) ev3_motor_reset_position_at_(__FILE__, __LINE__, (ports))

/* --- waiting ------------------------------------------------------------------ */

/*
 * Whether any of the motors still runs a command with a set amount (degrees,
 * rotations or seconds) started by the _start variants below.
 */
Ev3Result ev3_motor_try_is_busy(Ev3Port ports, bool *busy);
bool ev3_motor_is_busy_at_(const char *file, int line, Ev3Port ports);
#define ev3_motor_is_busy(ports) ev3_motor_is_busy_at_(__FILE__, __LINE__, (ports))

/*
 * Waits until the motors are done, for at most timeout_ms (0: no limit). The
 * try variant returns EV3_ERR_TIMEOUT when time ran out; the plain one returns
 * true when the motors are done and false when time ran out.
 */
Ev3Result ev3_motor_try_wait(Ev3Port ports, int timeout_ms);
bool ev3_motor_wait_at_(const char *file, int line, Ev3Port ports, int timeout_ms);
#define ev3_motor_wait(ports, timeout_ms) ev3_motor_wait_at_(__FILE__, __LINE__, (ports), (timeout_ms))

/*
 * The run commands above wait until the motors are done. These start the same
 * command and return at once, so a program can do something else meanwhile and
 * wait with ev3_motor_wait().
 */
Ev3Result ev3_motor_try_start_degrees(Ev3Port ports, int speed, int degrees);
void ev3_motor_start_degrees_at_(const char *file, int line, Ev3Port ports, int speed, int degrees);
#define ev3_motor_start_degrees(ports, speed, degrees) \
    ev3_motor_start_degrees_at_(__FILE__, __LINE__, (ports), (speed), (degrees))

Ev3Result ev3_motor_try_start_seconds(Ev3Port ports, int speed, float seconds);
void ev3_motor_start_seconds_at_(const char *file, int line, Ev3Port ports, int speed, float seconds);
#define ev3_motor_start_seconds(ports, speed, seconds) \
    ev3_motor_start_seconds_at_(__FILE__, __LINE__, (ports), (speed), (seconds))

/* --- ramps -------------------------------------------------------------------- */

/*
 * Turns the motors with a smooth start and end: they speed up over the first
 * up degrees, keep the speed for steady degrees and slow down over the last
 * down degrees, then stop as mode says. The three amounts cannot be negative;
 * a negative speed turns the other way. Returns when the motors are done.
 */
Ev3Result ev3_motor_try_run_degrees_ramp(Ev3Port ports, int speed, int up, int steady, int down, Ev3StopMode mode);
void ev3_motor_run_degrees_ramp_at_(const char *file, int line, Ev3Port ports, int speed, int up, int steady, int down,
                                    Ev3StopMode mode);
#define ev3_motor_run_degrees_ramp(ports, speed, up, steady, down, mode) \
    ev3_motor_run_degrees_ramp_at_(__FILE__, __LINE__, (ports), (speed), (up), (steady), (down), (mode))

/* As ev3_motor_run_degrees_ramp(), in seconds. */
Ev3Result ev3_motor_try_run_seconds_ramp(Ev3Port ports, int speed, float up, float steady, float down,
                                         Ev3StopMode mode);
void ev3_motor_run_seconds_ramp_at_(const char *file, int line, Ev3Port ports, int speed, float up, float steady,
                                    float down, Ev3StopMode mode);
#define ev3_motor_run_seconds_ramp(ports, speed, up, steady, down, mode) \
    ev3_motor_run_seconds_ramp_at_(__FILE__, __LINE__, (ports), (speed), (up), (steady), (down), (mode))

/* The two ramps above, returning at once; wait with ev3_motor_wait(). */
Ev3Result ev3_motor_try_start_degrees_ramp(Ev3Port ports, int speed, int up, int steady, int down, Ev3StopMode mode);
void ev3_motor_start_degrees_ramp_at_(const char *file, int line, Ev3Port ports, int speed, int up, int steady,
                                      int down, Ev3StopMode mode);
#define ev3_motor_start_degrees_ramp(ports, speed, up, steady, down, mode) \
    ev3_motor_start_degrees_ramp_at_(__FILE__, __LINE__, (ports), (speed), (up), (steady), (down), (mode))

Ev3Result ev3_motor_try_start_seconds_ramp(Ev3Port ports, int speed, float up, float steady, float down,
                                           Ev3StopMode mode);
void ev3_motor_start_seconds_ramp_at_(const char *file, int line, Ev3Port ports, int speed, float up, float steady,
                                      float down, Ev3StopMode mode);
#define ev3_motor_start_seconds_ramp(ports, speed, up, steady, down, mode) \
    ev3_motor_start_seconds_ramp_at_(__FILE__, __LINE__, (ports), (speed), (up), (steady), (down), (mode))

/* --- two motors together ------------------------------------------------------ */

/*
 * Drives two motors as a pair, such as the wheels of a robot: the firmware
 * keeps them in step, so the robot goes straight or turns evenly.
 *
 * left and right are one port each. turn goes from -200 to 200 and says how
 * the right motor follows the left one; a negative turn swaps their roles:
 *     0         both motors at the same speed: straight on
 *     1 to 99   the right motor slower: a curve to the right
 *     100       the right motor stands still: a turn around the right wheel
 *     101..199  the right motor backwards, slower than the left one
 *     200       the right motor backwards as fast: a turn on the spot
 *
 * Degrees, rotations and seconds count the faster motor. Only one pair runs at
 * a time: starting a pair with other motors stops the pair that ran.
 */
Ev3Result ev3_motor_try_sync_start(Ev3Port left, Ev3Port right, int speed, int turn);
void ev3_motor_sync_start_at_(const char *file, int line, Ev3Port left, Ev3Port right, int speed, int turn);
#define ev3_motor_sync_start(left, right, speed, turn) \
    ev3_motor_sync_start_at_(__FILE__, __LINE__, (left), (right), (speed), (turn))

/* Drives the pair by a number of degrees and returns when it is done; then the motors brake. */
Ev3Result ev3_motor_try_sync_run_degrees(Ev3Port left, Ev3Port right, int speed, int turn, int degrees);
void ev3_motor_sync_run_degrees_at_(const char *file, int line, Ev3Port left, Ev3Port right, int speed, int turn,
                                    int degrees);
#define ev3_motor_sync_run_degrees(left, right, speed, turn, degrees) \
    ev3_motor_sync_run_degrees_at_(__FILE__, __LINE__, (left), (right), (speed), (turn), (degrees))

/* As ev3_motor_sync_run_degrees(), in whole turns of the faster motor. */
Ev3Result ev3_motor_try_sync_run_rotations(Ev3Port left, Ev3Port right, int speed, int turn, float rotations);
void ev3_motor_sync_run_rotations_at_(const char *file, int line, Ev3Port left, Ev3Port right, int speed, int turn,
                                      float rotations);
#define ev3_motor_sync_run_rotations(left, right, speed, turn, rotations) \
    ev3_motor_sync_run_rotations_at_(__FILE__, __LINE__, (left), (right), (speed), (turn), (rotations))

/* Drives the pair for a time and returns when it is over; then the motors brake. */
Ev3Result ev3_motor_try_sync_run_seconds(Ev3Port left, Ev3Port right, int speed, int turn, float seconds);
void ev3_motor_sync_run_seconds_at_(const char *file, int line, Ev3Port left, Ev3Port right, int speed, int turn,
                                    float seconds);
#define ev3_motor_sync_run_seconds(left, right, speed, turn, seconds) \
    ev3_motor_sync_run_seconds_at_(__FILE__, __LINE__, (left), (right), (speed), (turn), (seconds))

/* The two runs above, returning at once; wait with ev3_motor_wait(left | right, ...). */
Ev3Result ev3_motor_try_sync_start_degrees(Ev3Port left, Ev3Port right, int speed, int turn, int degrees);
void ev3_motor_sync_start_degrees_at_(const char *file, int line, Ev3Port left, Ev3Port right, int speed, int turn,
                                      int degrees);
#define ev3_motor_sync_start_degrees(left, right, speed, turn, degrees) \
    ev3_motor_sync_start_degrees_at_(__FILE__, __LINE__, (left), (right), (speed), (turn), (degrees))

Ev3Result ev3_motor_try_sync_start_seconds(Ev3Port left, Ev3Port right, int speed, int turn, float seconds);
void ev3_motor_sync_start_seconds_at_(const char *file, int line, Ev3Port left, Ev3Port right, int speed, int turn,
                                      float seconds);
#define ev3_motor_sync_start_seconds(left, right, speed, turn, seconds) \
    ev3_motor_sync_start_seconds_at_(__FILE__, __LINE__, (left), (right), (speed), (turn), (seconds))

/*
 * Stops the pair. ev3_motor_stop(left | right, mode) does the same. A pair
 * started with ev3_motor_sync_start() counts as busy until it is stopped, so do
 * not wait for it with ev3_motor_wait().
 */
Ev3Result ev3_motor_try_sync_stop(Ev3Port left, Ev3Port right, Ev3StopMode mode);
void ev3_motor_sync_stop_at_(const char *file, int line, Ev3Port left, Ev3Port right, Ev3StopMode mode);
#define ev3_motor_sync_stop(left, right, mode) ev3_motor_sync_stop_at_(__FILE__, __LINE__, (left), (right), (mode))

#ifdef __cplusplus
}
#endif

#endif /* EV3_MOTOR_H */
