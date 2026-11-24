/*
 * ev3lib errors: the codes' names, the message of the last failed try call,
 * and the stop of a program whose plain call failed.
 */
#include <errno.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "ev3.h"
#include "ev3_core_internal.h"

/* One program, one thread of library calls: a static buffer is enough. */
static char message[200];

const char *ev3_error_string(Ev3Result result)
{
    switch (result) {
    case EV3_OK: return "EV3_OK";
    case EV3_ERR_PORT: return "EV3_ERR_PORT";
    case EV3_ERR_DEVICE: return "EV3_ERR_DEVICE";
    case EV3_ERR_PARAM: return "EV3_ERR_PARAM";
    case EV3_ERR_IO: return "EV3_ERR_IO";
    case EV3_ERR_TIMEOUT: return "EV3_ERR_TIMEOUT";
    case EV3_ERR_BUSY: return "EV3_ERR_BUSY";
    case EV3_ERR_NOT_SUPPORTED: return "EV3_ERR_NOT_SUPPORTED";
    case EV3_ERR_OVERFLOW: return "EV3_ERR_OVERFLOW";
    }
    return "EV3_ERR_UNKNOWN";
}

const char *ev3_error_message(void)
{
    return message;
}

Ev3Result ev3_fail(Ev3Result result, const char *format, ...)
{
    va_list args;

    va_start(args, format);
    vsnprintf(message, sizeof message, format, args);
    va_end(args);
    return result;
}

Ev3Result ev3_fail_errno(Ev3Result result, const char *what)
{
    return ev3_fail(result, "%s: %s", what, strerror(errno));
}

void ev3_error_clear(void)
{
    message[0] = '\0';
}

void ev3_stop_on_error_(const char *file, int line, const char *function, Ev3Result result)
{
    fflush(stdout);
    fprintf(stderr, "%s:%d: %s: %s\n%s\n", file, line, function,
            message[0] != '\0' ? message : ev3_error_string(result), EV3_LIBRARY_STOP_LINE);
    /* exit(), not _exit(): the exit handlers stop the motors. */
    exit(EV3_EXIT_LIBRARY_ERROR);
}

/* --- ports ------------------------------------------------------------------ */

static char port_name_buffer[8];

const char *ev3_port_name(Ev3Port port)
{
    static const char motors[] = "ABCD";
    for (int i = 0; i < 4; i++) {
        if (port == (EV3_PORT_A << i)) {
            snprintf(port_name_buffer, sizeof port_name_buffer, "%c", motors[i]);
            return port_name_buffer;
        }
        if (port == (EV3_PORT_1 << i)) {
            snprintf(port_name_buffer, sizeof port_name_buffer, "%d", i + 1);
            return port_name_buffer;
        }
    }
    if (port != 0 && (port & ~EV3_PORT_ALL) == 0) {
        /* Several motor ports, as A+B: at most "A+B+C+D", which the buffer holds. */
        size_t used = 0;
        for (int i = 0; i < 4; i++) {
            if (port & (EV3_PORT_A << i)) {
                if (used > 0) {
                    port_name_buffer[used++] = '+';
                }
                port_name_buffer[used++] = motors[i];
            }
        }
        port_name_buffer[used] = '\0';
        return port_name_buffer;
    }
    snprintf(port_name_buffer, sizeof port_name_buffer, "%#x", (unsigned int)port);
    return port_name_buffer;
}

Ev3Result ev3_check_motor_ports(Ev3Port ports)
{
    if (ports != 0 && (ports & ~EV3_PORT_ALL) == 0) {
        return EV3_OK;
    }
    if (ports & (EV3_PORT_1 | EV3_PORT_2 | EV3_PORT_3 | EV3_PORT_4)) {
        return ev3_fail(EV3_ERR_PORT, "port %s is a sensor port; motors are on ports A to D",
                        ev3_port_name(ports));
    }
    return ev3_fail(EV3_ERR_PORT, "%s is not a motor port; motors are on ports A to D (EV3_PORT_A ...)",
                    ev3_port_name(ports));
}

Ev3Result ev3_check_motor_port(Ev3Port port, int *index)
{
    Ev3Result result = ev3_check_motor_ports(port);
    if (result != EV3_OK) {
        return result;
    }
    for (int i = 0; i < 4; i++) {
        if (port == (EV3_PORT_A << i)) {
            *index = i;
            return EV3_OK;
        }
    }
    return ev3_fail(EV3_ERR_PORT, "ports %s are several; this takes one motor port", ev3_port_name(port));
}

Ev3Result ev3_check_sensor_port(Ev3Port port, int *index)
{
    for (int i = 0; i < 4; i++) {
        if (port == (EV3_PORT_1 << i)) {
            *index = i;
            return EV3_OK;
        }
    }
    if (port != 0 && (port & ~EV3_PORT_ALL) == 0) {
        return ev3_fail(EV3_ERR_PORT, "port %s is a motor port; sensors are on ports 1 to 4",
                        ev3_port_name(port));
    }
    return ev3_fail(EV3_ERR_PORT, "%s is not a sensor port; sensors are on ports 1 to 4 (EV3_PORT_1 ...)",
                    ev3_port_name(port));
}
