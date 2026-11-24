/*
 * ev3lib sensors: the firmware's analog and UART drivers, read through their
 * shared memory, as the LEGO firmware sources (1.09D developer edition) define
 * them:
 *   - lms2012/source/lms2012.h:1006-1038 ANALOG (/dev/lms_analog) and
 *     1061-1077 UART (/dev/lms_uart), with the fast datalog buffer and the old
 *     NXT color support compiled in and the preempted VM left out
 *     (lms2012.h:157-171), so the layouts below match them byte for byte;
 *   - ANALOG.InConn[port] tells what is plugged in (lms2012.h:637-652:
 *     CONN_INPUT_DUMB 121 for the EV3 touch sensor, CONN_INPUT_UART 122 for the
 *     EV3 color, ultrasonic, gyro and IR sensors, CONN_NONE 126), and
 *     ANALOG.InDcm[port] the type of a dumb device (bytecodes.h:1000: TOUCH 16);
 *   - a UART sensor describes its modes itself: the driver keeps each mode's
 *     name, data format and ranges in UART.TypeData[port][mode] (lms2012.h:806-829),
 *     so a mode is looked up by its name and its data decoded by its format;
 *   - the newest reading of a UART sensor is UART.Raw[port][Actual[port]]
 *     (c_input.c:2083-2161), valid while Status has UART_DATA_READY and not
 *     UART_PORT_CHANGED (c_input.c:2983-2989);
 *   - a mode is set with the UART_SET_CONN ioctl and a DEVCON for all four
 *     ports (d_uart.c:3581-3624): a port whose Connection is not
 *     CONN_INPUT_UART has its UART shut down, so every port gets its own
 *     InConn; a changed mode clears UART_DATA_READY until the sensor sends data
 *     in the new mode;
 *   - the EV3 touch sensor reads on pin 6, between 1000 (released) and 2000
 *     (pressed) (c_input.c:2645, typedata.rcf type 16).
 */
#include <errno.h>
#include <fcntl.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <time.h>
#include <unistd.h>

#include "ev3.h"
#include "ev3_core_internal.h"

#define PORTS 4
#define MODES 8
#define LOG_SIZE 300
#define DATA_LENGTH 32

#define CONN_INPUT_DUMB 121
#define CONN_INPUT_UART 122
#define CONN_NONE       126
#define CONN_ERROR      127

#define TYPE_TOUCH      16
#define TYPE_COLOR      29
#define TYPE_ULTRASONIC 30
#define TYPE_GYRO       32
#define TYPE_IR         33

#define UART_PORT_CHANGED 0x01
#define UART_DATA_READY   0x08

#define DATA_8  0
#define DATA_16 1
#define DATA_32 2
#define DATA_F  3

#define TOUCH_THRESHOLD 1500
#define MODE_WAIT_MS 1500
#define NEXT_FRAME_WAIT_MS 300
#define FLAG_DROP_WAIT_MS 200
#define POLL_MS 5

/* lms2012.h TYPES */
typedef struct {
    char name[12];
    int8_t type;
    int8_t connection;
    int8_t mode;
    int8_t data_sets;
    int8_t format;
    int8_t figures;
    int8_t decimals;
    int8_t views;
    float raw_min;
    float raw_max;
    float pct_min;
    float pct_max;
    float si_min;
    float si_max;
    uint16_t invalid_time;
    uint16_t id_value;
    int8_t pins;
    char symbol[5];
    uint16_t align;
} ev3_types;

/* lms2012.h UART */
typedef struct {
    ev3_types type_data[PORTS][MODES];
    uint16_t repeat[PORTS][LOG_SIZE];
    int8_t raw[PORTS][LOG_SIZE][DATA_LENGTH];
    uint16_t actual[PORTS];
    uint16_t log_in[PORTS];
    int8_t status[PORTS];
    int8_t output[PORTS][DATA_LENGTH];
    int8_t output_length[PORTS];
} ev3_uart;

/* lms2012.h COLORSTRUCT */
typedef struct {
    uint32_t calibration[3][4];
    uint16_t cal_limits[2];
    uint16_t crc;
    uint16_t ad_raw[4];
    uint16_t sensor_raw[4];
} ev3_nxt_color;

