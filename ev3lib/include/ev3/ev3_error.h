/*
 * ev3lib errors and ports.
 *
 * Every function comes in two variants:
 *
 *   ev3_motor_start(EV3_PORT_A, 50);
 *       The plain variant, for most programs. It returns the value it reads, or
 *       nothing. On an error it prints where the error happened and what went
 *       wrong to stderr, for example
 *           main.c:12: ev3_motor_start: port 1 is a sensor port; motors are on ports A to D
 *       and ends the program with exit code EV3_EXIT_LIBRARY_ERROR. The motors
 *       stop, as at every end of a program.
 *
 *   Ev3Result result = ev3_motor_try_start(EV3_PORT_A, 50);
 *       The try variant, for a program that handles errors itself. It always
 *       returns, with EV3_OK or an error code; a value it reads comes back
 *       through a pointer. ev3_error_message() says what went wrong.
 *
 * The plain variants are macros, so they can name the line that called them.
 */
#ifndef EV3_ERROR_H
#define EV3_ERROR_H

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    EV3_OK                = 0,
    EV3_ERR_PORT          = -1, /* not a port, or a port of the other kind */
    EV3_ERR_DEVICE        = -2, /* nothing, or something else, is plugged into the port */
    EV3_ERR_PARAM         = -3, /* a value outside its range */
    EV3_ERR_IO            = -4, /* the firmware's device could not be read or written */
    EV3_ERR_TIMEOUT       = -5, /* a wait ran out of time */
    EV3_ERR_BUSY          = -6, /* the device is busy */
    EV3_ERR_NOT_SUPPORTED = -7, /* the original firmware cannot do this */
    EV3_ERR_OVERFLOW      = -8  /* a text or data longer than allowed */
} Ev3Result;

/* The exit code of a program the plain variant of a function stopped. */
#define EV3_EXIT_LIBRARY_ERROR 70

/* The last line a stopped program writes to stderr, after the error itself. */
#define EV3_LIBRARY_STOP_LINE "ev3lib stopped the program because of this error."

/* The name of an error code, such as "EV3_ERR_PORT". */
const char *ev3_error_string(Ev3Result result);

/*
 * What went wrong in the last try call that failed, in words, such as
 * "port 1 is a sensor port; motors are on ports A to D". Empty before the
 * first error.
 */
const char *ev3_error_message(void);

/* --- ports ------------------------------------------------------------------ */

/*
 * Motor ports A to D can be combined with |, for commands that drive several
 * motors at once: ev3_motor_start(EV3_PORT_A | EV3_PORT_B, 50). Sensor ports 1
 * to 4 have values of their own, so a sensor port passed to a motor function,
 * or the other way round, is an error the library catches.
 */
typedef int Ev3Port;

#define EV3_PORT_A   0x0001
#define EV3_PORT_B   0x0002
#define EV3_PORT_C   0x0004
#define EV3_PORT_D   0x0008
#define EV3_PORT_ALL 0x000F

#define EV3_PORT_1   0x0100
#define EV3_PORT_2   0x0200
#define EV3_PORT_3   0x0400
#define EV3_PORT_4   0x0800

/* --- used by the plain variants; not for programs ----------------------------- */

/*
 * Prints "file:line: function: message" and EV3_LIBRARY_STOP_LINE to stderr and
 * ends the program with EV3_EXIT_LIBRARY_ERROR.
 */
#if defined(__cplusplus)
[[noreturn]]
#else
_Noreturn
#endif
void ev3_stop_on_error_(const char *file, int line, const char *function, Ev3Result result);

#ifdef __cplusplus
}
#endif

#endif /* EV3_ERROR_H */
