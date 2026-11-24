/*
 * ev3lib motors: commands for the firmware's PWM device (/dev/lms_pwm) and
 * readings from its motor device (/dev/lms_motor), as the LEGO firmware
 * sources (1.09D developer edition) define them:
 *   - d_pwm/Linuxmod_AM1808/d_pwm.c:2166-3035: one command per write(), the
 *     opcode, a port mask (bit 0 = port A) and the arguments. POWER and SPEED
 *     only take effect with START; STEP_SPEED and TIME_SPEED start at once;
 *   - the step and time commands are the STEPSPEED and TIMESPEED structs of
 *     lms2012/source/lms2012.h, copied whole with memcpy() by the driver and
 *     written with sizeof by the firmware's own output module
 *     (c_output/source/c_output.c:1138, 1224), so they keep the structs'
 *     ARM EABI layout, padding included;
 *   - d_pwm.c:3037-3045: reading the PWM device gives "<ready> <test> ", two
 *     port masks; a test bit stays set while a step or time command runs
 *     (opOUTPUT_TEST, c_output.c);
 *   - d_pwm.c:3811-3836 and lms2012.h MOTORDATA: the motor device shares one
 *     { TachoCounts, Speed, TachoSensor } per port through mmap();
 *     TachoSensor is the rotation sensor's count in degrees, which
 *     opOUTPUT_CLR_COUNT sets to 0 (d_pwm.c:2279-2293).
 */
#include <errno.h>
#include <fcntl.h>

#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#include "ev3.h"
#include "ev3_core_internal.h"

#define EV3_MOTOR_PORTS 4
#define EV3_WAIT_STEP_MS 10

/* lms2012.h STEPSPEED and TIMESPEED: the same layout, steps or milliseconds. */
typedef struct {
    int8_t cmd;
    int8_t nos;
    int8_t speed;
    int32_t value1;
    int32_t value2;
    int32_t value3;
    int8_t brake;
} ev3_step_command;

/* lms2012.h MOTORDATA, one per port. */
typedef struct {
    int32_t tacho_counts;
    int8_t speed;
    int32_t tacho_sensor;
} ev3_motor_data;

/* The driver copies these bytes as the firmware's structs: the layouts must match on every target. */
_Static_assert(sizeof(ev3_step_command) == 20, "STEPSPEED/TIMESPEED are 20 bytes");
_Static_assert(offsetof(ev3_step_command, value1) == 4, "the first 32-bit value follows one byte of padding");
_Static_assert(offsetof(ev3_step_command, brake) == 16, "the brake byte follows the three values");
_Static_assert(sizeof(ev3_motor_data) == 12, "MOTORDATA is 12 bytes per port");
_Static_assert(offsetof(ev3_motor_data, tacho_sensor) == 8, "TachoSensor follows Speed and three bytes of padding");

static Ev3Result check_percent(const char *what, int value)
{
    /* A value outside -100..100 would wrap around in the command byte and could reverse the motor. */
    if (value < -100 || value > 100) {
        return ev3_fail(EV3_ERR_PARAM, "%s %d is outside -100 to 100", what, value);
    }
    return EV3_OK;
}

static Ev3Result write_command(const unsigned char *command, size_t length)
{
    if (ev3_core_pwm_write(command, length) != 0) {
        return ev3_fail_errno(EV3_ERR_IO, "the motor device cannot be written");
    }
    return EV3_OK;
}

/* A port mask command and its START, as POWER and SPEED need. */
static Ev3Result set_and_start(unsigned char opcode, const char *what, Ev3Port ports, int value)
{
    unsigned char set[3];
    unsigned char start[2];
    Ev3Result result = ev3_check_motor_ports(ports);

    if (result == EV3_OK) {
        result = check_percent(what, value);
    }
    if (result != EV3_OK) {
        return result;
    }
    set[0] = opcode;
    set[1] = (unsigned char)ports;
    set[2] = (unsigned char)(signed char)value;
    start[0] = EV3_OP_OUTPUT_START;
    start[1] = (unsigned char)ports;
    result = write_command(set, sizeof set);
    return result == EV3_OK ? write_command(start, sizeof start) : result;
}

Ev3Result ev3_motor_try_start(Ev3Port ports, int speed)
{
    return set_and_start(EV3_OP_OUTPUT_SPEED, "speed", ports, speed);
}

