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
 *     ARM EABI layout, padding included; their three values are the ramp up,
 *     the steady part and the ramp down;
 *   - the sync commands (d_pwm.c:2704-3035) are the STEPSYNC and TIMESYNC
 *     structs the same way: two motors, a turn ratio and one amount;
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

/*
 * lms2012.h STEPSYNC and TIMESYNC: two motors, a turn ratio and one amount, in
 * steps (degrees) or milliseconds. d_pwm.c:2704-3035 takes the motors in port
 * order whatever the program calls them: a positive turn makes the lower port
 * the faster one, a negative turn the higher port.
 */
typedef struct {
    int8_t cmd;
    int8_t nos;
    int8_t speed;
    int16_t turn;
    int32_t amount;
    int8_t brake;
} ev3_sync_command;

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
_Static_assert(sizeof(ev3_sync_command) == 16, "STEPSYNC/TIMESYNC are 16 bytes");
_Static_assert(offsetof(ev3_sync_command, turn) == 4, "the turn ratio follows one byte of padding");
_Static_assert(offsetof(ev3_sync_command, amount) == 8, "the amount follows the turn and two bytes of padding");
_Static_assert(offsetof(ev3_sync_command, brake) == 12, "the brake byte follows the amount");
_Static_assert(sizeof(ev3_motor_data) == 12,"MOTORDATA is 12 bytes per port");
_Static_assert(offsetof(ev3_motor_data, tacho_sensor) == 8, "TachoSensor follows Speed and three bytes of padding");

static Ev3Result check_percent(const char *what, int value)
{
    /* A value outside -100..100 would wrap around in the command byte and could reverse the motor. */
    if (value < -100 || value > 100) {
        return ev3_fail(EV3_ERR_PARAM, "%s %d is outside -100 to 100", what, value);
    }
    return EV3_OK;
}