/* lms2012.h ANALOG */
typedef struct {
    int16_t in_pin1[PORTS];
    int16_t in_pin6[PORTS];
    int16_t out_pin5[PORTS];
    int16_t battery_temp;
    int16_t motor_current;
    int16_t battery_current;
    int16_t cell123456;
    int16_t pin1[PORTS][LOG_SIZE];
    int16_t pin6[PORTS][LOG_SIZE];
    uint16_t actual[PORTS];
    uint16_t log_in[PORTS];
    uint16_t log_out[PORTS];
    ev3_nxt_color nxt_color[PORTS];
    int16_t out_pin5_low[PORTS];
    int8_t updated[PORTS];
    int8_t in_dcm[PORTS];
    int8_t in_conn[PORTS];
    int8_t out_dcm[PORTS];
    int8_t out_conn[PORTS];
} ev3_analog;

/* lms2012.h DEVCON */
typedef struct {
    int8_t connection[PORTS];
    int8_t type[PORTS];
    int8_t mode[PORTS];
} ev3_devcon;

_Static_assert(sizeof(ev3_types) == 56, "TYPES is 56 bytes");
_Static_assert(offsetof(ev3_types, raw_min) == 20, "TYPES' floats start at 20");
_Static_assert(offsetof(ev3_types, symbol) == 49, "TYPES' symbol starts at 49");
_Static_assert(offsetof(ev3_uart, raw) == 4192, "UART.Raw starts at 4192");
_Static_assert(offsetof(ev3_uart, actual) == 42592, "UART.Actual starts at 42592");
_Static_assert(offsetof(ev3_uart, status) == 42608, "UART.Status starts at 42608");
_Static_assert(sizeof(ev3_uart) == 42744, "UART is 42744 bytes");
_Static_assert(sizeof(ev3_nxt_color) == 72, "COLORSTRUCT is 72 bytes");
_Static_assert(offsetof(ev3_analog, nxt_color) == 4856, "ANALOG.NxtCol starts at 4856");
_Static_assert(offsetof(ev3_analog, in_dcm) == 5156, "ANALOG.InDcm starts at 5156");
_Static_assert(offsetof(ev3_analog, in_conn) == 5160, "ANALOG.InConn starts at 5160");
_Static_assert(sizeof(ev3_analog) == 5172, "ANALOG is 5172 bytes");
_Static_assert(sizeof(ev3_devcon) == 12, "DEVCON is 12 bytes");

/* lms2012.h UARTCTL */
typedef struct {
    ev3_types type_data;
    int8_t port;
    int8_t mode;
} ev3_uartctl;

_Static_assert(sizeof(ev3_uartctl) == 60, "UARTCTL is 60 bytes");

/* lms2012.h:1107-1108 */
#define UART_SET_CONN       _IOWR('u', 0, ev3_devcon)
#define UART_READ_MODE_INFO _IOWR('u', 1, ev3_uartctl)

/* lms2012.h:354-357, 612: an analog count in millivolts, and the span of one ID on pin 1. */
#define COUNTS_TO_MV(c) ((int)(c) * 5000 / 4095)
#define ID_HYSTERESIS_MV 50
/* typedata.rcf: the EV3 touch sensor's ID on pin 1. */
#define TOUCH_ID_MV 417

static volatile ev3_analog *analog;
static volatile ev3_uart *uart;
static int uart_fd = -1;
/* The mode ev3lib last set on each port, or -1. */
static int modes[PORTS] = { -1, -1, -1, -1 };

static Ev3Result map_device(const char *device, size_t size, int flags, int *fd, volatile void **mapped)
{
    void *memory;
    int opened = open(device, flags);

    if (opened < 0) {
        return ev3_fail_errno(EV3_ERR_IO, "the sensor device cannot be opened");
    }
    memory = mmap(NULL, size, PROT_READ, MAP_SHARED, opened, 0);
    if (memory == MAP_FAILED) {
        int saved = errno;
        close(opened);
        errno = saved;
        return ev3_fail_errno(EV3_ERR_IO, "the sensor data cannot be mapped");
    }
    if (fd != NULL) {
        *fd = opened;
    } else {
        close(opened);
    }
    *mapped = memory;
    return EV3_OK;
}

