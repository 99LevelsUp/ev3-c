/*
 * Host tests for the ev3lib core: output buffering, motor commands, the stop
 * on exit, the Back takeover request, and the Back button state.
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
static char project_path[300];
static char bak_path[320];
static char out_path[300];
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
    if (strcmp(name, "back-state") == 0) {
        printf("%d\n", ev3_button_back_pressed());
        return 0;
    }
    fprintf(stderr, "unknown scenario %s\n", name);
    return 2;
}

/* --- helpers, run in the test process ------------------------------------ */

/* Runs a scenario in a child process with stdout sent to out_path; returns its wait status. */
static int run_scenario(const char *name)
{
    char pwm_env[320], ui_env[320], project_env[320];
    char *argv[] = { (char *)self, "--scenario", (char *)name, NULL };
    char *envp[] = { pwm_env, ui_env, project_env, NULL };
    int status = -1;
    pid_t child;

    snprintf(pwm_env, sizeof pwm_env, "EV3C_PWM_DEVICE=%s", pwm_path);
    snprintf(ui_env, sizeof ui_env, "EV3C_UI_DEVICE=%s", ui_path);
    snprintf(project_env, sizeof project_env, "EV3C_PROJECT_DIR=%s", project_path);
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
    snprintf(project_path, sizeof project_path, "%s/project", root);
    snprintf(bak_path, sizeof bak_path, "%s/.bak", project_path);
    snprintf(out_path, sizeof out_path, "%s/out", root);
    if (mkdir(project_path, 0755) != 0) {
        fprintf(stderr, "cannot create %s: %s\n", project_path, strerror(errno));
        return 1;
    }

    test_output_printed_before_abort_reaches_the_file();
    test_motor_power_and_stop_write_firmware_commands();
    test_return_from_main_stops_all_motors();
    test_back_takeover_creates_and_removes_the_request();
    test_back_state_follows_the_button_device();

    unlink(bak_path);
    rmdir(project_path);
    unlink(pwm_path);
    unlink(ui_path);
    unlink(out_path);
    rmdir(root);

    if (failures != 0) {
        fprintf(stderr, "test_core: %d check(s) failed\n", failures);
        return 1;
    }
    printf("test_core: all checks passed\n");
    return 0;
}
