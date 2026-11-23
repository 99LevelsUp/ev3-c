/* Shared between the ev3lib core sources; not part of the public API. */
#ifndef EV3_CORE_INTERNAL_H
#define EV3_CORE_INTERNAL_H

#include <stddef.h>

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

/*
 * Writes one command to the motor device. The driver handles one command
 * per write(), so callers never join commands. Returns 0 or -1.
 */
int ev3_core_pwm_write(const unsigned char *command, size_t length);

/* Paths of the devices, from the environment or the brick's defaults. */
const char *ev3_core_pwm_device(void);
const char *ev3_core_motor_device(void);
const char *ev3_core_ui_device(void);
const char *ev3_core_sound_device(void);

/* Writes `length` bytes to a device in one write(). Returns 0 or -1. */
int ev3_core_device_write(const char *device, const void *data, size_t length);

#endif /* EV3_CORE_INTERNAL_H */