Ev3Result ev3_motor_try_set_power(Ev3Port ports, int power)
{
    return set_and_start(EV3_OP_OUTPUT_POWER, "power", ports, power);
}

Ev3Result ev3_motor_try_stop(Ev3Port ports, Ev3StopMode mode)
{
    unsigned char stop[3];
    Ev3Result result = ev3_check_motor_ports(ports);

    if (result != EV3_OK) {
        return result;
    }
    if (mode != EV3_STOP_COAST && mode != EV3_STOP_BRAKE) {
        return ev3_fail(EV3_ERR_PARAM, "stop mode %d is neither EV3_STOP_COAST nor EV3_STOP_BRAKE", (int)mode);
    }
    stop[0] = EV3_OP_OUTPUT_STOP;
    stop[1] = (unsigned char)ports;
    stop[2] = mode == EV3_STOP_BRAKE ? 1 : 0;
    return write_command(stop, sizeof stop);
}

Ev3Result ev3_motor_try_set_polarity(Ev3Port ports, int polarity)
{
    unsigned char command[3];
    Ev3Result result = ev3_check_motor_ports(ports);

    if (result != EV3_OK) {
        return result;
    }
    if (polarity < -1 || polarity > 1) {
        return ev3_fail(EV3_ERR_PARAM, "polarity %d is not 1, -1 or 0", polarity);
    }
    command[0] = EV3_OP_OUTPUT_POLARITY;
    command[1] = (unsigned char)ports;
    command[2] = (unsigned char)(signed char)polarity;
    return write_command(command, sizeof command);
}

/* A step or time command: no ramp up or down, the whole amount at the given speed, then brake. */
static Ev3Result start_for(unsigned char opcode, Ev3Port ports, int speed, int32_t amount)
{
    ev3_step_command command;

    memset(&command, 0, sizeof command);
    command.cmd = (int8_t)opcode;
    command.nos = (int8_t)ports;
    command.speed = (int8_t)speed;
    command.value2 = amount;
    command.brake = 1;
    return write_command((const unsigned char *)&command, sizeof command);
}

Ev3Result ev3_motor_try_start_degrees(Ev3Port ports, int speed, int degrees)
{
    Ev3Result result = ev3_check_motor_ports(ports);

    if (result == EV3_OK) {
        result = check_percent("speed", speed);
    }
    if (result != EV3_OK) {
        return result;
    }
    if (degrees == INT32_MIN) {
        return ev3_fail(EV3_ERR_PARAM, "degrees %d is out of range", degrees);
    }
    /* The firmware counts steps up from 0; a negative amount turns the other way. */
    if (degrees < 0) {
        degrees = -degrees;
        speed = -speed;
    }
    if (degrees == 0 || speed == 0) {
        return EV3_OK;
    }
    return start_for(EV3_OP_OUTPUT_STEP_SPEED, ports, speed, degrees);
}

/* Seconds as whole milliseconds, refused when negative or too large. */
static Ev3Result milliseconds(float seconds, int32_t *ms)
{
    if (!(seconds >= 0.0f) || seconds > 2000000.0f) {
        return ev3_fail(EV3_ERR_PARAM, "%g seconds is not a time from 0 to 2000000", (double)seconds);
    }
    *ms = (int32_t)(seconds * 1000.0f + 0.5f);
    return EV3_OK;
}

Ev3Result ev3_motor_try_start_seconds(Ev3Port ports, int speed, float seconds)
{
    int32_t ms;
    Ev3Result result = ev3_check_motor_ports(ports);

    if (result == EV3_OK) {
        result = check_percent("speed", speed);
    }
    if (result == EV3_OK) {
        result = milliseconds(seconds, &ms);
    }
    if (result != EV3_OK) {
        return result;
    }
    if (ms == 0) {
        return EV3_OK;
    }
    return start_for(EV3_OP_OUTPUT_TIME_SPEED, ports, speed, ms);
}

Ev3Result ev3_motor_try_run_degrees(Ev3Port ports, int speed, int degrees)
{
    Ev3Result result = ev3_motor_try_start_degrees(ports, speed, degrees);
    return result == EV3_OK ? ev3_motor_try_wait(ports, 0) : result;
}