static Ev3Result check_stop_mode(Ev3StopMode mode)
{
    if (mode != EV3_STOP_COAST && mode != EV3_STOP_BRAKE) {
        return ev3_fail(EV3_ERR_PARAM, "stop mode %d is neither EV3_STOP_COAST nor EV3_STOP_BRAKE", (int)mode);
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

    if (result == EV3_OK) {
        result = check_stop_mode(mode);
    }
    if (result != EV3_OK) {
        return result;
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

/* A step or time command: speeding up over up, the given speed over steady, slowing down over down. */
static Ev3Result start_ramp(unsigned char opcode, Ev3Port ports, int speed, int32_t up, int32_t steady, int32_t down,
                            bool brake)
{
    ev3_step_command command;

    memset(&command, 0, sizeof command);
    command.cmd = (int8_t)opcode;
    command.nos = (int8_t)ports;
    command.speed = (int8_t)speed;
    command.value1 = up;
    command.value2 = steady;
    command.value3 = down;
    command.brake = brake ? 1 : 0;
    return write_command((const unsigned char *)&command, sizeof command);
}

/* A step or time command without a ramp: the whole amount at the given speed, then brake. */
static Ev3Result start_for(unsigned char opcode, Ev3Port ports, int speed, int32_t amount)
{
    return start_ramp(opcode, ports, speed, 0, amount, 0, true);
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

/* Whole turns as degrees, rounded. */
static Ev3Result rotations_to_degrees(float rotations, int *degrees)
{
    float value = rotations * 360.0f;

    if (!(value > -2000000000.0f && value < 2000000000.0f)) {
        return ev3_fail(EV3_ERR_PARAM, "%g rotations is out of range", (double)rotations);
    }
    *degrees = (int)(value < 0.0f ? value - 0.5f : value + 0.5f);
    return EV3_OK;
}

Ev3Result ev3_motor_try_run_rotations(Ev3Port ports, int speed, float rotations)
{
    int degrees = 0;
    Ev3Result result = rotations_to_degrees(rotations, &degrees);
    return result == EV3_OK ? ev3_motor_try_run_degrees(ports, speed, degrees) : result;
}

Ev3Result ev3_motor_try_run_seconds(Ev3Port ports, int speed, float seconds)
{
    Ev3Result result = ev3_motor_try_start_seconds(ports, speed, seconds);
    return result == EV3_OK ? ev3_motor_try_wait(ports, 0) : result;
}

/* --- ramps --------------------------------------------------------------------- */

/* A ramp's three amounts: none negative, and together within the firmware's 32-bit count. */
static Ev3Result check_ramp(const char *unit, long long up, long long steady, long long down)
{
    if (up < 0 || steady < 0 || down < 0) {
        return ev3_fail(EV3_ERR_PARAM, "%s %lld, %lld and %lld: a ramp cannot be negative; a negative speed turns the other way",
                        unit, up, steady, down);
    }
    if (up + steady + down > 2000000000LL) {
        return ev3_fail(EV3_ERR_PARAM, "%s %lld, %lld and %lld are too many together", unit, up, steady, down);
    }
    return EV3_OK;
}

Ev3Result ev3_motor_try_start_degrees_ramp(Ev3Port ports, int speed, int up, int steady, int down, Ev3StopMode mode)
{
    Ev3Result result = ev3_check_motor_ports(ports);

    if (result == EV3_OK) {
        result = check_percent("speed", speed);
    }
    if (result == EV3_OK) {
        result = check_stop_mode(mode);
    }
    if (result == EV3_OK) {
        result = check_ramp("degrees", up, steady, down);
    }
    if (result != EV3_OK) {
        return result;
    }
    /* At speed 0 a count of degrees would never end. */
    if (speed == 0 || (long long)up + steady + down == 0) {
        return EV3_OK;
    }
    return start_ramp(EV3_OP_OUTPUT_STEP_SPEED, ports, speed, up, steady, down, mode == EV3_STOP_BRAKE);
}

Ev3Result ev3_motor_try_run_degrees_ramp(Ev3Port ports, int speed, int up, int steady, int down, Ev3StopMode mode)
{
    Ev3Result result = ev3_motor_try_start_degrees_ramp(ports, speed, up, steady, down, mode);
    return result == EV3_OK ? ev3_motor_try_wait(ports, 0) : result;
}

Ev3Result ev3_motor_try_start_seconds_ramp(Ev3Port ports, int speed, float up, float steady, float down,
                                           Ev3StopMode mode)
{
    int32_t up_ms = 0, steady_ms = 0, down_ms = 0;
    Ev3Result result = ev3_check_motor_ports(ports);

    if (result == EV3_OK) {
        result = check_percent("speed", speed);
    }
    if (result == EV3_OK) {
        result = check_stop_mode(mode);
    }
    if (result == EV3_OK) {
        result = milliseconds(up, &up_ms);
    }
    if (result == EV3_OK) {
        result = milliseconds(steady, &steady_ms);
    }
    if (result == EV3_OK) {
        result = milliseconds(down, &down_ms);
    }
    if (result == EV3_OK) {
        result = check_ramp("milliseconds", up_ms, steady_ms, down_ms);
    }
    if (result != EV3_OK) {
        return result;
    }
    if ((long long)up_ms + steady_ms + down_ms == 0) {
        return EV3_OK;
    }
    return start_ramp(EV3_OP_OUTPUT_TIME_SPEED, ports, speed, up_ms, steady_ms, down_ms, mode == EV3_STOP_BRAKE);
}

Ev3Result ev3_motor_try_run_seconds_ramp(Ev3Port ports, int speed, float up, float steady, float down,
                                         Ev3StopMode mode)
{
    Ev3Result result = ev3_motor_try_start_seconds_ramp(ports, speed, up, steady, down, mode);
    return result == EV3_OK ? ev3_motor_try_wait(ports, 0) : result;
}

/* --- two motors together ------------------------------------------------------- */

/*
 * Checks a sync pair and gives the port mask and the turn as the driver reads
 * it: the driver takes the lower port as the one a positive turn speeds up, so
 * the turn changes sign when the left motor is on the higher port.
 */
static Ev3Result sync_pair(Ev3Port left, Ev3Port right, int speed, int turn, Ev3Port *ports, int *driver_turn)
{
    int left_index = 0, right_index = 0;
    Ev3Result result = ev3_check_motor_port(left, &left_index);

    if (result == EV3_OK) {
        result = ev3_check_motor_port(right, &right_index);
    }
    if (result != EV3_OK) {
        return result;
    }
    if (left_index == right_index) {
        return ev3_fail(EV3_ERR_PARAM, "left and right are both port %s; two motors together need two ports",
                        ev3_port_name(left));
    }
    result = check_percent("speed", speed);
    if (result != EV3_OK) {
        return result;
    }
    if (turn < -200 || turn > 200) {
        return ev3_fail(EV3_ERR_PARAM, "turn %d is outside -200 to 200", turn);
    }
    *ports = left | right;
    *driver_turn = left_index < right_index ? turn : -turn;
    return EV3_OK;
}

static Ev3Result start_sync(unsigned char opcode, Ev3Port ports, int speed, int turn, int32_t amount)
{
    ev3_sync_command command;

    memset(&command, 0, sizeof command);
    command.cmd = (int8_t)opcode;
    command.nos = (int8_t)ports;
    command.speed = (int8_t)speed;
    command.turn = (int16_t)turn;
    command.amount = amount;
    command.brake = 1;
    return write_command((const unsigned char *)&command, sizeof command);
}

Ev3Result ev3_motor_try_sync_start(Ev3Port left, Ev3Port right, int speed, int turn)
{
    Ev3Port ports = 0;
    int driver_turn = 0;
    Ev3Result result = sync_pair(left, right, speed, turn, &ports, &driver_turn);

    /* A step count of 0 runs until the motors are stopped (d_pwm.c:2735-2739). */
    return result == EV3_OK ? start_sync(EV3_OP_OUTPUT_STEP_SYNC, ports, speed, driver_turn, 0) : result;
}

Ev3Result ev3_motor_try_sync_start_degrees(Ev3Port left, Ev3Port right, int speed, int turn, int degrees)
{
    Ev3Port ports = 0;
    int driver_turn = 0;
    Ev3Result result = sync_pair(left, right, speed, turn, &ports, &driver_turn);

    if (result != EV3_OK) {
        return result;
    }
    if (degrees == INT32_MIN) {
        return ev3_fail(EV3_ERR_PARAM, "degrees %d is out of range", degrees);
    }
    if (degrees < 0) {
        degrees = -degrees;
        speed = -speed;
    }
    /* 0 degrees would run without end; so would a count at speed 0. */
    if (degrees == 0 || speed == 0) {
        return EV3_OK;
    }
    return start_sync(EV3_OP_OUTPUT_STEP_SYNC, ports, speed, driver_turn, degrees);
}

Ev3Result ev3_motor_try_sync_start_seconds(Ev3Port left, Ev3Port right, int speed, int turn, float seconds)
{
    Ev3Port ports = 0;
    int driver_turn = 0;
    int32_t ms = 0;
    Ev3Result result = sync_pair(left, right, speed, turn, &ports, &driver_turn);

    if (result == EV3_OK) {
        result = milliseconds(seconds, &ms);
    }
    if (result != EV3_OK || ms == 0) {
        return result;
    }
    return start_sync(EV3_OP_OUTPUT_TIME_SYNC, ports, speed, driver_turn, ms);
}

Ev3Result ev3_motor_try_sync_run_degrees(Ev3Port left, Ev3Port right, int speed, int turn, int degrees)
{
    Ev3Result result = ev3_motor_try_sync_start_degrees(left, right, speed, turn, degrees);
    return result == EV3_OK ? ev3_motor_try_wait(left | right, 0) : result;
}

Ev3Result ev3_motor_try_sync_run_rotations(Ev3Port left, Ev3Port right, int speed, int turn, float rotations)
{
    int degrees = 0;
    Ev3Result result = rotations_to_degrees(rotations, &degrees);
    return result == EV3_OK ? ev3_motor_try_sync_run_degrees(left, right, speed, turn, degrees) : result;
}

Ev3Result ev3_motor_try_sync_run_seconds(Ev3Port left, Ev3Port right, int speed, int turn, float seconds)
{
    Ev3Result result = ev3_motor_try_sync_start_seconds(left, right, speed, turn, seconds);
    return result == EV3_OK ? ev3_motor_try_wait(left | right, 0) : result;
}

Ev3Result ev3_motor_try_sync_stop(Ev3Port left, Ev3Port right, Ev3StopMode mode)
{
    int index = 0;
    Ev3Result result = ev3_check_motor_port(left, &index);

    if (result == EV3_OK) {
        result = ev3_check_motor_port(right, &index);
    }
    return result == EV3_OK ? ev3_motor_try_stop(left | right, mode) : result;
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

void ev3_motor_run_degrees_ramp_at_(EV3_AT, Ev3Port ports, int speed, int up, int steady, int down, Ev3StopMode mode)
{
    EV3_MUST("ev3_motor_run_degrees_ramp", ev3_motor_try_run_degrees_ramp(ports, speed, up, steady, down, mode));
}

void ev3_motor_start_degrees_ramp_at_(EV3_AT, Ev3Port ports, int speed, int up, int steady, int down, Ev3StopMode mode)
{
    EV3_MUST("ev3_motor_start_degrees_ramp", ev3_motor_try_start_degrees_ramp(ports, speed, up, steady, down, mode));
}

void ev3_motor_run_seconds_ramp_at_(EV3_AT, Ev3Port ports, int speed, float up, float steady, float down,
                                    Ev3StopMode mode)
{
    EV3_MUST("ev3_motor_run_seconds_ramp", ev3_motor_try_run_seconds_ramp(ports, speed, up, steady, down, mode));
}

void ev3_motor_start_seconds_ramp_at_(EV3_AT, Ev3Port ports, int speed, float up, float steady, float down,
                                      Ev3StopMode mode)
{
    EV3_MUST("ev3_motor_start_seconds_ramp", ev3_motor_try_start_seconds_ramp(ports, speed, up, steady, down, mode));
}

void ev3_motor_sync_start_at_(EV3_AT, Ev3Port left, Ev3Port right, int speed, int turn)
{
    EV3_MUST("ev3_motor_sync_start", ev3_motor_try_sync_start(left, right, speed, turn));
}

void ev3_motor_sync_run_degrees_at_(EV3_AT, Ev3Port left, Ev3Port right, int speed, int turn, int degrees)
{
    EV3_MUST("ev3_motor_sync_run_degrees", ev3_motor_try_sync_run_degrees(left, right, speed, turn, degrees));
}

void ev3_motor_sync_run_rotations_at_(EV3_AT, Ev3Port left, Ev3Port right, int speed, int turn, float rotations)
{
    EV3_MUST("ev3_motor_sync_run_rotations", ev3_motor_try_sync_run_rotations(left, right, speed, turn, rotations));
}

void ev3_motor_sync_run_seconds_at_(EV3_AT, Ev3Port left, Ev3Port right, int speed, int turn, float seconds)
{
    EV3_MUST("ev3_motor_sync_run_seconds", ev3_motor_try_sync_run_seconds(left, right, speed, turn, seconds));
}

void ev3_motor_sync_start_degrees_at_(EV3_AT, Ev3Port left, Ev3Port right, int speed, int turn, int degrees)
{
    EV3_MUST("ev3_motor_sync_start_degrees", ev3_motor_try_sync_start_degrees(left, right, speed, turn, degrees));
}

void ev3_motor_sync_start_seconds_at_(EV3_AT, Ev3Port left, Ev3Port right, int speed, int turn, float seconds)
{
    EV3_MUST("ev3_motor_sync_start_seconds", ev3_motor_try_sync_start_seconds(left, right, speed, turn, seconds));
}

void ev3_motor_sync_stop_at_(EV3_AT, Ev3Port left, Ev3Port right, Ev3StopMode mode)
{
    EV3_MUST("ev3_motor_sync_stop", ev3_motor_try_sync_stop(left, right, mode));
}
