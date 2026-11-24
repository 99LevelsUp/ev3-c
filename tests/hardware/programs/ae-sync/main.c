/*
 * Acceptance: two motors together and ramps, with a motor on A and one on B
 * turning freely. Nothing to do but watch: each step resets both positions,
 * runs, and prints where both motors ended next to what they should read.
 * Small differences of a few degrees are the motors coasting into the brake.
 */
#include <stdio.h>

#include "ev3.h"

static void reset(void)
{
    ev3_motor_reset_position(EV3_PORT_A | EV3_PORT_B);
    ev3_sleep_ms(300);
}

static void report(const char *what, const char *expected)
{
    ev3_sleep_ms(300);
    printf("%-34s A=%5d B=%5d   expected %s\n", what, ev3_motor_get_position(EV3_PORT_A),
           ev3_motor_get_position(EV3_PORT_B), expected);
}

static void sync_step(const char *what, Ev3Port left, Ev3Port right, int turn, int degrees, const char *expected)
{
    reset();
    ev3_motor_sync_run_degrees(left, right, 40, turn, degrees);
    report(what, expected);
}

int main(void)
{
    ev3_sound_play_tone(880, 150);

    sync_step("A,B turn 0, 360", EV3_PORT_A, EV3_PORT_B, 0, 360, "A=360 B=360");
    sync_step("A,B turn 50, 360", EV3_PORT_A, EV3_PORT_B, 50, 360, "A=360 B=180");
    sync_step("A,B turn 100, 360", EV3_PORT_A, EV3_PORT_B, 100, 360, "A=360 B=0");
    sync_step("A,B turn 150, 360", EV3_PORT_A, EV3_PORT_B, 150, 360, "A=360 B=-180");
    sync_step("A,B turn 200, 360", EV3_PORT_A, EV3_PORT_B, 200, 360, "A=360 B=-360");
    sync_step("A,B turn -50, 360", EV3_PORT_A, EV3_PORT_B, -50, 360, "A=180 B=360");
    sync_step("B,A turn 50, 360 (B is left)", EV3_PORT_B, EV3_PORT_A, 50, 360, "A=180 B=360");
    sync_step("A,B turn 0, -360", EV3_PORT_A, EV3_PORT_B, 0, -360, "A=-360 B=-360");

    reset();
    ev3_motor_sync_run_seconds(EV3_PORT_A, EV3_PORT_B, 40, 0, 1.0f);
    report("A,B turn 0, 1 s", "A=B");

    reset();
    ev3_motor_sync_start(EV3_PORT_A, EV3_PORT_B, 30, 0);
    ev3_sleep_ms(1000);
    ev3_motor_sync_stop(EV3_PORT_A, EV3_PORT_B, EV3_STOP_BRAKE);
    report("A,B start, 1 s, stop", "A=B");

    reset();
    ev3_motor_run_degrees_ramp(EV3_PORT_A, 60, 90, 180, 90, EV3_STOP_BRAKE);
    report("A ramp 90+180+90", "A=360 B=0");

    reset();
    ev3_motor_run_seconds_ramp(EV3_PORT_B, -50, 0.5f, 1.0f, 0.5f, EV3_STOP_BRAKE);
    report("B ramp -50, 0.5+1+0.5 s", "A=0 B<0");

    ev3_sound_play_tone(1320, 300);
    printf("done\n");
    return 0;
}
