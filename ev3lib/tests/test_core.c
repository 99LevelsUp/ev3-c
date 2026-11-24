/*
 * Host tests for the ev3lib core: output buffering, motor commands, the stop
 * on exit, the Back takeover request, the Back button state, the list of
 * files a program returns, regulated and measured motor commands, the motor
 * state, every button, the LEDs, and tones.
 *
 * Each scenario runs in a fresh process: the test executes itself with
 * "--scenario <name>" and an environment that points ev3lib at fake device
 * files in a temporary folder. That way ev3lib's start-up code sees the
 * prepared environment exactly as a program on the brick sees the one the
 * supervisor sets, and a return from main() runs the real exit path.
 *
 * Needs POSIX (fork, exec, mmap); run it on Linux or macOS.
 */
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "ev3.h"

static const char *self;
static char root[256];
static char pwm_path[300];
static char ui_path[300];
static char motor_path[300];
static char sound_path[300];
static char project_path[300];
static char bak_path[320];
static char out_path[300];
static char err_path[300];
static char analog_path[300];
static char uart_path[300];
static char lcd_path[300];
static char lcd_marker[300];
static char ui_library_path[300];
static char run_path[300];
static char ret_path[320];
static char returned_path[320];
static int failures;

#define CHECK(cond, ...)                                                   \
    do {                                                                   \
        if (!(cond)) {                                                     \
            failures++;                                                    \
            fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__);           \
            fprintf(stderr, __VA_ARGS__);                                  \
            fputc('\n', stderr);                                           \
        }                                                                  \
    } while (0)

/* --- scenarios, run in the child process --------------------------------- */

