/*
 * ev3lib motors: commands for the firmware's PWM device (/dev/lms_pwm). Each
 * command is the opcode followed by a port mask (bit 0 = port A) and its
 * argument, as the firmware's own output module writes them (d_pwm.c of the
 * LEGO firmware sources).
 */
#include "ev3.h"
#include "ev3_core_internal.h"

int ev3_motor_set_power(int ports, int power)
{
    unsigned char set[3];
    unsigned char start[2];

    /* A power outside -100..100 would wrap around in the command byte and could reverse the motor. */
    if (power > 100) {
        power = 100;
    } else if (power < -100) {
        power = -100;
    }
    set[0] = EV3_OP_OUTPUT_POWER;
    set[1] = (unsigned char)ports;
    set[2] = (unsigned char)power;
    start[0] = EV3_OP_OUTPUT_START;
    start[1] = (unsigned char)ports;
    if (ev3_core_pwm_write(set, sizeof set) != 0) {
        return -1;
    }
    return ev3_core_pwm_write(start, sizeof start);
}

int ev3_motor_stop(int ports, int brake)
{
    unsigned char stop[3];

    stop[0] = EV3_OP_OUTPUT_STOP;
    stop[1] = (unsigned char)ports;
    stop[2] = brake ? 1 : 0;
    return ev3_core_pwm_write(stop, sizeof stop);
}