static Ev3Result use_analog(void)
{
    volatile void *mapped;
    Ev3Result result;

    if (analog != NULL) {
        return EV3_OK;
    }
    result = map_device(ev3_core_analog_device(), sizeof(ev3_analog), O_RDONLY, NULL, &mapped);
    if (result == EV3_OK) {
        analog = (volatile ev3_analog *)mapped;
    }
    return result;
}

static Ev3Result use_uart(void)
{
    volatile void *mapped;
    Ev3Result result;

    if (uart != NULL) {
        return EV3_OK;
    }
    result = map_device(ev3_core_uart_device(), sizeof(ev3_uart), O_RDWR, &uart_fd, &mapped);
    if (result == EV3_OK) {
        uart = (volatile ev3_uart *)mapped;
    }
    return result;
}

static const char *type_name(Ev3SensorType type)
{
    switch (type) {
    case EV3_SENSOR_TOUCH: return "a touch sensor";
    case EV3_SENSOR_COLOR: return "a color sensor";
    case EV3_SENSOR_ULTRASONIC: return "an ultrasonic sensor";
    case EV3_SENSOR_GYRO: return "a gyro sensor";
    case EV3_SENSOR_IR: return "an infrared sensor";
    case EV3_SENSOR_OTHER: return "another device";
    case EV3_SENSOR_NONE: break;
    }
    return "nothing";
}

/*
 * A mode's description, as the UART sensor sent it. The driver keeps it in an
 * array of its own (d_uart.c:2239-2910), not in the shared UART.TypeData, and
 * hands it out with UART_READ_MODE_INFO (d_uart.c:3626-3648). That call also
 * clears the "changed" flag the firmware's device manager reads while it
 * learns a new sensor's modes, so it is made only once the manager is done,
 * when the port no longer reports UART_PORT_CHANGED. It blanks the first
 * character of a name that did not change, so names are compared from the
 * second character on. A fake device file in the host tests takes no ioctl
 * (ENOTTY); then the description comes from the shared UART.TypeData.
 */
static Ev3Result mode_info(int index, int mode, ev3_types *info)
{
    ev3_uartctl control;

    if (uart->status[index] & UART_PORT_CHANGED) {
        return ev3_fail(EV3_ERR_BUSY, "the sensor on port %d is still starting; try again in a moment", index + 1);
    }
    memset(&control, 0, sizeof control);
    control.port = (int8_t)index;
    control.mode = (int8_t)mode;
    if (ioctl(uart_fd, UART_READ_MODE_INFO, &control) == 0) {
        *info = control.type_data;
        return EV3_OK;
    }
    if (errno != ENOTTY) {
        return ev3_fail_errno(EV3_ERR_IO, "the sensor's modes cannot be read");
    }
    for (size_t i = 0; i < sizeof *info; i++) {
        ((char *)info)[i] = ((volatile const char *)&uart->type_data[index][mode])[i];
    }
    return EV3_OK;
}

/* Whether the touch sensor's ID resistor is on pin 1 (c_input.c:968-978). */
static bool is_touch(int index)
{
    int id = COUNTS_TO_MV(analog->in_pin1[index]);
    return id >= TOUCH_ID_MV - ID_HYSTERESIS_MV && id < TOUCH_ID_MV + ID_HYSTERESIS_MV;
}