static int scenario(const char *name)
{
    if (strcmp(name, "abort") == 0) {
        /* Two lines: musl writes the first line of a file-backed stdout at once
           and only then switches to full buffering, so a single line would
           pass even without ev3lib. */
        printf("first line\n");
        printf("second line\n");
        abort();
    }
    if (strcmp(name, "motor") == 0) {
        ev3_motor_set_power(EV3_PORT_A | EV3_PORT_B, 50);
        ev3_motor_set_power(EV3_PORT_D, -100);
        ev3_motor_stop(EV3_PORT_A, EV3_STOP_BRAKE);
        return 0;
    }
    if (strcmp(name, "idle") == 0) {
        return 0;
    }
    if (strcmp(name, "take-back") == 0) {
        ev3_button_take_back();
        return 0;
    }
    if (strcmp(name, "release-back") == 0) {
        ev3_button_release_back();
        return 0;
    }
    if (strcmp(name, "motor-more") == 0) {
        ev3_motor_start(EV3_PORT_A | EV3_PORT_B, 40);
        ev3_motor_set_polarity(EV3_PORT_C, -1);
        ev3_motor_start_degrees(EV3_PORT_A, 30, 360);
        ev3_motor_start_seconds(EV3_PORT_B, -20, 1.5f);
        ev3_motor_start_degrees(EV3_PORT_C, 25, -90);
        ev3_motor_reset_position(EV3_PORT_A | EV3_PORT_D);
        return 0;
    }
    if (strcmp(name, "motor-sync") == 0) {
        ev3_motor_sync_start(EV3_PORT_B, EV3_PORT_C, 50, 0);
        ev3_motor_sync_start_degrees(EV3_PORT_C, EV3_PORT_B, 40, 30, -720);
        ev3_motor_sync_start_seconds(EV3_PORT_A, EV3_PORT_D, 20, 200, 1.5f);
        ev3_motor_start_degrees_ramp(EV3_PORT_A, 60, 90, 360, 45, EV3_STOP_COAST);
        ev3_motor_start_seconds_ramp(EV3_PORT_D, -30, 0.5f, 1.0f, 0.25f, EV3_STOP_BRAKE);
        ev3_motor_sync_stop(EV3_PORT_B, EV3_PORT_C, EV3_STOP_BRAKE);
        printf("%d %d %d %d\n", ev3_motor_try_sync_start(EV3_PORT_B, EV3_PORT_B, 50, 0) == EV3_ERR_PARAM,
               ev3_motor_try_sync_start(EV3_PORT_B, EV3_PORT_C, 50, 201) == EV3_ERR_PARAM,
               ev3_motor_try_sync_start(EV3_PORT_A | EV3_PORT_B, EV3_PORT_C, 50, 0) == EV3_ERR_PORT,
               ev3_motor_try_start_degrees_ramp(EV3_PORT_A, 50, -10, 100, 10, EV3_STOP_BRAKE) == EV3_ERR_PARAM);
        return 0;
    }
    if (strcmp(name, "motor-busy") == 0) {
        printf("%d %d %d %d %d\n", ev3_motor_is_busy(EV3_PORT_B), ev3_motor_is_busy(EV3_PORT_A),
               ev3_motor_wait(EV3_PORT_A, 0), ev3_motor_wait(EV3_PORT_B, 30),
               ev3_motor_try_wait(EV3_PORT_B, 30) == EV3_ERR_TIMEOUT);
        return 0;
    }
    if (strcmp(name, "motor-read") == 0) {
        int degrees = 0;
        printf("%d %d %d\n", ev3_motor_get_speed(EV3_PORT_C), ev3_motor_get_position(EV3_PORT_C),
               ev3_motor_try_get_position(EV3_PORT_A | EV3_PORT_B, &degrees) == EV3_ERR_PORT);
        return 0;
    }
    if (strcmp(name, "buttons") == 0) {
        bool pressed = false;
        printf("%d %d %d %d %d\n", ev3_button_is_pressed(EV3_BUTTON_UP), ev3_button_is_pressed(EV3_BUTTON_ENTER),
               ev3_button_is_pressed(EV3_BUTTON_LEFT), ev3_button_is_pressed(EV3_BUTTON_ANY),
               ev3_button_try_is_pressed((Ev3Button)0x40, &pressed) == EV3_ERR_PARAM);
        return 0;
    }
    if (strcmp(name, "led") == 0) {
        ev3_led_set(EV3_LED_RED_FLASH);
        return ev3_led_try_set((Ev3LedPattern)10) == EV3_ERR_PARAM ? 0 : 1;
    }
    if (strcmp(name, "sound") == 0) {
        ev3_sound_play_tone_volume(440, 200, 70);
        ev3_sound_play_tone(1000, 0);
        ev3_sound_stop();
        return ev3_sound_try_play_tone(70000, 10) == EV3_ERR_PARAM ? 0 : 1;
    }
    if (strcmp(name, "plain-error") == 0) {
        /* A sensor port given to a motor: the plain variant names the line and stops. */
        ev3_motor_start(EV3_PORT_1, 50);
        printf("not reached\n");
        return 0;
    }
    if (strcmp(name, "try-error") == 0) {
        Ev3Result result = ev3_motor_try_start(EV3_PORT_A, 150);
        printf("%s|%s\n", ev3_error_string(result), ev3_error_message());
        result = ev3_motor_try_stop(EV3_PORT_A | EV3_PORT_2, EV3_STOP_COAST);
        printf("%s|%s\n", ev3_error_string(result), ev3_error_message());
        return 0;
    }
    if (strcmp(name, "run-blocking") == 0) {
        /* The fake motor device reports nothing busy, so the run returns after its command. */
        ev3_motor_run_degrees(EV3_PORT_B, 50, 180);
        ev3_motor_run_rotations(EV3_PORT_B, -50, 0.5f);
        ev3_motor_run_seconds(EV3_PORT_B, 10, 0.25f);
        return 0;
    }
    if (strcmp(name, "return-files") == 0) {
        /* One file opened through ev3lib, one created otherwise and marked after. */
        const char *project = getenv("EV3C_PROJECT_DIR");
        char other[400];
        FILE *f = ev3_return_fopen(getenv("EV3C_TEST_RETURNED"), "w");
        if (f == NULL || fputs("1,2\n", f) < 0 || fclose(f) != 0) {
            return 1;
        }
        snprintf(other, sizeof other, "%s/other.txt", project);
        f = fopen(other, "w");
        if (f == NULL || fclose(f) != 0) {
            return 2;
        }
        ev3_return_file(other);
        /* A relative path is taken from the current folder and recorded in full. */
        if (chdir(project) != 0 || (f = ev3_return_fopen("relative.txt", "w")) == NULL || fclose(f) != 0) {
            return 3;
        }
        return 0;
    }
    if (strcmp(name, "return-refused") == 0) {
        char missing[400];
        FILE *f = NULL;
        snprintf(missing, sizeof missing, "%s/missing.txt", getenv("EV3C_PROJECT_DIR"));
        if (ev3_return_try_file(missing) != EV3_ERR_IO) {
            return 1;
        }
        if (ev3_return_try_fopen("bad\nname.txt", "w", &f) != EV3_ERR_PARAM || f != NULL
            || access("bad\nname.txt", F_OK) == 0) {
            return 2;
        }
        return 0;
    }
    if (strcmp(name, "return-without-run") == 0) {
        /* Started outside a run: the file opens, and nothing is recorded. */
        FILE *f = ev3_return_fopen(getenv("EV3C_TEST_RETURNED"), "w");
        return f != NULL && fclose(f) == 0 ? 0 : 1;
    }
    if (strcmp(name, "back-state") == 0) {
        printf("%d\n", ev3_button_is_pressed(EV3_BUTTON_BACK));
        return 0;
    }
    if (strcmp(name, "lcd") == 0) {
        ev3_lcd_set_auto_update(false);
        ev3_lcd_fill_rect(0, 0, 3, 1);
        ev3_lcd_draw_pixel(4, 0, true);
        ev3_lcd_draw_text(0, 10, "!");
        ev3_lcd_set_font(EV3_FONT_TINY);
        ev3_lcd_draw_text(30, 20, "\xC4\x8D"); /* a letter the fonts lack: one '?' */
        ev3_lcd_update();
        printf("%d %d %d\n", ev3_lcd_try_set_font((Ev3Font)4) == EV3_ERR_PARAM,
               ev3_lcd_try_fill_rect(0, 0, -1, 2) == EV3_ERR_PARAM, ev3_lcd_try_set_cursor(178, 0) == EV3_ERR_PARAM);
        return 0;
    }
    if (strcmp(name, "lcd-shapes") == 0) {
        /* A drawing that never ends is killed by the alarm and fails the test. */
        alarm(5);
        ev3_lcd_set_auto_update(false);
        ev3_lcd_draw_line(0, 0, EV3_LCD_WIDTH - 1, EV3_LCD_HEIGHT - 1); /* shallow */
        ev3_lcd_draw_line(170, 120, 160, 0);                          /* steep, backwards */
        ev3_lcd_draw_line(5, 100, 50, 100);                           /* flat */
        ev3_lcd_draw_circle(45, 64, 30);
        ev3_lcd_fill_circle(130, 40, 18);
        ev3_lcd_draw_rect(100, 2, 20, 20);
        ev3_lcd_invert_rect(90, 90, 80, 10);
        ev3_lcd_update();
        return 0;
    }
    if (strcmp(name, "lcd-scroll") == 0) {
        /* 20 lines of '!' in the normal font (9 rows) scroll the display several times. */
        alarm(5);
        ev3_lcd_set_auto_update(false);
        for (int i = 0; i < 20; i++) {
            ev3_lcd_println("!");
        }
        ev3_lcd_update();
        return 0;
    }
    if (strcmp(name, "lcd-no-fonts") == 0) {
        Ev3Result result = ev3_lcd_try_draw_text(0, 0, "x");
        printf("%s|%s\n", ev3_error_string(result), ev3_error_message());
        return 0;
    }
    if (strcmp(name, "sensors") == 0) {
        Ev3ColorRgb rgb = ev3_sensor_read_rgb(EV3_PORT_2);
        int angle = 0;
        printf("%d %d %d %d\n", ev3_sensor_get_type(EV3_PORT_1), ev3_sensor_get_type(EV3_PORT_2),
               ev3_sensor_get_type(EV3_PORT_3), ev3_sensor_get_type(EV3_PORT_4));
        printf("%d %d %.1f %d/%d/%d\n", ev3_sensor_read_touch(EV3_PORT_1), ev3_sensor_read_reflected(EV3_PORT_2),
               (double)ev3_sensor_read_distance_cm(EV3_PORT_3), rgb.red, rgb.green, rgb.blue);
        printf("%s|%s\n", ev3_error_string(ev3_sensor_try_read_gyro_angle(EV3_PORT_4, &angle)), ev3_error_message());
        printf("%s|%s\n", ev3_error_string(ev3_sensor_try_read_gyro_angle(EV3_PORT_2, &angle)), ev3_error_message());
        printf("%s|%s\n", ev3_error_string(ev3_sensor_try_read_gyro_angle(EV3_PORT_B, &angle)), ev3_error_message());
        return 0;
    }
    fprintf(stderr, "unknown scenario %s\n", name);
    return 2;
}

