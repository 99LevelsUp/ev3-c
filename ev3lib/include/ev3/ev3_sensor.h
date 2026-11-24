/*
 * ev3lib sensors on ports 1 to 4: the EV3 touch, color, ultrasonic, gyro and
 * infrared sensors.
 *
 * Each reading switches the sensor to the mode it needs, so a program just
 * reads what it wants; switching takes a moment the first time.
 *
 * Each function comes as a try variant, which returns an Ev3Result, and as a
 * plain variant, which stops the program on an error (see ev3_error.h): for
 * example when nothing, or another sensor, is plugged into the port.
 */
#ifndef EV3_SENSOR_H
#define EV3_SENSOR_H

#include <stdbool.h>

#include "ev3_error.h"

#ifdef __cplusplus
extern "C" {
#endif

/* --- what is plugged in -------------------------------------------------------- */

typedef enum {
    EV3_SENSOR_NONE       = 0, /* nothing plugged in */
    EV3_SENSOR_TOUCH      = 1,
    EV3_SENSOR_COLOR      = 2,
    EV3_SENSOR_ULTRASONIC = 3,
    EV3_SENSOR_GYRO       = 4,
    EV3_SENSOR_IR         = 5,
    EV3_SENSOR_OTHER      = 6  /* something ev3lib does not read yet, such as an NXT sensor */
} Ev3SensorType;

/* What is plugged into the port. */
Ev3Result ev3_sensor_try_get_type(Ev3Port port, Ev3SensorType *type);
Ev3SensorType ev3_sensor_get_type_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_get_type(port) ev3_sensor_get_type_at_(__FILE__, __LINE__, (port))

/* --- touch ---------------------------------------------------------------------- */

/* Whether the touch sensor is pressed. */
Ev3Result ev3_sensor_try_read_touch(Ev3Port port, bool *pressed);
bool ev3_sensor_read_touch_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_read_touch(port) ev3_sensor_read_touch_at_(__FILE__, __LINE__, (port))

/* --- color ---------------------------------------------------------------------- */

typedef enum {
    EV3_COLOR_NONE   = 0,
    EV3_COLOR_BLACK  = 1,
    EV3_COLOR_BLUE   = 2,
    EV3_COLOR_GREEN  = 3,
    EV3_COLOR_YELLOW = 4,
    EV3_COLOR_RED    = 5,
    EV3_COLOR_WHITE  = 6,
    EV3_COLOR_BROWN  = 7
} Ev3Color;

/* The color in front of the sensor. */
Ev3Result ev3_sensor_try_read_color(Ev3Port port, Ev3Color *color);
Ev3Color ev3_sensor_read_color_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_read_color(port) ev3_sensor_read_color_at_(__FILE__, __LINE__, (port))

/* Reflected light of the sensor's red lamp, 0 (dark) to 100 (bright). */
Ev3Result ev3_sensor_try_read_reflected(Ev3Port port, int *percent);
int ev3_sensor_read_reflected_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_read_reflected(port) ev3_sensor_read_reflected_at_(__FILE__, __LINE__, (port))

/* Light around the sensor, 0 (dark) to 100 (bright). */
Ev3Result ev3_sensor_try_read_ambient(Ev3Port port, int *percent);
int ev3_sensor_read_ambient_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_read_ambient(port) ev3_sensor_read_ambient_at_(__FILE__, __LINE__, (port))

/* Raw red, green and blue of reflected white light, each about 0 to 1023. */
typedef struct {
    int red;
    int green;
    int blue;
} Ev3ColorRgb;

Ev3Result ev3_sensor_try_read_rgb(Ev3Port port, Ev3ColorRgb *rgb);
Ev3ColorRgb ev3_sensor_read_rgb_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_read_rgb(port) ev3_sensor_read_rgb_at_(__FILE__, __LINE__, (port))

/* --- ultrasonic ------------------------------------------------------------------- */

/* The distance to what is in front, in centimetres (0 to 255; 255 when nothing is found). */
Ev3Result ev3_sensor_try_read_distance_cm(Ev3Port port, float *cm);
float ev3_sensor_read_distance_cm_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_read_distance_cm(port) ev3_sensor_read_distance_cm_at_(__FILE__, __LINE__, (port))

/* The distance in inches (0 to 100). */
Ev3Result ev3_sensor_try_read_distance_in(Ev3Port port, float *inches);
float ev3_sensor_read_distance_in_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_read_distance_in(port) ev3_sensor_read_distance_in_at_(__FILE__, __LINE__, (port))

/* Whether another ultrasonic sensor is sending nearby. */
Ev3Result ev3_sensor_try_ultrasonic_hears_other(Ev3Port port, bool *heard);
bool ev3_sensor_ultrasonic_hears_other_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_ultrasonic_hears_other(port) \
    ev3_sensor_ultrasonic_hears_other_at_(__FILE__, __LINE__, (port))

/* --- gyro --------------------------------------------------------------------------- */

/*
 * Before a program starts working with the gyro, it should reset it with
 * ev3_sensor_gyro_reset(), with the robot standing still:
 *
 *     ev3_sensor_gyro_reset(EV3_PORT_4);
 *     ev3_motor_start(EV3_PORT_B | EV3_PORT_C, 30);
 *     while (ev3_sensor_read_gyro_angle(EV3_PORT_4) < 90) { ... }
 *
 * The gyro counts its angle from when it was plugged in, across programs, so
 * without a reset the angle would start wherever an earlier program left it.
 * ev3lib starts it at 0 at the program's first reading anyway, but a reset
 * says where 0 is, and can be repeated, for example before each turn.
 */

/*
 * The angle turned, in degrees, clockwise positive: since the program first
 * read the gyro, or since ev3_sensor_gyro_reset().
 */
Ev3Result ev3_sensor_try_read_gyro_angle(Ev3Port port, int *degrees);
int ev3_sensor_read_gyro_angle_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_read_gyro_angle(port) ev3_sensor_read_gyro_angle_at_(__FILE__, __LINE__, (port))

/* How fast the sensor turns now, in degrees per second. */
Ev3Result ev3_sensor_try_read_gyro_rate(Ev3Port port, int *degrees_per_second);
int ev3_sensor_read_gyro_rate_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_read_gyro_rate(port) ev3_sensor_read_gyro_rate_at_(__FILE__, __LINE__, (port))

/* Counts the gyro's angle from 0 again, from where it points now. */
Ev3Result ev3_sensor_try_gyro_reset(Ev3Port port);
void ev3_sensor_gyro_reset_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_gyro_reset(port) ev3_sensor_gyro_reset_at_(__FILE__, __LINE__, (port))

/* --- infrared ------------------------------------------------------------------------- */

/* How near something is, 0 (close) to 100 (far). */
Ev3Result ev3_sensor_try_read_ir_proximity(Ev3Port port, int *proximity);
int ev3_sensor_read_ir_proximity_at_(const char *file, int line, Ev3Port port);
#define ev3_sensor_read_ir_proximity(port) ev3_sensor_read_ir_proximity_at_(__FILE__, __LINE__, (port))

/* Where the beacon on a channel (1 to 4) is. */
typedef struct {
    bool found;    /* whether the beacon was seen; heading and distance are 0 if not */
    int heading;   /* -25 (far left) to 25 (far right) */
    int distance;  /* 0 (close) to 100 (far) */
} Ev3IrSeek;

Ev3Result ev3_sensor_try_read_ir_seek(Ev3Port port, int channel, Ev3IrSeek *seek);
Ev3IrSeek ev3_sensor_read_ir_seek_at_(const char *file, int line, Ev3Port port, int channel);
#define ev3_sensor_read_ir_seek(port, channel) \
    ev3_sensor_read_ir_seek_at_(__FILE__, __LINE__, (port), (channel))

/* The buttons pressed on the remote control set to a channel (1 to 4). */
#define EV3_IR_NONE                0
#define EV3_IR_RED_UP              1
#define EV3_IR_RED_DOWN            2
#define EV3_IR_BLUE_UP             3
#define EV3_IR_BLUE_DOWN           4
#define EV3_IR_RED_UP_BLUE_UP      5
#define EV3_IR_RED_UP_BLUE_DOWN    6
#define EV3_IR_RED_DOWN_BLUE_UP    7
#define EV3_IR_RED_DOWN_BLUE_DOWN  8
#define EV3_IR_BEACON              9
#define EV3_IR_RED_UP_RED_DOWN     10
#define EV3_IR_BLUE_UP_BLUE_DOWN   11

Ev3Result ev3_sensor_try_read_ir_remote(Ev3Port port, int channel, int *buttons);
int ev3_sensor_read_ir_remote_at_(const char *file, int line, Ev3Port port, int channel);
#define ev3_sensor_read_ir_remote(port, channel) \
    ev3_sensor_read_ir_remote_at_(__FILE__, __LINE__, (port), (channel))

#ifdef __cplusplus
}
#endif

#endif /* EV3_SENSOR_H */