static Ev3Result type_at(int index, Ev3SensorType *type)
{
    ev3_types info;
    Ev3Result result = use_analog();
    int connection;

    if (result != EV3_OK) {
        return result;
    }
    connection = analog->in_conn[index];
    if (connection == CONN_NONE || connection == CONN_ERROR || connection == 0) {
        *type = EV3_SENSOR_NONE;
        return EV3_OK;
    }
    if (connection == CONN_INPUT_DUMB) {
        *type = is_touch(index) ? EV3_SENSOR_TOUCH : EV3_SENSOR_OTHER;
        return EV3_OK;
    }
    if (connection != CONN_INPUT_UART) {
        *type = EV3_SENSOR_OTHER;
        return EV3_OK;
    }
    result = use_uart();
    if (result == EV3_OK) {
        result = mode_info(index, 0, &info);
    }
    if (result != EV3_OK) {
        return result;
    }
    switch (info.type) {
    case TYPE_COLOR: *type = EV3_SENSOR_COLOR; break;
    case TYPE_ULTRASONIC: *type = EV3_SENSOR_ULTRASONIC; break;
    case TYPE_GYRO: *type = EV3_SENSOR_GYRO; break;
    case TYPE_IR: *type = EV3_SENSOR_IR; break;
    case 0:
        return ev3_fail(EV3_ERR_BUSY, "the sensor on port %d is still starting; try again in a moment", index + 1);
    default: *type = EV3_SENSOR_OTHER; break;
    }
    return EV3_OK;
}

Ev3Result ev3_sensor_try_get_type(Ev3Port port, Ev3SensorType *type)
{
    int index;
    Ev3Result result = ev3_check_sensor_port(port, &index);

    if (result == EV3_OK && type == NULL) {
        result = ev3_fail(EV3_ERR_PARAM, "type points nowhere (NULL)");
    }
    return result == EV3_OK ? type_at(index, type) : result;
}

/* The port's index when the wanted sensor is plugged into it. */
static Ev3Result expect(Ev3Port port, Ev3SensorType wanted, int *index)
{
    Ev3SensorType found;
    Ev3Result result = ev3_check_sensor_port(port, index);

    if (result == EV3_OK) {
        result = type_at(*index, &found);
    }
    if (result != EV3_OK) {
        return result;
    }
    if (found == EV3_SENSOR_NONE) {
        return ev3_fail(EV3_ERR_DEVICE, "nothing is plugged into port %d; plug in %s", *index + 1, type_name(wanted));
    }
    if (found != wanted) {
        return ev3_fail(EV3_ERR_DEVICE, "port %d has %s, not %s", *index + 1, type_name(found), type_name(wanted));
    }
    return EV3_OK;
}

