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
        if (ev3_motor_set_power(EV3_PORT_A | EV3_PORT_B, 50) != 0
            || ev3_motor_set_power(EV3_PORT_D, -150) != 0
            || ev3_motor_stop(EV3_PORT_A, 1) != 0) {
            return 1;
        }
        return 0;
    }
    if (strcmp(name, "idle") == 0) {
        return 0;
    }
    if (strcmp(name, "take-back") == 0) {
        return ev3_button_take_back() == 0 ? 0 : 1;
    }
    if (strcmp(name, "release-back") == 0) {
        return ev3_button_release_back() == 0 ? 0 : 1;
    }
    if (strcmp(name, "motor-more") == 0) {
        if (ev3_motor_set_speed(EV3_PORT_A | EV3_PORT_B, 40) != 0
            || ev3_motor_set_polarity(EV3_PORT_C, -1) != 0
            || ev3_motor_run_degrees(EV3_PORT_A, 30, 360, 1) != 0
            || ev3_motor_run_ms(EV3_PORT_B, -20, 1500, 0) != 0
            || ev3_motor_reset_count(EV3_PORT_A | EV3_PORT_D) != 0) {
            return 1;
        }
        return 0;
    }
    if (strcmp(name, "motor-busy") == 0) {
        printf("%d %d %d\n", ev3_motor_busy(EV3_PORT_B), ev3_motor_busy(EV3_PORT_A), ev3_motor_wait(EV3_PORT_A));
        return 0;
    }
    if (strcmp(name, "motor-read") == 0) {
        int speed = 0, degrees = 0;
        int ok = ev3_motor_read(EV3_PORT_C, &speed, &degrees);
        errno = 0;
        printf("%d %d %d %d\n", ok, speed, degrees, ev3_motor_read(EV3_PORT_A | EV3_PORT_B, &speed, &degrees) == -1 && errno == EINVAL);
        return 0;
    }
    if (strcmp(name, "buttons") == 0) {
        errno = 0;
        printf("%d %d %d %d\n", ev3_button_pressed(EV3_BUTTON_UP), ev3_button_pressed(EV3_BUTTON_ENTER),
               ev3_button_pressed(EV3_BUTTON_LEFT), ev3_button_pressed(9) == -1 && errno == EINVAL);
        return 0;
    }
    if (strcmp(name, "led") == 0) {
        errno = 0;
        if (ev3_led_set(EV3_LED_RED_FLASH) != 0 || ev3_led_set(10) != -1 || errno != EINVAL) {
            return 1;
        }
        return 0;
    }
    if (strcmp(name, "sound") == 0) {
        if (ev3_sound_tone(50, 440, 200) != 0 || ev3_sound_tone(150, 70000, 0) != 0 || ev3_sound_stop() != 0) {
            return 1;
        }
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
        if (f == NULL || fclose(f) != 0 || ev3_return_file(other) != 0) {
            return 2;
        }
        /* A relative path is taken from the current folder and recorded in full. */
        if (chdir(project) != 0 || (f = ev3_return_fopen("relative.txt", "w")) == NULL || fclose(f) != 0) {
            return 3;
        }
        return 0;
    }
    if (strcmp(name, "return-refused") == 0) {
        char missing[400];
        snprintf(missing, sizeof missing, "%s/missing.txt", getenv("EV3C_PROJECT_DIR"));
        errno = 0;
        if (ev3_return_file(missing) != -1 || errno != ENOENT) {
            return 1;
        }
        errno = 0;
        if (ev3_return_fopen("bad\nname.txt", "w") != NULL || errno != EINVAL || access("bad\nname.txt", F_OK) == 0) {
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
        printf("%d\n", ev3_button_back_pressed());
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
    char *argv[] = { (char *)self, "--scenario", (char *)name, NULL };
    char *envp[] = { pwm_env, ui_env, motor_env, sound_env, project_env, returned_env, in_run ? run_env : NULL, NULL };
    int status = -1;
    pid_t child;

    snprintf(pwm_env, sizeof pwm_env, "EV3C_PWM_DEVICE=%s", pwm_path);
    snprintf(ui_env, sizeof ui_env, "EV3C_UI_DEVICE=%s", ui_path);
    snprintf(motor_env, sizeof motor_env, "EV3C_MOTOR_DEVICE=%s", motor_path);
    snprintf(sound_env, sizeof sound_env, "EV3C_SOUND_DEVICE=%s", sound_path);
    snprintf(project_env, sizeof project_env, "EV3C_PROJECT_DIR=%s", project_path);
    snprintf(run_env, sizeof run_env, "EV3C_RUN_DIR=%s", run_path);
    snprintf(returned_env, sizeof returned_env, "EV3C_TEST_RETURNED=%s", returned_path);
    fflush(NULL);
    child = fork();
    if (child == 0) {
        int out = open(out_path, O_WRONLY | O_CREAT | O_TRUNC, 0644);
        if (out < 0 || dup2(out, STDOUT_FILENO) < 0) {
            _exit(127);
        }
        close(out);
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
        0xA4, 0x08, 0x9C, /* power -150 limited to -100 on D */
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
        0xAF, 0x02, 0xEC, 0, 0, 0, 0, 0, 0xDC, 0x05, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,  /* B: 1500 ms at -20, coast */
        0xB2, 0x09,                             /* clear the counts of A and D */
        0x02,                                   /* stop all motors when main() returns */
    };
    unsigned char actual[128];
    size_t length;

    write_file(pwm_path, "", 0);
    CHECK(exited_ok(run_scenario("motor-more")), "the motor-more scenario should exit with 0");
    length = read_file(pwm_path, actual, sizeof actual);
    CHECK(bytes_equal(expected, sizeof expected, actual, length), "motor device received other bytes");
}

static void test_motor_state_reads_the_test_flags(void)
{
    /* Reading the PWM device gives "<ready> <test> " (d_pwm.c Device1Read); test bit 1 is port B. */
    unsigned char out[32];
    size_t length;

    write_file(pwm_path, "3 2 ", 4);
    CHECK(exited_ok(run_scenario("motor-busy")), "the motor-busy scenario should exit with 0");
    length = read_file(out_path, out, sizeof out);
    CHECK(length == 6 && memcmp(out, "1 0 0\n", 6) == 0, "B busy, A idle, waiting for A returns at once; got \"%.*s\"", (int)length, (const char *)out);
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
    CHECK(length == 12 && memcmp(out, "0 -25 720 1\n", 12) == 0, "port C reads speed -25 and 720 degrees; got \"%.*s\"", (int)length, (const char *)out);
}

static void test_every_button_reads_from_the_button_device(void)
{
    static const unsigned char enter_down[6] = { 0, 1, 0, 0, 0, 0 };
    unsigned char out[32];
    size_t length;

    write_file(ui_path, enter_down, sizeof enter_down);
    CHECK(exited_ok(run_scenario("buttons")), "the buttons scenario should exit with 0");
    length = read_file(out_path, out, sizeof out);
    CHECK(length == 8 && memcmp(out, "0 1 0 1\n", 8) == 0, "only Enter is down, and button 9 does not exist; got \"%.*s\"", (int)length, (const char *)out);
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
        0x01, 50, 0xB8, 0x01, 0xC8, 0x00,       /* 440 Hz, 200 ms, volume 50 */
        0x01, 100, 0xFF, 0xFF, 0x00, 0x00,      /* volume and frequency limited; 0 ms sounds until stopped */
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
    test_motor_state_reads_the_test_flags();
    test_motor_read_takes_speed_and_degrees_from_the_shared_motor_data();
    test_every_button_reads_from_the_button_device();
    test_led_pattern_is_written_as_the_firmware_reads_it();
    test_tones_are_written_as_the_sound_driver_reads_them();
    test_returned_files_are_listed_in_the_run_folder();
    test_return_refuses_a_missing_file_and_a_line_break();
    test_return_outside_a_run_opens_the_file_and_records_nothing();

    unlink(bak_path);
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
    rmdir(root);

    if (failures != 0) {
        fprintf(stderr, "test_core: %d check(s) failed\n", failures);
        return 1;
    }
    printf("test_core: all checks passed\n");
    return 0;
}
