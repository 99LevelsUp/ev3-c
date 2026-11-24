/*
 * ev3lib core: start-up and exit, the environment the supervisor passes, the
 * device paths and writer shared by the other modules, and sleep.
 */
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>
#include <unistd.h>

#include "ev3.h"
#include "ev3_core_internal.h"

static int initialized;
static const char *pwm_device = "/dev/lms_pwm";
static const char *motor_device = "/dev/lms_motor";
static const char *ui_device = "/dev/lms_ui";
static const char *sound_device = "/dev/lms_sound";
static const char *analog_device = "/dev/lms_analog";
static const char *uart_device = "/dev/lms_uart";
static const char *iic_device = "/dev/lms_iic";
static const char *project_dir;
static const char *run_dir;

static const char *env_or(const char *name, const char *fallback)
{
    const char *value = getenv(name);
    return value != NULL && value[0] != '\0' ? value : fallback;
}

void ev3_core_init(void)
{
    if (initialized) {
        return;
    }
    initialized = 1;

    /*
     * The supervisor sends stdout and stderr to files in the run folder.
     * Written to a file, stdout is fully buffered by default, so a crash or a
     * stop would lose every line still in the buffer. Line buffering writes
     * each line as it is finished. stderr is made unbuffered, which is what C
     * libraries already default to: every call reaches the file, even a line
     * the program has not finished when it crashes.
     */
    setvbuf(stdout, NULL, _IOLBF, 0);
    setvbuf(stderr, NULL, _IONBF, 0);

    pwm_device = env_or("EV3C_PWM_DEVICE", "/dev/lms_pwm");
    motor_device = env_or("EV3C_MOTOR_DEVICE", "/dev/lms_motor");
    ui_device = env_or("EV3C_UI_DEVICE", "/dev/lms_ui");
    sound_device = env_or("EV3C_SOUND_DEVICE", "/dev/lms_sound");
    analog_device = env_or("EV3C_ANALOG_DEVICE", "/dev/lms_analog");
    uart_device = env_or("EV3C_UART_DEVICE", "/dev/lms_uart");
    iic_device = env_or("EV3C_IIC_DEVICE", "/dev/lms_iic");
    project_dir = env_or("EV3C_PROJECT_DIR", NULL);
    run_dir = env_or("EV3C_RUN_DIR", NULL);

    /* Runs on return from main() and on exit(), not when a signal kills the program. */
    atexit(ev3_core_exit);
}

/* Runs before main(), so the program's first output is already line buffered. */
__attribute__((constructor)) static void ev3_core_start(void)
{
    ev3_core_init();
}

void ev3_core_exit(void)
{
    /*
     * opPROGRAM_STOP is what the firmware itself writes to the motor device
     * when a program ends (cOutputPrgStop in c_output.c of the LEGO firmware
     * sources): it stops every motor.
     */
    static const unsigned char stop_all[] = { EV3_OP_PROGRAM_STOP };
    (void)ev3_core_pwm_write(stop_all, sizeof stop_all);
}

const char *ev3_core_project_dir(void)
{
    return project_dir;
}

const char *ev3_core_run_dir(void)
{
    return run_dir;
}

const char *ev3_core_pwm_device(void)
{
    return pwm_device;
}

const char *ev3_core_motor_device(void)
{
    return motor_device;
}

const char *ev3_core_ui_device(void)
{
    return ui_device;
}

const char *ev3_core_sound_device(void)
{
    return sound_device;
}

const char *ev3_core_analog_device(void)
{
    return analog_device;
}

const char *ev3_core_uart_device(void)
{
    return uart_device;
}

const char *ev3_core_iic_device(void)
{
    return iic_device;
}

int ev3_core_pwm_write(const unsigned char *command, size_t length)
{
    return ev3_core_device_write(pwm_device, command, length);
}

int ev3_core_device_write(const char *device, const void *data, size_t length)
{
    /*
     * Opened per command, as the firmware's drivers take one command per
     * write(). O_APPEND changes nothing on a device and keeps every command
     * in order in a fake device file during host tests.
     */
    int fd = open(device, O_WRONLY | O_APPEND);
    ssize_t written;
    int saved;

    if (fd < 0) {
        return -1;
    }
    written = write(fd, data, length);
    saved = errno;
    close(fd);
    if (written != (ssize_t)length) {
        errno = written < 0 ? saved : EIO;
        return -1;
    }
    return 0;
}

void ev3_core_sleep(unsigned int ms)
{
    struct timespec left;

    left.tv_sec = (time_t)(ms / 1000);
    left.tv_nsec = (long)(ms % 1000) * 1000000L;
    while (nanosleep(&left, &left) != 0 && errno == EINTR) {
        /* A signal woke us early: sleep for the rest. */
    }
}

Ev3Result ev3_try_sleep_ms(int ms)
{
    if (ms < 0) {
        return ev3_fail(EV3_ERR_PARAM, "%d ms is negative", ms);
    }
    ev3_core_sleep((unsigned int)ms);
    return EV3_OK;
}

Ev3Result ev3_try_sleep_sec(float seconds)
{
    if (!(seconds >= 0.0f) || seconds > 2000000.0f) {
        return ev3_fail(EV3_ERR_PARAM, "%g seconds is not a time from 0 to 2000000", (double)seconds);
    }
    ev3_core_sleep((unsigned int)(seconds * 1000.0f + 0.5f));
    return EV3_OK;
}

void ev3_sleep_ms_at_(EV3_AT, int ms)
{
    EV3_MUST("ev3_sleep_ms", ev3_try_sleep_ms(ms));
}

void ev3_sleep_sec_at_(EV3_AT, float seconds)
{
    EV3_MUST("ev3_sleep_sec", ev3_try_sleep_sec(seconds));
}
