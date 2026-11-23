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

static int limit(int value)
{
    /* A value outside -100..100 would wrap around in the command byte and could reverse the motor. */
    return value > 100 ? 100 : value < -100 ? -100 : value;
}

/* A port mask command and its START, as POWER and SPEED need. */
static int set_and_start(unsigned char opcode, int ports, int value)
{
    unsigned char set[3];
    unsigned char start[2];

    set[0] = opcode;
    set[1] = (unsigned char)ports;
    set[2] = (unsigned char)limit(value);
    start[0] = EV3_OP_OUTPUT_START;
    start[1] = (unsigned char)ports;
    if (ev3_core_pwm_write(set, sizeof set) != 0) {
        return -1;
    }
    return ev3_core_pwm_write(start, sizeof start);
}

int ev3_motor_set_power(int ports, int power)
{
    return set_and_start(EV3_OP_OUTPUT_POWER, ports, power);
}

int ev3_motor_set_speed(int ports, int speed)
{
    return set_and_start(EV3_OP_OUTPUT_SPEED, ports, speed);
}

int ev3_motor_stop(int ports, int brake)
{
    unsigned char stop[3];

    stop[0] = EV3_OP_OUTPUT_STOP;
    stop[1] = (unsigned char)ports;
    stop[2] = brake ? 1 : 0;
    return ev3_core_pwm_write(stop, sizeof stop);
}

int ev3_motor_set_polarity(int ports, int polarity)
{
    unsigned char command[3];

    if (polarity < -1 || polarity > 1) {
        errno = EINVAL;
        return -1;
    }
    command[0] = EV3_OP_OUTPUT_POLARITY;
    command[1] = (unsigned char)ports;
    command[2] = (unsigned char)(signed char)polarity;
    return ev3_core_pwm_write(command, sizeof command);
}

/* A step or time command: no ramp up or down, the whole amount at the given speed. */
static int run_for(unsigned char opcode, int ports, int speed, unsigned int amount, int brake)
{
    ev3_step_command command;

    memset(&command, 0, sizeof command);
    command.cmd = (int8_t)opcode;
    command.nos = (int8_t)ports;
    command.speed = (int8_t)limit(speed);
    command.value2 = amount > INT32_MAX ? INT32_MAX : (int32_t)amount;
    command.brake = brake ? 1 : 0;
    return ev3_core_pwm_write((const unsigned char *)&command, sizeof command);
}

int ev3_motor_run_degrees(int ports, int speed, unsigned int degrees, int brake)
{
    return run_for(EV3_OP_OUTPUT_STEP_SPEED, ports, speed, degrees, brake);
}

int ev3_motor_run_ms(int ports, int speed, unsigned int ms, int brake)
{
    return run_for(EV3_OP_OUTPUT_TIME_SPEED, ports, speed, ms, brake);
}

int ev3_motor_busy(int ports)
{
    char text[32];
    unsigned int ready = 0;
    unsigned int test = 0;
    ssize_t length;
    int fd = open(ev3_core_pwm_device(), O_RDONLY);

    if (fd < 0) {
        return -1;
    }
    length = read(fd, text, sizeof text - 1);
    close(fd);
    if (length <= 0) {
        errno = length < 0 ? errno : EIO;
        return -1;
    }
    text[length] = '\0';
    if (sscanf(text, "%u %u", &ready, &test) != 2) {
        errno = EIO;
        return -1;
    }
    return (test & (unsigned int)ports) != 0;
}

int ev3_motor_wait(int ports)
{
    int busy;

    while ((busy = ev3_motor_busy(ports)) == 1) {
        ev3_core_sleep_ms(EV3_WAIT_STEP_MS);
    }
    return busy == 0 ? 0 : -1;
}

/* The index of the one port in the mask, or -1. */
static int port_index(int port)
{
    for (int i = 0; i < EV3_MOTOR_PORTS; i++) {
        if (port == (1 << i)) {
            return i;
        }
    }
    return -1;
}

int ev3_motor_read(int port, int *speed, int *degrees)
{
    size_t size = sizeof(ev3_motor_data) * EV3_MOTOR_PORTS;
    volatile const ev3_motor_data *motors;
    void *mapped;
    int index = port_index(port);
    int fd;

    if (index < 0) {
        errno = EINVAL;
        return -1;
    }
    fd = open(ev3_core_motor_device(), O_RDONLY);
    if (fd < 0) {
        return -1;
    }
    mapped = mmap(NULL, size, PROT_READ, MAP_SHARED, fd, 0);
    close(fd);
    if (mapped == MAP_FAILED) {
        return -1;
    }
    motors = (volatile const ev3_motor_data *)mapped;
    if (speed != NULL) {
        *speed = motors[index].speed;
    }
    if (degrees != NULL) {
        *degrees = motors[index].tacho_sensor;
    }
    munmap(mapped, size);
    return 0;
}

int ev3_motor_reset_count(int ports)
{
    unsigned char command[2];

    command[0] = EV3_OP_OUTPUT_CLR_COUNT;
    command[1] = (unsigned char)ports;
    return ev3_core_pwm_write(command, sizeof command);
}
