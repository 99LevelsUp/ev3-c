/* Shared between the ev3lib core sources; not part of the public API. */
#ifndef EV3_CORE_INTERNAL_H
#define EV3_CORE_INTERNAL_H

#include <stddef.h>

/* Firmware PWM device commands, from lms2012/source/bytecodes.h. */
#define EV3_OP_PROGRAM_STOP 0x02
#define EV3_OP_OUTPUT_STOP  0xA3
#define EV3_OP_OUTPUT_POWER 0xA4
#define EV3_OP_OUTPUT_START 0xA6

/*
 * Writes one command to the motor device. The driver handles one command
 * per write(), so callers never join commands. Returns 0 or -1.
 */
int ev3_core_pwm_write(const unsigned char *command, size_t length);

/* Path of the button device. */
const char *ev3_core_ui_device(void);

#endif /* EV3_CORE_INTERNAL_H */