Ev3Result ev3_motor_try_run_rotations(Ev3Port ports, int speed, float rotations)
{
    float degrees = rotations * 360.0f;

    if (!(degrees > -2000000000.0f && degrees < 2000000000.0f)) {
        return ev3_fail(EV3_ERR_PARAM, "%g rotations is out of range", (double)rotations);
    }
    return ev3_motor_try_run_degrees(ports, speed, (int)(degrees < 0.0f ? degrees - 0.5f : degrees + 0.5f));
}

Ev3Result ev3_motor_try_run_seconds(Ev3Port ports, int speed, float seconds)
{
    Ev3Result result = ev3_motor_try_start_seconds(ports, speed, seconds);
    return result == EV3_OK ? ev3_motor_try_wait(ports, 0) : result;
}

Ev3Result ev3_motor_try_is_busy(Ev3Port ports, bool *busy)
{
    char text[32];
    unsigned int ready = 0;
    unsigned int test = 0;
    ssize_t length;
    int fd;
    Ev3Result result = ev3_check_motor_ports(ports);

    if (result != EV3_OK) {
        return result;
    }
    if (busy == NULL) {
        return ev3_fail(EV3_ERR_PARAM, "busy points nowhere (NULL)");
    }
    fd = open(ev3_core_pwm_device(), O_RDONLY);
    if (fd < 0) {
        return ev3_fail_errno(EV3_ERR_IO, "the motor device cannot be opened");
    }
    length = read(fd, text, sizeof text - 1);
    close(fd);
    if (length <= 0) {
        return ev3_fail(EV3_ERR_IO, "the motor device gave no state");
    }
    text[length] = '\0';
    if (sscanf(text, "%u %u", &ready, &test) != 2) {
        return ev3_fail(EV3_ERR_IO, "the motor device gave a state that cannot be read");
    }
    *busy = (test & (unsigned int)ports) != 0;
    return EV3_OK;
}