static long long now_ms(void)
{
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (long long)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

/*
 * The mode named `name`, as the sensor describes its modes, with its
 * description; `fallback` when no mode has that name.
 */
static Ev3Result mode_named(int index, const char *name, int fallback, int *mode, ev3_types *info)
{
    for (int candidate = 0; candidate < MODES; candidate++) {
        char found[sizeof info->name + 1];
        Ev3Result result = mode_info(index, candidate, info);
        if (result != EV3_OK) {
            return result;
        }
        memcpy(found, info->name, sizeof info->name);
        found[sizeof found - 1] = '\0';
        if (info->type != 0 && strcmp(found + 1, name + 1) == 0) {
            *mode = candidate;
            return EV3_OK;
        }
    }
    *mode = fallback;
    return mode_info(index, fallback, info);
}

/*
 * Sets the port's mode (sent every time: the firmware's own device manager
 * may have set another one since), waits until the sensor sends data in it,
 * and gives the newest values, decoded by the mode's format.
 */
static Ev3Result uart_values(int index, const char *mode_name, int fallback, int32_t *values, int count,
                             ev3_types *info)
{
    ev3_devcon devcon;
    long long deadline;
    unsigned int actual;
    bool changed;
    int8_t data[DATA_LENGTH];
    int mode;
    Ev3Result result = use_uart();

    if (result != EV3_OK) {
        return result;
    }
    result = mode_named(index, mode_name, fallback, &mode, info);
    if (result != EV3_OK) {
        return result;
    }
    changed = modes[index] != mode;
    modes[index] = mode;
    for (int port = 0; port < PORTS; port++) {
        /* The driver reads only Connection and Mode (d_uart.c:3586-3620). */
        devcon.connection[port] = analog->in_conn[port];
        devcon.type[port] = port == index ? info->type : 0;
        devcon.mode[port] = (int8_t)(modes[port] >= 0 ? modes[port] : 0);
    }
    /* A fake device file in the host tests takes no ioctl (ENOTTY); the brick's driver always does. */
    if (ioctl(uart_fd, UART_SET_CONN, &devcon) != 0 && errno != ENOTTY) {
        return ev3_fail_errno(EV3_ERR_IO, "the sensor's mode cannot be set");
    }
    /*
     * A changed mode clears UART_DATA_READY until data in the new mode arrive
     * (d_uart.c:3605-3607, 3096-3102). On a brick the flag can still be set for
     * a moment after the ioctl, with the old mode's data behind it (seen on
     * BBB: the first RGB reading after a reflected one), so after a change the
     * flag must first go, then come back.
     */
    if (changed) {
        deadline = now_ms() + FLAG_DROP_WAIT_MS;
        while ((uart->status[index] & UART_DATA_READY) != 0 && now_ms() < deadline) {
            ev3_core_sleep(POLL_MS);
        }
    }
    deadline = now_ms() + MODE_WAIT_MS;
    while ((uart->status[index] & (UART_DATA_READY | UART_PORT_CHANGED)) != UART_DATA_READY) {
        if (now_ms() >= deadline) {
            return ev3_fail(EV3_ERR_TIMEOUT, "the sensor on port %d sent no data in mode %s", index + 1, mode_name);
        }
        ev3_core_sleep(POLL_MS);
    }
    if (changed) {
        /* The first reading after a change of mode can be one the sensor had not measured yet: wait for the next. */
        unsigned int first = uart->actual[index];
        deadline = now_ms() + NEXT_FRAME_WAIT_MS;
        while (uart->actual[index] == first && now_ms() < deadline) {
            ev3_core_sleep(POLL_MS);
        }
    }
    /* The driver moves Actual on as data arrives: read it until it holds still. */
    do {
        actual = uart->actual[index];
        for (int i = 0; i < DATA_LENGTH; i++) {
            data[i] = uart->raw[index][actual % LOG_SIZE][i];
        }
    } while (actual != uart->actual[index]);

    for (int i = 0; i < count; i++) {
        switch (info->format & 0x0F) {
        case DATA_8:
            values[i] = data[i];
            break;
        case DATA_16: {
            int16_t value;
            memcpy(&value, data + 2 * i, sizeof value);
            values[i] = value;
            break;
        }
        case DATA_32:
            memcpy(&values[i], data + 4 * i, sizeof values[i]);
            break;
        case DATA_F: {
            float value;
            memcpy(&value, data + 4 * i, sizeof value);
            values[i] = (int32_t)value;
            break;
        }
        default:
            return ev3_fail(EV3_ERR_DEVICE, "the sensor on port %d describes mode %s with an unknown format", index + 1, mode_name);
        }
    }
    return EV3_OK;
}

/* A raw value in the mode's SI unit, from the ranges the sensor gives. */
static float si(const ev3_types *info, int32_t raw)
{
    if (info->raw_max == info->raw_min) {
        return (float)raw;
    }
    return ((float)raw - info->raw_min) / (info->raw_max - info->raw_min) * (info->si_max - info->si_min) + info->si_min;
}

/* One value of the sensor in a mode. */
static Ev3Result read_one(Ev3Port port, Ev3SensorType wanted, const char *mode, int fallback,
                          int32_t *value, ev3_types *info)
{
    int index;
    Ev3Result result = expect(port, wanted, &index);
    return result == EV3_OK ? uart_values(index, mode, fallback, value, 1, info) : result;
}

static Ev3Result check_out(const void *out)
{
    return out == NULL ? ev3_fail(EV3_ERR_PARAM, "the result points nowhere (NULL)") : EV3_OK;
}

/* --- touch ------------------------------------------------------------------------- */

Ev3Result ev3_sensor_try_read_touch(Ev3Port port, bool *pressed)
{
    int index;
    Ev3Result result = check_out(pressed);

    if (result == EV3_OK) {
        result = expect(port, EV3_SENSOR_TOUCH, &index);
    }
    if (result == EV3_OK) {
        *pressed = analog->in_pin6[index] > TOUCH_THRESHOLD;
    }
    return result;
}

/* --- color ------------------------------------------------------------------------- */

Ev3Result ev3_sensor_try_read_color(Ev3Port port, Ev3Color *color)
{
    int32_t value = 0;
    ev3_types info;
    Ev3Result result = check_out(color);

    if (result == EV3_OK) {
        result = read_one(port, EV3_SENSOR_COLOR, "COL-COLOR", 2, &value, &info);
    }
    if (result == EV3_OK) {
        *color = value >= EV3_COLOR_NONE && value <= EV3_COLOR_BROWN ? (Ev3Color)value : EV3_COLOR_NONE;
    }
    return result;
}

static Ev3Result read_percent(Ev3Port port, const char *mode, int fallback, int *percent)
{
    int32_t value = 0;
    ev3_types info;
    Ev3Result result = check_out(percent);

    if (result == EV3_OK) {
        result = read_one(port, EV3_SENSOR_COLOR, mode, fallback, &value, &info);
    }
    if (result == EV3_OK) {
        *percent = value < 0 ? 0 : value > 100 ? 100 : (int)value;
    }
    return result;
}

Ev3Result ev3_sensor_try_read_reflected(Ev3Port port, int *percent)
{
    return read_percent(port, "COL-REFLECT", 0, percent);
}

Ev3Result ev3_sensor_try_read_ambient(Ev3Port port, int *percent)
{
    return read_percent(port, "COL-AMBIENT", 1, percent);
}

Ev3Result ev3_sensor_try_read_rgb(Ev3Port port, Ev3ColorRgb *rgb)
{
    int32_t values[3] = { 0, 0, 0 };
    ev3_types info;
    int index;
    Ev3Result result = check_out(rgb);

    if (result == EV3_OK) {
        result = expect(port, EV3_SENSOR_COLOR, &index);
    }
    if (result == EV3_OK) {
        result = uart_values(index, "RGB-RAW", 4, values, 3, &info);
    }
    if (result == EV3_OK) {
        rgb->red = (int)values[0];
        rgb->green = (int)values[1];
        rgb->blue = (int)values[2];
    }
    return result;
}

/* --- ultrasonic ---------------------------------------------------------------------- */

static Ev3Result read_distance(Ev3Port port, const char *mode, int fallback, float *distance)
{
    int32_t value = 0;
    ev3_types info;
    Ev3Result result = check_out(distance);

    if (result == EV3_OK) {
        result = read_one(port, EV3_SENSOR_ULTRASONIC, mode, fallback, &value, &info);
    }
    if (result == EV3_OK) {
        *distance = si(&info, value);
    }
    return result;
}

Ev3Result ev3_sensor_try_read_distance_cm(Ev3Port port, float *cm)
{
    return read_distance(port, "US-DIST-CM", 0, cm);
}

Ev3Result ev3_sensor_try_read_distance_in(Ev3Port port, float *inches)
{
    return read_distance(port, "US-DIST-IN", 1, inches);
}

Ev3Result ev3_sensor_try_ultrasonic_hears_other(Ev3Port port, bool *heard)
{
    int32_t value = 0;
    ev3_types info;
    Ev3Result result = check_out(heard);

    if (result == EV3_OK) {
        result = read_one(port, EV3_SENSOR_ULTRASONIC, "US-LISTEN", 2, &value, &info);
    }
    if (result == EV3_OK) {
        *heard = value != 0;
    }
    return result;
}

/* --- gyro -------------------------------------------------------------------------------- */

/*
 * The angle and the rate both come from the gyro's GYRO-G&A mode, which sends
 * the two together, so reading one after the other never changes the mode.
 *
 * The gyro keeps counting its angle from when it was plugged in, across
 * programs, and a change of mode does not reliably start it at 0 again
 * (checked on a brick). So the angle a program reads is counted from the
 * program's first reading, and from each ev3_sensor_gyro_reset() after it.
 */
static int32_t gyro_zero[PORTS];
static bool gyro_zeroed[PORTS];

static Ev3Result read_gyro(Ev3Port port, int32_t values[2], int *index)
{
    ev3_types info;
    Ev3Result result = expect(port, EV3_SENSOR_GYRO, index);

    if (result == EV3_OK) {
        result = uart_values(*index, "GYRO-G&A", 3, values, 2, &info);
    }
    if (result == EV3_OK && !gyro_zeroed[*index]) {
        gyro_zero[*index] = values[0];
        gyro_zeroed[*index] = true;
    }
    return result;
}

Ev3Result ev3_sensor_try_read_gyro_angle(Ev3Port port, int *degrees)
{
    int32_t values[2] = { 0, 0 };
    int index = 0;
    Ev3Result result = check_out(degrees);

    if (result == EV3_OK) {
        result = read_gyro(port, values, &index);
    }
    if (result == EV3_OK) {
        *degrees = (int)(values[0] - gyro_zero[index]);
    }
    return result;
}

Ev3Result ev3_sensor_try_read_gyro_rate(Ev3Port port, int *degrees_per_second)
{
    int32_t values[2] = { 0, 0 };
    int index = 0;
    Ev3Result result = check_out(degrees_per_second);

    if (result == EV3_OK) {
        result = read_gyro(port, values, &index);
    }
    if (result == EV3_OK) {
        *degrees_per_second = (int)values[1];
    }
    return result;
}

Ev3Result ev3_sensor_try_gyro_reset(Ev3Port port)
{
    int32_t values[2] = { 0, 0 };
    int index = 0;
    Ev3Result result = read_gyro(port, values, &index);

    if (result == EV3_OK) {
        gyro_zero[index] = values[0];
    }
    return result;
}

/* --- infrared ------------------------------------------------------------------------------ */

static Ev3Result check_channel(int channel)
{
    if (channel < 1 || channel > 4) {
        return ev3_fail(EV3_ERR_PARAM, "channel %d is not 1 to 4", channel);
    }
    return EV3_OK;
}

Ev3Result ev3_sensor_try_read_ir_proximity(Ev3Port port, int *proximity)
{
    int32_t value = 0;
    ev3_types info;
    Ev3Result result = check_out(proximity);

    if (result == EV3_OK) {
        result = read_one(port, EV3_SENSOR_IR, "IR-PROX", 0, &value, &info);
    }
    if (result == EV3_OK) {
        *proximity = (int)value;
    }
    return result;
}

Ev3Result ev3_sensor_try_read_ir_seek(Ev3Port port, int channel, Ev3IrSeek *seek)
{
    int32_t values[8];
    ev3_types info;
    int index;
    Ev3Result result = check_out(seek);

    if (result == EV3_OK) {
        result = check_channel(channel);
    }
    if (result == EV3_OK) {
        result = expect(port, EV3_SENSOR_IR, &index);
    }
    if (result == EV3_OK) {
        result = uart_values(index, "IR-SEEK", 1, values, 8, &info);
    }
    if (result == EV3_OK) {
        int heading = (int)values[2 * (channel - 1)];
        int distance = (int)values[2 * (channel - 1) + 1];
        /* -128 is the firmware's "no value" for one byte: no beacon on that channel. */
        seek->found = distance != -128;
        seek->heading = seek->found ? heading : 0;
        seek->distance = seek->found ? distance : 0;
    }
    return result;
}

Ev3Result ev3_sensor_try_read_ir_remote(Ev3Port port, int channel, int *buttons)
{
    int32_t values[4];
    ev3_types info;
    int index;
    Ev3Result result = check_out(buttons);

    if (result == EV3_OK) {
        result = check_channel(channel);
    }
    if (result == EV3_OK) {
        result = expect(port, EV3_SENSOR_IR, &index);
    }
    if (result == EV3_OK) {
        result = uart_values(index, "IR-REMOTE", 2, values, 4, &info);
    }
    if (result == EV3_OK) {
        *buttons = (int)values[channel - 1];
    }
    return result;
}

/* --- plain variants ---------------------------------------------------------------------------- */

Ev3SensorType ev3_sensor_get_type_at_(EV3_AT, Ev3Port port)
{
    Ev3SensorType type = EV3_SENSOR_NONE;
    EV3_MUST("ev3_sensor_get_type", ev3_sensor_try_get_type(port, &type));
    return type;
}

bool ev3_sensor_read_touch_at_(EV3_AT, Ev3Port port)
{
    bool pressed = false;
    EV3_MUST("ev3_sensor_read_touch", ev3_sensor_try_read_touch(port, &pressed));
    return pressed;
}

Ev3Color ev3_sensor_read_color_at_(EV3_AT, Ev3Port port)
{
    Ev3Color color = EV3_COLOR_NONE;
    EV3_MUST("ev3_sensor_read_color", ev3_sensor_try_read_color(port, &color));
    return color;
}

int ev3_sensor_read_reflected_at_(EV3_AT, Ev3Port port)
{
    int percent = 0;
    EV3_MUST("ev3_sensor_read_reflected", ev3_sensor_try_read_reflected(port, &percent));
    return percent;
}

int ev3_sensor_read_ambient_at_(EV3_AT, Ev3Port port)
{
    int percent = 0;
    EV3_MUST("ev3_sensor_read_ambient", ev3_sensor_try_read_ambient(port, &percent));
    return percent;
}

Ev3ColorRgb ev3_sensor_read_rgb_at_(EV3_AT, Ev3Port port)
{
    Ev3ColorRgb rgb = { 0, 0, 0 };
    EV3_MUST("ev3_sensor_read_rgb", ev3_sensor_try_read_rgb(port, &rgb));
    return rgb;
}

float ev3_sensor_read_distance_cm_at_(EV3_AT, Ev3Port port)
{
    float cm = 0.0f;
    EV3_MUST("ev3_sensor_read_distance_cm", ev3_sensor_try_read_distance_cm(port, &cm));
    return cm;
}

float ev3_sensor_read_distance_in_at_(EV3_AT, Ev3Port port)
{
    float inches = 0.0f;
    EV3_MUST("ev3_sensor_read_distance_in", ev3_sensor_try_read_distance_in(port, &inches));
    return inches;
}

bool ev3_sensor_ultrasonic_hears_other_at_(EV3_AT, Ev3Port port)
{
    bool heard = false;
    EV3_MUST("ev3_sensor_ultrasonic_hears_other", ev3_sensor_try_ultrasonic_hears_other(port, &heard));
    return heard;
}

int ev3_sensor_read_gyro_angle_at_(EV3_AT, Ev3Port port)
{
    int degrees = 0;
    EV3_MUST("ev3_sensor_read_gyro_angle", ev3_sensor_try_read_gyro_angle(port, &degrees));
    return degrees;
}

int ev3_sensor_read_gyro_rate_at_(EV3_AT, Ev3Port port)
{
    int rate = 0;
    EV3_MUST("ev3_sensor_read_gyro_rate", ev3_sensor_try_read_gyro_rate(port, &rate));
    return rate;
}

void ev3_sensor_gyro_reset_at_(EV3_AT, Ev3Port port)
{
    EV3_MUST("ev3_sensor_gyro_reset", ev3_sensor_try_gyro_reset(port));
}

int ev3_sensor_read_ir_proximity_at_(EV3_AT, Ev3Port port)
{
    int proximity = 0;
    EV3_MUST("ev3_sensor_read_ir_proximity", ev3_sensor_try_read_ir_proximity(port, &proximity));
    return proximity;
}

Ev3IrSeek ev3_sensor_read_ir_seek_at_(EV3_AT, Ev3Port port, int channel)
{
    Ev3IrSeek seek = { false, 0, 0 };
    EV3_MUST("ev3_sensor_read_ir_seek", ev3_sensor_try_read_ir_seek(port, channel, &seek));
    return seek;
}

int ev3_sensor_read_ir_remote_at_(EV3_AT, Ev3Port port, int channel)
{
    int buttons = 0;
    EV3_MUST("ev3_sensor_read_ir_remote", ev3_sensor_try_read_ir_remote(port, channel, &buttons));
    return buttons;
}
