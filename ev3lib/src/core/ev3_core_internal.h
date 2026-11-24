/* Shared between the ev3lib core sources; not part of the public API. */
#ifndef EV3_CORE_INTERNAL_H
#define EV3_CORE_INTERNAL_H

#include <stddef.h>

#include "ev3.h"

/* Firmware PWM device commands, from lms2012/source/bytecodes.h:405-426. */
#define EV3_OP_PROGRAM_STOP    0x02
#define EV3_OP_OUTPUT_STOP     0xA3
#define EV3_OP_OUTPUT_POWER    0xA4
#define EV3_OP_OUTPUT_SPEED    0xA5
#define EV3_OP_OUTPUT_START    0xA6
#define EV3_OP_OUTPUT_POLARITY 0xA7
#define EV3_OP_OUTPUT_STEP_SPEED 0xAE
#define EV3_OP_OUTPUT_TIME_SPEED 0xAF
#define EV3_OP_OUTPUT_CLR_COUNT  0xB2

/* --- errors ------------------------------------------------------------------ */

/* Records what went wrong for ev3_error_message() and returns the code. */
Ev3Result ev3_fail(Ev3Result result, const char *format, ...)
    __attribute__((format(printf, 2, 3)));

/* As ev3_fail(), with ": " and strerror(errno) after the text. */
Ev3Result ev3_fail_errno(Ev3Result result, const char *what);

/* Forgets the last message, so a plain call never prints an older one. */
void ev3_error_clear(void);

/*
 * The parameters every plain variant takes first, and the check that stops the
 * program when its try variant failed:
 *     void ev3_x_at_(EV3_AT, int a) { EV3_MUST("ev3_x", ev3_try_x(a)); }
 */
#define EV3_AT const char *file_, int line_
#define EV3_MUST(name, call)                                   \
    do {                                                       \
        Ev3Result must_result_;                                \
        ev3_error_clear();                                     \
        must_result_ = (call);                                 \
        if (must_result_ != EV3_OK) {                          \
            ev3_stop_on_error_(file_, line_, name, must_result_); \
        }                                                      \
    } while (0)

/* --- ports ------------------------------------------------------------------- */

/* "A" to "D", "1" to "4", or the value in hex. */
const char *ev3_port_name(Ev3Port port);

/* One or more motor ports combined with |. */
Ev3Result ev3_check_motor_ports(Ev3Port ports);

/* Exactly one motor port; its index 0..3 in *index. */
Ev3Result ev3_check_motor_port(Ev3Port port, int *index);

/* Exactly one sensor port; its index 0..3 in *index. */
Ev3Result ev3_check_sensor_port(Ev3Port port, int *index);

/* --- devices ------------------------------------------------------------------ */

/*
 * Writes one command to the motor device. The driver handles one command
 * per write(), so callers never join commands. Returns 0 or -1 with errno.
 */
int ev3_core_pwm_write(const unsigned char *command, size_t length);

/* Paths of the devices, from the environment or the brick's defaults. */
const char *ev3_core_pwm_device(void);
const char *ev3_core_motor_device(void);
const char *ev3_core_ui_device(void);
const char *ev3_core_sound_device(void);
const char *ev3_core_analog_device(void);
const char *ev3_core_uart_device(void);
const char *ev3_core_iic_device(void);

/* Writes `length` bytes to a device in one write(). Returns 0 or -1 with errno. */
int ev3_core_device_write(const char *device, const void *data, size_t length);

/* Sleeps, resuming after a signal. */
void ev3_core_sleep(unsigned int ms);

#endif /* EV3_CORE_INTERNAL_H */