/* --- helpers, run in the test process ------------------------------------ */

/*
 * Runs a scenario in a child process with stdout sent to out_path; returns its
 * wait status. With in_run nonzero the child gets EV3C_RUN_DIR, as a program
 * the supervisor starts does.
 */
static int run_scenario_in(const char *name, int in_run)
{
    char pwm_env[320], ui_env[320], motor_env[320], sound_env[320], project_env[320], run_env[320], returned_env[340];
    char analog_env[320], uart_env[320], lcd_env[320], ui_library_env[320];
    char *argv[] = { (char *)self, "--scenario", (char *)name, NULL };
    char *envp[] = { pwm_env, ui_env, motor_env, sound_env, project_env, returned_env, analog_env, uart_env, lcd_env, ui_library_env,
                     in_run ? run_env : NULL, NULL };
    int status = -1;
    pid_t child;

    snprintf(pwm_env, sizeof pwm_env, "EV3C_PWM_DEVICE=%s", pwm_path);
    snprintf(ui_env, sizeof ui_env, "EV3C_UI_DEVICE=%s", ui_path);
    snprintf(motor_env, sizeof motor_env, "EV3C_MOTOR_DEVICE=%s", motor_path);
    snprintf(sound_env, sizeof sound_env, "EV3C_SOUND_DEVICE=%s", sound_path);
    snprintf(project_env, sizeof project_env, "EV3C_PROJECT_DIR=%s", project_path);
    snprintf(run_env, sizeof run_env, "EV3C_RUN_DIR=%s", run_path);
    snprintf(returned_env, sizeof returned_env, "EV3C_TEST_RETURNED=%s", returned_path);
    snprintf(analog_env, sizeof analog_env, "EV3C_ANALOG_DEVICE=%s", analog_path);
    snprintf(uart_env, sizeof uart_env, "EV3C_UART_DEVICE=%s", uart_path);
    snprintf(lcd_env, sizeof lcd_env, "EV3C_LCD_DEVICE=%s", lcd_path);
    snprintf(ui_library_env, sizeof ui_library_env, "EV3C_UI_LIBRARY=%s", ui_library_path);
    fflush(NULL);
    child = fork();
    if (child == 0) {
        int out = open(out_path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
        int err = open(err_path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
        if (out < 0 || err < 0 || dup2(out, STDOUT_FILENO) < 0 || dup2(err, STDERR_FILENO) < 0) {
            _exit(127);
        }
        close(out);
        close(err);
        execve(self, argv, envp);
        _exit(127);
    }
    if (child < 0 || waitpid(child, &status, 0) != child) {
        return -1;
    }
    return status;
}

static int run_scenario(const char *name)
{
    return run_scenario_in(name, 1);
}

static int exited_ok(int status)
{
    return status != -1 && WIFEXITED(status) && WEXITSTATUS(status) == 0;
}

static void write_file(const char *path, const void *data, size_t length)
{
    FILE *f = fopen(path, "wb");
    if (f == NULL || fwrite(data, 1, length, f) != length || fclose(f) != 0) {
        fprintf(stderr, "cannot write %s: %s\n", path, strerror(errno));
        exit(1);
    }
}

static size_t read_file(const char *path, unsigned char *buffer, size_t capacity)
{
    FILE *f = fopen(path, "rb");
    size_t length;
    if (f == NULL) {
        return 0;
    }
    length = fread(buffer, 1, capacity, f);
    fclose(f);
    return length;
}

static void print_bytes(const char *label, const unsigned char *bytes, size_t length)
{
    fprintf(stderr, "  %s:", label);
    for (size_t i = 0; i < length; i++) {
        fprintf(stderr, " %02X", bytes[i]);
    }
    fputc('\n', stderr);
}

static int bytes_equal(const unsigned char *expected, size_t expected_length,
                       const unsigned char *actual, size_t actual_length)
{
    if (expected_length == actual_length && memcmp(expected, actual, actual_length) == 0) {
        return 1;
    }
    print_bytes("expected", expected, expected_length);
    print_bytes("actual  ", actual, actual_length);
    return 0;
}

/* --- tests ---------------------------------------------------------------- */

static void test_output_printed_before_abort_reaches_the_file(void)
{
    unsigned char out[128];
    size_t length;
    int status = run_scenario("abort");

    CHECK(status != -1 && WIFSIGNALED(status) && WTERMSIG(status) == SIGABRT,
          "the scenario should end with SIGABRT (status %d)", status);
    length = read_file(out_path, out, sizeof out);
    CHECK(length == strlen("first line\nsecond line\n")
          && memcmp(out, "first line\nsecond line\n", length) == 0,
          "stdout captured before abort() is \"%.*s\"", (int)length, (const char *)out);
}

static void test_motor_power_and_stop_write_firmware_commands(void)
{
    static const unsigned char expected[] = {
        0xA4, 0x03, 50,   /* power 50 on A and B */
        0xA6, 0x03,       /* start A and B */
        0xA4, 0x08, 0x9C, /* power -100 on D */
        0xA6, 0x08,       /* start D */
        0xA3, 0x01, 0x01, /* stop A with brake */
        0x02,             /* stop all motors when main() returns */
    };
    unsigned char actual[64];
    size_t length;

    write_file(pwm_path, "", 0);
    CHECK(exited_ok(run_scenario("motor")), "the motor scenario should exit with 0");
    length = read_file(pwm_path, actual, sizeof actual);
    CHECK(bytes_equal(expected, sizeof expected, actual, length), "motor device received other bytes");
}

static void test_return_from_main_stops_all_motors(void)
{
    static const unsigned char expected[] = { 0x02 };
    unsigned char actual[16];
    size_t length;

    write_file(pwm_path, "", 0);
    CHECK(exited_ok(run_scenario("idle")), "the idle scenario should exit with 0");
    length = read_file(pwm_path, actual, sizeof actual);
    CHECK(bytes_equal(expected, sizeof expected, actual, length), "a return from main() should write one stop command");
}

static void test_back_takeover_creates_and_removes_the_request(void)
{
    CHECK(access(bak_path, F_OK) != 0, "no Back request should exist before the test");
    CHECK(exited_ok(run_scenario("take-back")), "taking Back over should succeed");
    CHECK(access(bak_path, F_OK) == 0, "taking Back over should create %s", bak_path);
    CHECK(exited_ok(run_scenario("release-back")), "releasing Back should succeed");
    CHECK(access(bak_path, F_OK) != 0, "releasing Back should remove %s", bak_path);
}

static void test_back_state_follows_the_button_device(void)
{
    /* The device holds one byte per button: up, enter, down, right, left, back. */
    static const unsigned char back_down[6] = { 0, 0, 0, 0, 0, 1 };
    static const unsigned char others_down[6] = { 1, 1, 1, 1, 1, 0 };
    unsigned char out[16];
    size_t length;

    write_file(ui_path, back_down, sizeof back_down);
    CHECK(exited_ok(run_scenario("back-state")), "the back-state scenario should exit with 0");
    length = read_file(out_path, out, sizeof out);
    CHECK(length == 2 && memcmp(out, "1\n", 2) == 0, "Back held down should read as 1, got \"%.*s\"", (int)length, (const char *)out);

    write_file(ui_path, others_down, sizeof others_down);
    CHECK(exited_ok(run_scenario("back-state")), "the back-state scenario should exit with 0");
    length = read_file(out_path, out, sizeof out);
    CHECK(length == 2 && memcmp(out, "0\n", 2) == 0, "Back released should read as 0, got \"%.*s\"", (int)length, (const char *)out);
}

static void test_regulated_and_measured_motor_commands(void)
{
    /*
     * The step and time commands are the firmware's STEPSPEED and TIMESPEED structs
     * (lms2012.h) as the VM writes them with sizeof: three bytes, one byte of padding,
     * three 32-bit little-endian values, the brake byte and three bytes of padding.
     */
    static const unsigned char expected[] = {
        0xA5, 0x03, 40,                         /* speed 40 on A and B */
        0xA6, 0x03,                             /* start A and B */
        0xA7, 0x04, 0xFF,                       /* reverse C */
        0xAE, 0x01, 30, 0, 0, 0, 0, 0, 0x68, 0x01, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,    /* A: 360 degrees at 30, brake */
        0xAF, 0x02, 0xEC, 0, 0, 0, 0, 0, 0xDC, 0x05, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,  /* B: 1500 ms at -20, brake */
        0xAE, 0x04, 0xE7, 0, 0, 0, 0, 0, 0x5A, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,     /* C: -90 degrees is 90 at -25 */
        0xB2, 0x09,                             /* clear the positions of A and D */
        0x02,                                   /* stop all motors when main() returns */
    };
    unsigned char actual[128];
    size_t length;

    write_file(pwm_path, "", 0);
    CHECK(exited_ok(run_scenario("motor-more")), "the motor-more scenario should exit with 0");
    length = read_file(pwm_path, actual, sizeof actual);
    CHECK(bytes_equal(expected, sizeof expected, actual, length), "motor device received other bytes");
}

static void test_sync_and_ramp_commands(void)
{
    /*
     * STEPSYNC and TIMESYNC (lms2012.h): three bytes, one byte of padding, the
     * 16-bit turn, two bytes of padding, the 32-bit amount, the brake byte and
     * three bytes of padding. The driver takes the lower port as the one a
     * positive turn speeds up, so a pair named right to left flips the turn.
     */
    static const unsigned char expected[] = {
        0xB0, 0x06, 50, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,                      /* B+C straight on, no end */
        0xB0, 0x06, 0xD8, 0, 0xE2, 0xFF, 0, 0, 0xD0, 0x02, 0, 0, 1, 0, 0, 0,        /* left C: turn -30, 720 at -40 */
        0xB1, 0x09, 20, 0, 0xC8, 0, 0, 0, 0xDC, 0x05, 0, 0, 1, 0, 0, 0,             /* A+D spin for 1500 ms */
        0xAE, 0x01, 60, 0, 0x5A, 0, 0, 0, 0x68, 0x01, 0, 0, 0x2D, 0, 0, 0, 0, 0, 0, 0, /* A: 90 up, 360, 45 down, coast */
        0xAF, 0x08, 0xE2, 0, 0xF4, 0x01, 0, 0, 0xE8, 0x03, 0, 0, 0xFA, 0, 0, 0, 1, 0, 0, 0, /* D: 500, 1000, 250 ms */
        0xA3, 0x06, 0x01,                                                           /* stop B and C with brake */
        0x02,                                                                       /* stop all when main() returns */
    };
    unsigned char actual[160];
    unsigned char out[32];
    size_t length;

    write_file(pwm_path, "", 0);
    CHECK(exited_ok(run_scenario("motor-sync")), "the motor-sync scenario should exit with 0");
    length = read_file(pwm_path, actual, sizeof actual);
    CHECK(bytes_equal(expected, sizeof expected, actual, length), "motor device received other bytes");
    length = read_file(out_path, out, sizeof out);
    CHECK(length == 8 && memcmp(out, "1 1 1 1\n", 8) == 0,
          "one port twice, turn 201, two ports as one side and a negative ramp are refused; got \"%.*s\"", (int)length,
          (const char *)out);
}

static void test_motor_state_reads_the_test_flags(void)
{
    /* Reading the PWM device gives "<ready> <test> " (d_pwm.c Device1Read); test bit 1 is port B. */
    unsigned char out[32];
    size_t length;

    write_file(pwm_path, "3 2 ", 4);
    CHECK(exited_ok(run_scenario("motor-busy")), "the motor-busy scenario should exit with 0");
    length = read_file(out_path, out, sizeof out);
    CHECK(length == 10 && memcmp(out, "1 0 1 0 1\n", 10) == 0,
          "B busy, A idle, waiting for A is done at once, waiting 30 ms for B runs out; got \"%.*s\"", (int)length, (const char *)out);
}

static void test_motor_read_takes_speed_and_degrees_from_the_shared_motor_data(void)
{
    /* MOTORDATA per port: TachoCounts (int32), Speed (int8), TachoSensor (int32), as d_pwm.c shares it. */
    struct { int counts; signed char speed; int sensor; } motors[4];
    unsigned char out[64];
    size_t length;

    memset(motors, 0, sizeof motors);
    motors[2].counts = 1000;
    motors[2].speed = -25;
    motors[2].sensor = 720;
    write_file(motor_path, motors, sizeof motors);
    CHECK(exited_ok(run_scenario("motor-read")), "the motor-read scenario should exit with 0");
    length = read_file(out_path, out, sizeof out);
    CHECK(length == 10 && memcmp(out, "-25 720 1\n", 10) == 0,
          "port C reads speed -25 and 720 degrees, and two ports are refused; got \"%.*s\"", (int)length, (const char *)out);
}

static void test_every_button_reads_from_the_button_device(void)
{
    static const unsigned char enter_down[6] = { 0, 1, 0, 0, 0, 0 };
    unsigned char out[32];
    size_t length;

    write_file(ui_path, enter_down, sizeof enter_down);
    CHECK(exited_ok(run_scenario("buttons")), "the buttons scenario should exit with 0");
    length = read_file(out_path, out, sizeof out);
    CHECK(length == 10 && memcmp(out, "0 1 0 1 1\n", 10) == 0,
          "only Enter is down, any button is, and 0x40 names none; got \"%.*s\"", (int)length, (const char *)out);
}

static void test_led_pattern_is_written_as_the_firmware_reads_it(void)
{
    /* d_ui.c Device1Write takes two bytes and reads the pattern from the first, as a digit. */
    static const unsigned char expected[] = { '5', 0 };
    unsigned char actual[16];
    size_t length;

    write_file(ui_path, "", 0);
    CHECK(exited_ok(run_scenario("led")), "the led scenario should exit with 0");
    length = read_file(ui_path, actual, sizeof actual);
    CHECK(bytes_equal(expected, sizeof expected, actual, length), "the LED device received other bytes");
}

static void test_tones_are_written_as_the_sound_driver_reads_them(void)
{
    /* d_sound.c: TONE (1), volume, frequency and duration as 16-bit little-endian; BREAK (0) stops. */
    static const unsigned char expected[] = {
        0x01, 70, 0xB8, 0x01, 0xC8, 0x00,       /* 440 Hz, 200 ms, volume 70 */
        0x01, 50, 0xE8, 0x03, 0x00, 0x00,       /* 1000 Hz at the default volume; 0 ms sounds until stopped */
        0x00,                                   /* stop */
    };
    unsigned char actual[32];
    size_t length;

    write_file(sound_path, "", 0);
    CHECK(exited_ok(run_scenario("sound")), "the sound scenario should exit with 0");
    length = read_file(sound_path, actual, sizeof actual);
    CHECK(bytes_equal(expected, sizeof expected, actual, length), "the sound device received other bytes");
}

static void test_returned_files_are_listed_in_the_run_folder(void)
{
    char expected[1200];
    char actual[1200];
    size_t length;

    unlink(ret_path);
    CHECK(exited_ok(run_scenario("return-files")), "the return-files scenario should exit with 0");
    snprintf(expected, sizeof expected, "%s\n%s/other.txt\n%s/relative.txt\n", returned_path, project_path, project_path);
    length = read_file(ret_path, (unsigned char *)actual, sizeof actual - 1);
    actual[length] = '\0';
    CHECK(strcmp(actual, expected) == 0, "the return list should be\n%s\nbut is\n%s", expected, actual);
    CHECK(access(returned_path, F_OK) == 0, "the returned file should exist");

    unlink(returned_path);
    unlink(ret_path);
}

static void test_return_refuses_a_missing_file_and_a_line_break(void)
{
    unlink(ret_path);
    CHECK(exited_ok(run_scenario("return-refused")), "missing files and line breaks should be refused");
    CHECK(access(ret_path, F_OK) != 0, "nothing should be recorded for refused files");
}

static void test_return_outside_a_run_opens_the_file_and_records_nothing(void)
{
    unlink(ret_path);
    CHECK(exited_ok(run_scenario_in("return-without-run", 0)), "the file should open without a run folder");
    CHECK(access(returned_path, F_OK) == 0, "the file should exist");
    CHECK(access(ret_path, F_OK) != 0, "no list should be written without a run folder");
    unlink(returned_path);
}

static void test_a_plain_call_that_fails_names_the_line_and_stops_the_program(void)
{
    static const unsigned char stop_all[] = { 0x02 };
    char err[512];
    unsigned char pwm[16];
    size_t length;
    int status;

    write_file(pwm_path, "", 0);
    status = run_scenario("plain-error");
    CHECK(status != -1 && WIFEXITED(status) && WEXITSTATUS(status) == EV3_EXIT_LIBRARY_ERROR,
          "the program should end with exit code %d (status %d)", EV3_EXIT_LIBRARY_ERROR, status);
    length = read_file(err_path, (unsigned char *)err, sizeof err - 1);
    err[length] = '\0';
    CHECK(strstr(err, "test_core.c:") != NULL
              && strstr(err, ": ev3_motor_start: port 1 is a sensor port; motors are on ports A to D\n"
                             EV3_LIBRARY_STOP_LINE "\n") != NULL,
          "stderr should name the file, line, function and what went wrong; got\n%s", err);
    length = read_file(out_path, pwm, sizeof pwm);
    CHECK(length == 0, "nothing after the failed call should run");
    length = read_file(pwm_path, pwm, sizeof pwm);
    CHECK(bytes_equal(stop_all, sizeof stop_all, pwm, length), "the stop should still stop every motor, and nothing else reach the motors");
}

static void test_a_try_call_returns_the_code_and_says_why(void)
{
    static const char expected[] =
        "EV3_ERR_PARAM|speed 150 is outside -100 to 100\n"
        "EV3_ERR_PORT|port 0x201 is a sensor port; motors are on ports A to D\n";
    char out[256];
    size_t length;

    write_file(pwm_path, "", 0);
    CHECK(exited_ok(run_scenario("try-error")), "the try-error scenario should exit with 0");
    length = read_file(out_path, (unsigned char *)out, sizeof out - 1);
    out[length] = '\0';
    CHECK(strcmp(out, expected) == 0, "expected\n%sgot\n%s", expected, out);
}

static void test_a_run_returns_when_the_motors_are_done(void)
{
    static const unsigned char expected[] = {
        0xAE, 0x02, 50, 0, 0, 0, 0, 0, 0xB4, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,          /* 180 degrees at 50 */
        0xAE, 0x02, 0xCE, 0, 0, 0, 0, 0, 0xB4, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,        /* half a turn at -50 */
        0xAF, 0x02, 10, 0, 0, 0, 0, 0, 0xFA, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0,          /* 250 ms at 10 */
        0x02,
    };
    unsigned char actual[128];
    size_t length;

    /* The fake device reads as "nothing ready, nothing busy", then the commands written after it. */
    write_file(pwm_path, "0 0 ", 4);
    CHECK(exited_ok(run_scenario("run-blocking")), "the run-blocking scenario should exit with 0");
    length = read_file(pwm_path, actual, sizeof actual);
    CHECK(length > 4 && bytes_equal(expected, sizeof expected, actual + 4, length - 4), "motor device received other bytes");
}

/* Offsets into lms2012.h's ANALOG and UART, as ev3lib's sensor code asserts them. */
#define ANALOG_SIZE 5172
#define ANALOG_PIN1(p) (2 * (p))
#define ANALOG_PIN6(p) (8 + 2 * (p))
#define ANALOG_CONN(p) (5160 + (p))
#define UART_SIZE 42744
#define UART_TYPES(p, m) (((p) * 8 + (m)) * 56)
#define UART_RAW(p, slot) (4192 + ((p) * 300 + (slot)) * 32)
#define UART_ACTUAL(p) (42592 + 2 * (p))
#define UART_STATUS(p) (42608 + (p))

static void put16(unsigned char *at, int value)
{
    at[0] = (unsigned char)(value & 0xFF);
    at[1] = (unsigned char)((value >> 8) & 0xFF);
}

static void put_float(unsigned char *at, float value)
{
    memcpy(at, &value, sizeof value);
}

/* One mode of a UART sensor, as the driver describes it (lms2012.h TYPES). */
static void put_mode(unsigned char *uart, int port, int mode, const char *name, int type, int format,
                     float raw_max, float si_max)
{
    unsigned char *types = uart + UART_TYPES(port, mode);
    memcpy(types, name, strlen(name));
    types[12] = (unsigned char)type;
    types[14] = (unsigned char)mode;
    types[15] = 1;
    types[16] = (unsigned char)format;
    put_float(types + 24, raw_max);
    put_float(types + 40, si_max);
}

static void test_sensors_are_found_and_read_from_the_shared_memory(void)
{
    static unsigned char analog[ANALOG_SIZE];
    static unsigned char uart[UART_SIZE];
    static const char expected[] =
        "1 2 3 0\n"
        "1 42 123.4 42/300/400\n"
        "EV3_ERR_DEVICE|nothing is plugged into port 4; plug in a gyro sensor\n"
        "EV3_ERR_DEVICE|port 2 has a color sensor, not a gyro sensor\n"
        "EV3_ERR_PORT|port B is a motor port; sensors are on ports 1 to 4\n";
    char out[512];
    size_t length;

    memset(analog, 0, sizeof analog);
    memset(uart, 0, sizeof uart);
    /* Port 1: a touch sensor (its ID on pin 1 is 417 mV, 341 counts), pressed (pin 6 high). */
    analog[ANALOG_CONN(0)] = 121;
    put16(analog + ANALOG_PIN1(0), 344);
    put16(analog + ANALOG_PIN6(0), 1800);
    /* Port 2: a color sensor; its newest data, in slot 5, serve both modes read. */
    analog[ANALOG_CONN(1)] = 122;
    put_mode(uart, 1, 0, "COL-REFLECT", 29, 0, 100.0f, 100.0f);
    put_mode(uart, 1, 4, "RGB-RAW", 29, 1, 1020.0f, 1020.0f);
    put16(uart + UART_ACTUAL(1), 5);
    put16(uart + UART_RAW(1, 5), 42);
    put16(uart + UART_RAW(1, 5) + 2, 300);
    put16(uart + UART_RAW(1, 5) + 4, 400);
    uart[UART_STATUS(1)] = 0x08;
    /* Port 3: an ultrasonic sensor, 1234 mm in raw units, 123.4 cm. */
    analog[ANALOG_CONN(2)] = 122;
    put_mode(uart, 2, 0, "US-DIST-CM", 30, 1, 2550.0f, 255.0f);
    put16(uart + UART_ACTUAL(2), 7);
    put16(uart + UART_RAW(2, 7), 1234);
    uart[UART_STATUS(2)] = 0x08;
    /* Port 4: empty. */
    analog[ANALOG_CONN(3)] = 126;
    write_file(analog_path, analog, sizeof analog);
    write_file(uart_path, uart, sizeof uart);

    CHECK(exited_ok(run_scenario("sensors")), "the sensors scenario should exit with 0");
    length = read_file(out_path, (unsigned char *)out, sizeof out - 1);
    out[length] = '\0';
    CHECK(strcmp(out, expected) == 0, "expected\n%sgot\n%s", expected, out);
}

static void put32(unsigned char *at, unsigned long value)
{
    put16(at, (int)(value & 0xFFFF));
    put16(at + 2, (int)(value >> 16));
}

/*
 * A stand-in for the firmware's libc_ui.so: a 32-bit little-endian ELF file
 * whose .data holds the four font sheets (sizes from d_lcd.c FontInfo) under
 * their symbols. Every sheet is blank but for a few rows of '!' and '?'.
 */
static void write_fake_ui_library(void)
{
    static const char strings[] = "\0normal_font_bits\0small_font_bits\0large_font_bits\0tiny_font_bits";
    static const unsigned long name_at[4] = { 1, 18, 34, 50 };
    static const unsigned long sheet[4] = { 16 * 1 * 9 * 6, 16 * 1 * 8 * 6, 16 * 2 * 16 * 6, 16 * 1 * 7 * 6 };
    enum { DATA_AT = 64, DATA_ADDR = 0x1000 };
    static unsigned char elf[8192];
    unsigned long data_size = 0, sheet_at[4], strtab_at, symtab_at, shdr_at, total;

    memset(elf, 0, sizeof elf);
    for (int f = 0; f < 4; f++) {
        sheet_at[f] = data_size;
        data_size += sheet[f];
    }
    /* normal '!' (index 1): row 0 has the leftmost pixel, row 1 the eighth */
    elf[DATA_AT + sheet_at[0] + 1] = 0x01;
    elf[DATA_AT + sheet_at[0] + 1 + 16] = 0x80;
    /* tiny '?' (index 31: column 15 of the second sheet row): row 0 has the fifth pixel */
    elf[DATA_AT + sheet_at[3] + 15 + 1 * 7 * 16] = 0x10;

    strtab_at = DATA_AT + data_size;
    memcpy(elf + strtab_at, strings, sizeof strings);
    symtab_at = (strtab_at + sizeof strings + 3) & ~3UL;
    for (int f = 0; f < 4; f++) {
        unsigned char *symbol = elf + symtab_at + 16 * (f + 1);
        put32(symbol, name_at[f]);
        put32(symbol + 4, DATA_ADDR + sheet_at[f]);
        put32(symbol + 8, sheet[f]);
        symbol[12] = 1; /* local object */
        put16(symbol + 14, 1);
    }
    shdr_at = symtab_at + 16 * 5;
    /* [1] .data, [2] .symtab linked to [3] .strtab */
    put32(elf + shdr_at + 40 + 4, 1), put32(elf + shdr_at + 40 + 12, DATA_ADDR);
    put32(elf + shdr_at + 40 + 16, DATA_AT), put32(elf + shdr_at + 40 + 20, data_size);
    put32(elf + shdr_at + 80 + 4, 2), put32(elf + shdr_at + 80 + 16, symtab_at);
    put32(elf + shdr_at + 80 + 20, 16 * 5), put32(elf + shdr_at + 80 + 24, 3), put32(elf + shdr_at + 80 + 36, 16);
    put32(elf + shdr_at + 120 + 4, 3), put32(elf + shdr_at + 120 + 16, strtab_at);
    put32(elf + shdr_at + 120 + 20, sizeof strings);
    total = shdr_at + 4 * 40;

    memcpy(elf, "\177ELF\001\001\001", 7);
    put16(elf + 16, 3), put16(elf + 18, 40);
    put32(elf + 0x20, shdr_at);
    put16(elf + 0x28, 52), put16(elf + 0x2E, 40), put16(elf + 0x30, 4);
    write_file(ui_library_path, elf, total);
}

static void test_the_display_draws_with_the_firmware_fonts_into_the_framebuffer(void)
{
    /*
     * The framebuffer as d_lcd.c writes it: 128 rows of 60 bytes, three pixels a
     * byte (bits 7-5, 4-2, 1-0), all of a pixel's bits set for black.
     */
    static unsigned char expected[60 * 128];
    unsigned char actual[60 * 128 + 1];
    unsigned char out[64];
    size_t length;
    struct stat st;

    memset(expected, 0, sizeof expected);
    expected[0] = 0xFF;            /* pixels 0-2 of row 0 */
    expected[1] = 0x1C;            /* pixel 4 */
    expected[10 * 60] = 0xE0;      /* '!' row 0: pixel 0 of row 10 */
    expected[11 * 60 + 2] = 0x1C;  /* '!' row 1: pixel 7 of row 11 */
    expected[20 * 60 + 11] = 0x1C; /* tiny '?' for the missing letter: pixel 34 of row 20 */

    write_fake_ui_library();
    write_file(lcd_path, "", 0);
    unlink(lcd_marker);
    CHECK(exited_ok(run_scenario("lcd")), "the lcd scenario should exit with 0");
    length = read_file(lcd_path, actual, sizeof actual);
    CHECK(bytes_equal(expected, sizeof expected, actual, length), "the framebuffer holds other bytes");
    length = read_file(out_path, out, sizeof out);
    CHECK(length == 6 && memcmp(out, "1 1 1\n", 6) == 0,
          "font 4, a negative width and a cursor off the display are refused; got \"%.*s\"", (int)length,
          (const char *)out);
    /* No launcher answers in the test, so the request for the screen stays behind. */
    CHECK(stat(lcd_marker, &st) == 0, "the program asks for the screen with .lcd in its project folder");
    unlink(lcd_marker);

    /* Shapes end, and land where they should: a line's ends, a circle's rightmost point. */
    write_file(lcd_path, "", 0);
    CHECK(exited_ok(run_scenario("lcd-shapes")), "the lcd-shapes scenario should end within 5 s and exit with 0");
    length = read_file(lcd_path, actual, sizeof actual);
    CHECK(length == 60 * 128, "the framebuffer is 60 by 128 bytes, got %zu", length);
    if (length == 60 * 128) {
        CHECK((actual[127 * 60 + 177 / 3] & 0xE0 >> 3 * (177 % 3)) != 0, "the diagonal reaches 177, 127");
        CHECK((actual[0 * 60 + 160 / 3] & 0xE0 >> 3 * (160 % 3)) != 0, "the steep line reaches 160, 0");
        CHECK((actual[64 * 60 + 75 / 3] & 0xE0 >> 3 * (75 % 3)) != 0, "the circle passes 75, 64");
        CHECK((actual[95 * 60 + 85 / 3] & 0xE0 >> 3 * (85 % 3)) == 0, "outside the shapes stays white");
    }
    unlink(lcd_marker);

    /*
     * Scrolling moves whole lines: every '!' keeps its first row on a multiple of
     * 9, so the top line is whole, and the cursor's empty line is the last one.
     */
    write_file(lcd_path, "", 0);
    CHECK(exited_ok(run_scenario("lcd-scroll")), "the lcd-scroll scenario should end within 5 s and exit with 0");
    length = read_file(lcd_path, actual, sizeof actual);
    CHECK(length == 60 * 128, "the framebuffer is 60 by 128 bytes, got %zu", length);
    if (length == 60 * 128) {
        for (int y = 0; y < 128; y++) {
            bool black = (actual[y * 60] & 0xE0) != 0;
            bool expect = y % 9 == 0 && y < 117;
            CHECK(black == expect, "row %d: pixel 0 is %s", y, black ? "black" : "white");
        }
    }
    unlink(lcd_marker);

    unlink(ui_library_path);
    CHECK(exited_ok(run_scenario("lcd-no-fonts")), "the lcd-no-fonts scenario should exit with 0");
    length = read_file(out_path, out, sizeof out - 1);
    out[length] = '\0';
    CHECK(strstr((const char *)out, "EV3_ERR_DEVICE|the brick's fonts cannot be read") == (const char *)out,
          "without the firmware's library text fails with EV3_ERR_DEVICE; got \"%s\"", (const char *)out);
}

int main(int argc, char **argv)
{
    if (argc == 3 && strcmp(argv[1], "--scenario") == 0) {
        return scenario(argv[2]);
    }
    self = argv[0];

    const char *tmp = getenv("TMPDIR");
    snprintf(root, sizeof root, "%s/ev3lib-test-XXXXXX", tmp != NULL && tmp[0] != '\0' ? tmp : "/tmp");
    if (mkdtemp(root) == NULL) {
        fprintf(stderr, "cannot create a temporary folder: %s\n", strerror(errno));
        return 1;
    }
    snprintf(pwm_path, sizeof pwm_path, "%s/pwm", root);
    snprintf(ui_path, sizeof ui_path, "%s/ui", root);
    snprintf(motor_path, sizeof motor_path, "%s/motor", root);
    snprintf(sound_path, sizeof sound_path, "%s/sound", root);
    snprintf(project_path, sizeof project_path, "%s/project", root);
    snprintf(bak_path, sizeof bak_path, "%s/.bak", project_path);
    snprintf(out_path, sizeof out_path, "%s/out", root);
    snprintf(err_path, sizeof err_path, "%s/err", root);
    snprintf(analog_path, sizeof analog_path, "%s/analog", root);
    snprintf(uart_path, sizeof uart_path, "%s/uart", root);
    snprintf(lcd_path, sizeof lcd_path, "%s/lcd", root);
    snprintf(ui_library_path, sizeof ui_library_path, "%s/libc_ui.so", root);
    snprintf(lcd_marker, sizeof lcd_marker, "%s/.lcd", project_path);
    snprintf(run_path, sizeof run_path, "%s/run", root);
    snprintf(ret_path, sizeof ret_path, "%s/ret", run_path);
    snprintf(returned_path, sizeof returned_path, "%s/results.csv", project_path);
    if (mkdir(project_path, 0755) != 0 || mkdir(run_path, 0755) != 0) {
        fprintf(stderr, "cannot create %s or %s: %s\n", project_path, run_path, strerror(errno));
        return 1;
    }

    test_output_printed_before_abort_reaches_the_file();
    test_motor_power_and_stop_write_firmware_commands();
    test_return_from_main_stops_all_motors();
    test_back_takeover_creates_and_removes_the_request();
    test_back_state_follows_the_button_device();
    test_regulated_and_measured_motor_commands();
    test_sync_and_ramp_commands();
    test_motor_state_reads_the_test_flags();
    test_motor_read_takes_speed_and_degrees_from_the_shared_motor_data();
    test_every_button_reads_from_the_button_device();
    test_led_pattern_is_written_as_the_firmware_reads_it();
    test_tones_are_written_as_the_sound_driver_reads_them();
    test_returned_files_are_listed_in_the_run_folder();
    test_return_refuses_a_missing_file_and_a_line_break();
    test_return_outside_a_run_opens_the_file_and_records_nothing();
    test_a_plain_call_that_fails_names_the_line_and_stops_the_program();
    test_a_try_call_returns_the_code_and_says_why();
    test_a_run_returns_when_the_motors_are_done();
    test_sensors_are_found_and_read_from_the_shared_memory();
    test_the_display_draws_with_the_firmware_fonts_into_the_framebuffer();

    unlink(bak_path);
    unlink(lcd_marker);
    unlink(lcd_path);
    unlink(ui_library_path);
    unlink(ret_path);
    rmdir(run_path);
    {
        char other[400];
        snprintf(other, sizeof other, "%s/other.txt", project_path);
        unlink(other);
        snprintf(other, sizeof other, "%s/relative.txt", project_path);
        unlink(other);
    }
    rmdir(project_path);
    unlink(pwm_path);
    unlink(ui_path);
    unlink(motor_path);
    unlink(sound_path);
    unlink(out_path);
    unlink(err_path);
    unlink(analog_path);
    unlink(uart_path);
    rmdir(root);

    if (failures != 0) {
        fprintf(stderr, "test_core: %d check(s) failed\n", failures);
        return 1;
    }
    printf("test_core: all checks passed\n");
    return 0;
}