static long long now_ms(void)
{
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (long long)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

Ev3Result ev3_motor_try_wait(Ev3Port ports, int timeout_ms)
{
    long long deadline;
    bool busy = false;
    Ev3Result result;

    if (timeout_ms < 0) {
        return ev3_fail(EV3_ERR_PARAM, "timeout %d ms is negative; 0 waits without a limit", timeout_ms);
    }
    deadline = now_ms() + timeout_ms;
    while ((result = ev3_motor_try_is_busy(ports, &busy)) == EV3_OK && busy) {
        if (timeout_ms > 0 && now_ms() >= deadline) {
            return ev3_fail(EV3_ERR_TIMEOUT, "the motors were not done within %d ms", timeout_ms);
        }
        ev3_core_sleep(EV3_WAIT_STEP_MS);
    }
    return result;
}

/* Reads one port's MOTORDATA. */
static Ev3Result read_motor(Ev3Port port, ev3_motor_data *data)
{
    size_t size = sizeof(ev3_motor_data) * EV3_MOTOR_PORTS;
    volatile const ev3_motor_data *motors;
    void *mapped;
    int index;
    int fd;
    Ev3Result result = ev3_check_motor_port(port, &index);

    if (result != EV3_OK) {
        return result;
    }
    fd = open(ev3_core_motor_device(), O_RDONLY);
    if (fd < 0) {
        return ev3_fail_errno(EV3_ERR_IO, "the motor measurement device cannot be opened");
    }
    mapped = mmap(NULL, size, PROT_READ, MAP_SHARED, fd, 0);
    close(fd);
    if (mapped == MAP_FAILED) {
        return ev3_fail_errno(EV3_ERR_IO, "the motor measurements cannot be mapped");
    }
    motors = (volatile const ev3_motor_data *)mapped;
    data->tacho_counts = motors[index].tacho_counts;
    data->speed = motors[index].speed;
    data->tacho_sensor = motors[index].tacho_sensor;
    munmap(mapped, size);
    return EV3_OK;
}

Ev3Result ev3_motor_try_get_position(Ev3Port port, int *degrees)
{
    ev3_motor_data data;
    Ev3Result result;

    if (degrees == NULL) {
        return ev3_fail(EV3_ERR_PARAM, "degrees points nowhere (NULL)");
    }
    result = read_motor(port, &data);
    if (result == EV3_OK) {
        *degrees = data.tacho_sensor;
    }
    return result;
}

Ev3Result ev3_motor_try_get_speed(Ev3Port port, int *speed)
{
    ev3_motor_data data;
    Ev3Result result;

    if (speed == NULL) {
        return ev3_fail(EV3_ERR_PARAM, "speed points nowhere (NULL)");
    }
    result = read_motor(port, &data);
    if (result == EV3_OK) {
        *speed = data.speed;
    }
    return result;
}

Ev3Result ev3_motor_try_reset_position(Ev3Port ports)
{
    unsigned char command[2];
    Ev3Result result = ev3_check_motor_ports(ports);

    if (result != EV3_OK) {
        return result;
    }
    command[0] = EV3_OP_OUTPUT_CLR_COUNT;
    command[1] = (unsigned char)ports;
    return write_command(command, sizeof command);
}

/* --- plain variants ------------------------------------------------------------ */

void ev3_motor_start_at_(EV3_AT, Ev3Port ports, int speed)
{
    EV3_MUST("ev3_motor_start", ev3_motor_try_start(ports, speed));
}

void ev3_motor_set_power_at_(EV3_AT, Ev3Port ports, int power)
{
    EV3_MUST("ev3_motor_set_power", ev3_motor_try_set_power(ports, power));
}

void ev3_motor_stop_at_(EV3_AT, Ev3Port ports, Ev3StopMode mode)
{
    EV3_MUST("ev3_motor_stop", ev3_motor_try_stop(ports, mode));
}

void ev3_motor_run_degrees_at_(EV3_AT, Ev3Port ports, int speed, int degrees)
{
    EV3_MUST("ev3_motor_run_degrees", ev3_motor_try_run_degrees(ports, speed, degrees));
}

void ev3_motor_run_rotations_at_(EV3_AT, Ev3Port ports, int speed, float rotations)
{
    EV3_MUST("ev3_motor_run_rotations", ev3_motor_try_run_rotations(ports, speed, rotations));
}

void ev3_motor_run_seconds_at_(EV3_AT, Ev3Port ports, int speed, float seconds)
{
    EV3_MUST("ev3_motor_run_seconds", ev3_motor_try_run_seconds(ports, speed, seconds));
}

void ev3_motor_set_polarity_at_(EV3_AT, Ev3Port ports, int polarity)
{
    EV3_MUST("ev3_motor_set_polarity", ev3_motor_try_set_polarity(ports, polarity));
}

int ev3_motor_get_position_at_(EV3_AT, Ev3Port port)
{
    int degrees = 0;
    EV3_MUST("ev3_motor_get_position", ev3_motor_try_get_position(port, &degrees));
    return degrees;
}

int ev3_motor_get_speed_at_(EV3_AT, Ev3Port port)
{
    int speed = 0;
    EV3_MUST("ev3_motor_get_speed", ev3_motor_try_get_speed(port, &speed));
    return speed;
}

void ev3_motor_reset_position_at_(EV3_AT, Ev3Port ports)
{
    EV3_MUST("ev3_motor_reset_position", ev3_motor_try_reset_position(ports));
}

bool ev3_motor_is_busy_at_(EV3_AT, Ev3Port ports)
{
    bool busy = false;
    EV3_MUST("ev3_motor_is_busy", ev3_motor_try_is_busy(ports, &busy));
    return busy;
}

bool ev3_motor_wait_at_(EV3_AT, Ev3Port ports, int timeout_ms)
{
    Ev3Result result;

    ev3_error_clear();
    result = ev3_motor_try_wait(ports, timeout_ms);
    if (result == EV3_ERR_TIMEOUT) {
        return false;
    }
    if (result != EV3_OK) {
        ev3_stop_on_error_(file_, line_, "ev3_motor_wait", result);
    }
    return true;
}

void ev3_motor_start_degrees_at_(EV3_AT, Ev3Port ports, int speed, int degrees)
{
    EV3_MUST("ev3_motor_start_degrees", ev3_motor_try_start_degrees(ports, speed, degrees));
}

void ev3_motor_start_seconds_at_(EV3_AT, Ev3Port ports, int speed, float seconds)
{
    EV3_MUST("ev3_motor_start_seconds", ev3_motor_try_start_seconds(ports, speed, seconds));
}
