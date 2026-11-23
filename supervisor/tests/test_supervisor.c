/*
 * Host tests for the supervisor process: how a run ends, process groups,
 * slot-end detection, output capture, the motor stop, the run lock, and the
 * markers in the project folder.
 *
 * Usage: test_supervisor <path to a host build of the supervisor>
 *
 * Each case builds a fake brick in a temporary folder under TMPDIR (default
 * /tmp): a project folder whose "prog" is a symlink to this test binary, a
 * fake /proc with an lms2012 process whose descriptor 5 is a symlink to the
 * project's .tok, an empty file standing in for the motor device, and a lock
 * file. The supervisor runs with EV3C_PROC_ROOT, EV3C_PWM_DEVICE and
 * EV3C_LOCK_FILE pointing there. Started as "prog", this binary plays the
 * student program named by SUPTEST_SCENARIO, which reaches it through the
 * supervisor's environment.
 *
 * Needs POSIX and a /proc that shows process states (Linux).
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#define VM_PID "4242"

static char sup_path[PATH_MAX];
static char self_path[PATH_MAX];
static char root[256];
static int failures;
static int case_no;

#define CHECK(cond, ...)                                                   \
    do {                                                                   \
        if (!(cond)) {                                                     \
            failures++;                                                    \
            fprintf(stderr, "FAIL %s:%d: ", __FILE__, __LINE__);           \
            fprintf(stderr, __VA_ARGS__);                                  \
            fputc('\n', stderr);                                           \
        }                                                                  \
    } while (0)

static long long now_ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000 + t.tv_nsec / 1000000;
}

static void sleep_ms(int ms)
{
    poll(NULL, 0, ms);
}

static int exists(const char *path)
{
    struct stat st;
    return lstat(path, &st) == 0;
}

static void write_file(const char *path, const char *text)
{
    FILE *f = fopen(path, "wb");
    if (f == NULL || fputs(text, f) == EOF || fclose(f) != 0) {
        fprintf(stderr, "cannot write %s\n", path);
        exit(2);
    }
}

/* Reads a whole file; returns its length, or -1 if it is missing. */
static long read_file(const char *path, char *buf, size_t size)
{
    FILE *f = fopen(path, "rb");
    size_t n;
    if (f == NULL) {
        buf[0] = '\0';
        return -1;
    }
    n = fread(buf, 1, size - 1, f);
    buf[n] = '\0';
    fclose(f);
    return (long)n;
}

static long file_size(const char *path)
{
    struct stat st;
    return stat(path, &st) == 0 ? (long)st.st_size : -1;
}

/* Is the line present in key=value text? */
static int has_line(const char *text, const char *line)
{
    size_t length = strlen(line);
    const char *at = text;
    while ((at = strstr(at, line)) != NULL) {
        if ((at == text || at[-1] == '\n') && (at[length] == '\n' || at[length] == '\0')) {
            return 1;
        }
        at += length;
    }
    return 0;
}

static int has_key(const char *text, const char *key)
{
    char prefix[64];
    const char *at = text;
    snprintf(prefix, sizeof prefix, "%s=", key);
    while ((at = strstr(at, prefix)) != NULL) {
        if (at == text || at[-1] == '\n') {
            return 1;
        }
        at++;
    }
    return 0;
}

/* A process counts as gone when it no longer exists or is a zombie. */
static int process_gone(pid_t pid)
{
    char path[64], text[256];
    const char *state;
    snprintf(path, sizeof path, "/proc/%d/stat", (int)pid);
    if (read_file(path, text, sizeof text) < 0) {
        return 1;
    }
    state = strrchr(text, ')');
    return state != NULL && (state[2] == 'Z' || state[2] == 'X');
}

/* --- the student program played by this binary ----------------------------- */

static void program_file(char *path, size_t size, const char *name)
{
    snprintf(path, size, "%s/%s", getenv("EV3C_RUN_DIR"), name);
}

static void mark(const char *name, const char *text)
{
    char path[PATH_MAX], tmp[PATH_MAX + 4];
    program_file(path, sizeof path, name);
    snprintf(tmp, sizeof tmp, "%s.t", path);
    write_file(tmp, text);
    rename(tmp, path);
}

static void wait_for_mark(const char *name)
{
    char path[PATH_MAX];
    program_file(path, sizeof path, name);
    while (!exists(path)) {
        sleep_ms(1);
    }
}

static void sleep_forever(void)
{
    for (;;) {
        sleep_ms(1000);
    }
}

static int play(const char *scenario)
{
    if (strcmp(scenario, "exit3") == 0) {
        printf("out line\n");
        fflush(stdout);
        fprintf(stderr, "err line\n");
        return 3;
    }
    if (strcmp(scenario, "segv") == 0) {
        int *volatile p = NULL;
        *p = 1;
        return 0;
    }
    if (strcmp(scenario, "sigkill") == 0) {
        raise(SIGKILL);
        return 0;
    }
    if (strncmp(scenario, "sleep:", 6) == 0) {
        mark("ready", "");
        sleep_ms(atoi(scenario + 6));
        return 0;
    }
    if (strcmp(scenario, "env") == 0) {
        char text[4096], fds[256] = "", cwd[PATH_MAX] = "";
        DIR *d = opendir("/proc/self/fd");
        struct dirent *e;
        while (d != NULL && (e = readdir(d)) != NULL) {
            if (e->d_name[0] != '.' && atoi(e->d_name) != dirfd(d)) {
                strcat(fds, fds[0] != '\0' ? "," : "");
                strcat(fds, e->d_name);
            }
        }
        if (d != NULL) {
            closedir(d);
        }
        if (getcwd(cwd, sizeof cwd) == NULL) {
            strcpy(cwd, "?");
        }
        snprintf(text, sizeof text, "project=%s\nrun=%s\nfds=%s\nown_group=%d\ncwd=%s\n",
                 getenv("EV3C_PROJECT_DIR"), getenv("EV3C_RUN_DIR"), fds, getpgid(0) == getpid(), cwd);
        mark("env", text);
        return 0;
    }
    if (strcmp(scenario, "ignore-term") == 0) {
        signal(SIGTERM, SIG_IGN);
        mark("ready", "");
        sleep_forever();
    }
    if (strcmp(scenario, "fork-sleep") == 0) {
        if (fork() == 0) {
            char pid[32];
            snprintf(pid, sizeof pid, "%d", (int)getpid());
            mark("gpid", pid);
            sleep_forever();
        }
        wait_for_mark("gpid");
        mark("ready", "");
        sleep_forever();
    }
    if (strcmp(scenario, "motor-grandchild") == 0) {
        const char *pwm = getenv("EV3C_PWM_DEVICE");
        if (fork() == 0) {
            const unsigned char power[] = { 0xA4, 0x01, 40 };
            char pid[32];
            snprintf(pid, sizeof pid, "%d", (int)getpid());
            for (;;) {
                int fd = open(pwm, O_WRONLY | O_APPEND);
                if (fd >= 0) {
                    ssize_t written = write(fd, power, sizeof power);
                    (void)written;
                    close(fd);
                }
                if (pid[0] != '\0') {
                    mark("gpid", pid);
                    pid[0] = '\0';
                }
                sleep_ms(2);
            }
        }
        wait_for_mark("gpid");
        return 0;
    }
    if (strcmp(scenario, "flood") == 0) {
        char block[500];
        int i;
        memset(block, 'x', sizeof block);
        for (i = 0; i < 10; i++) {
            fwrite(block, 1, sizeof block, stdout);
        }
        memset(block, 'e', sizeof block);
        fwrite(block, 1, sizeof block, stderr);
        fwrite(block, 1, sizeof block, stderr);
        return 0;
    }
    fprintf(stderr, "unknown scenario %s\n", scenario);
    return 99;
}

/* --- the fake brick -------------------------------------------------------- */

typedef struct {
    char dir[300];
    char project[340];
    char runs[360];
    char proc[340];
    char fd_link[360];
    char tok[360];
    char run_marker[360];
    char bak[360];
    char req[360];
    char pwm[340];
    char lock[340];
} brick;

static void make_dir(const char *path)
{
    if (mkdir(path, 0777) != 0) {
        fprintf(stderr, "cannot create %s: %s\n", path, strerror(errno));
        exit(2);
    }
}

static void setup(brick *b, const char *what)
{
    char path[400];
    snprintf(b->dir, sizeof b->dir, "%s/%02d-%s", root, ++case_no, what);
    make_dir(b->dir);
    snprintf(path, sizeof path, "%s/prjs", b->dir);
    make_dir(path);
    snprintf(b->project, sizeof b->project, "%s/prjs/p", b->dir);
    make_dir(b->project);
    snprintf(b->runs, sizeof b->runs, "%s/runs", b->project);
    snprintf(b->tok, sizeof b->tok, "%s/.tok", b->project);
    snprintf(b->run_marker, sizeof b->run_marker, "%s/.run", b->project);
    snprintf(b->bak, sizeof b->bak, "%s/.bak", b->project);
    snprintf(b->req, sizeof b->req, "%s/.req", b->project);
    snprintf(path, sizeof path, "%s/prog", b->project);
    if (symlink(self_path, path) != 0) {
        fprintf(stderr, "cannot link %s\n", path);
        exit(2);
    }
    /* What the launcher does before starting the supervisor. */
    write_file(b->tok, "");

    snprintf(b->proc, sizeof b->proc, "%s/proc", b->dir);
    make_dir(b->proc);
    snprintf(path, sizeof path, "%s/1", b->proc);
    make_dir(path);
    snprintf(path, sizeof path, "%s/1/stat", b->proc);
    write_file(path, "1 (init) S 0 1 1 0 -1\n");
    snprintf(path, sizeof path, "%s/" VM_PID, b->proc);
    make_dir(path);
    snprintf(path, sizeof path, "%s/" VM_PID "/stat", b->proc);
    write_file(path, VM_PID " (lms2012) S 1 " VM_PID " " VM_PID " 0 -1\n");
    snprintf(path, sizeof path, "%s/" VM_PID "/fd", b->proc);
    make_dir(path);
    snprintf(b->fd_link, sizeof b->fd_link, "%s/" VM_PID "/fd/5", b->proc);
    if (symlink(b->tok, b->fd_link) != 0) {
        fprintf(stderr, "cannot link %s\n", b->fd_link);
        exit(2);
    }

    snprintf(b->pwm, sizeof b->pwm, "%s/pwm", b->dir);
    write_file(b->pwm, "");
    snprintf(b->lock, sizeof b->lock, "%s/lock", b->dir);
}

/*
 * Starts the supervisor the way the launcher does. The child also opens a
 * descriptor without close-on-exec, standing in for the VM descriptors the
 * real supervisor inherits. With relative set, it starts from the brick
 * folder with a relative project path ending in '/', as the brick menu does.
 */
static pid_t start(const brick *b, const char *scenario, const char *rule, const char *cap, int relative)
{
    pid_t pid = fork();
    if (pid == 0) {
        char e1[300], e2[400], e3[400], e4[400];
        char *env[] = { e1, e2, e3, e4, "PATH=/usr/bin:/bin", NULL };
        char *argv[] = { sup_path, (char *)b->project, (char *)rule, (char *)cap, NULL };
        snprintf(e1, sizeof e1, "SUPTEST_SCENARIO=%s", scenario);
        snprintf(e2, sizeof e2, "EV3C_PROC_ROOT=%s", b->proc);
        snprintf(e3, sizeof e3, "EV3C_PWM_DEVICE=%s", b->pwm);
        snprintf(e4, sizeof e4, "EV3C_LOCK_FILE=%s", b->lock);
        if (open(b->tok, O_RDONLY) < 0) {
            _exit(98);
        }
        if (relative) {
            if (chdir(b->dir) != 0) {
                _exit(98);
            }
            argv[1] = "prjs/p/";
        }
        execve(sup_path, argv, env);
        _exit(97);
    }
    return pid;
}

/* True while the process has not exited; leaves it for finish() to reap. */
static int still_running(pid_t pid)
{
    siginfo_t info;
    memset(&info, 0, sizeof info);
    return waitid(P_PID, (id_t)pid, &info, WEXITED | WNOHANG | WNOWAIT) == 0 && info.si_pid == 0;
}

/* Waits for the supervisor; returns its exit status, or -1 after a timeout. */
static int finish(pid_t pid, int timeout_ms)
{
    long long deadline = now_ms() + timeout_ms;
    int status;
    while (waitpid(pid, &status, WNOHANG) == 0) {
        if (now_ms() > deadline) {
            kill(pid, SIGKILL);
            waitpid(pid, &status, 0);
            fprintf(stderr, "supervisor %d did not finish within %d ms\n", (int)pid, timeout_ms);
            return -1;
        }
        sleep_ms(2);
    }
    return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}

static void run_file(const brick *b, long n, const char *name, char *path, size_t size)
{
    snprintf(path, size, "%s/%ld/%s", b->runs, n, name);
}

static long read_run_file(const brick *b, long n, const char *name, char *buf, size_t size)
{
    char path[512];
    run_file(b, n, name, path, sizeof path);
    return read_file(path, buf, size);
}

static int wait_run_file(const brick *b, long n, const char *name, int timeout_ms)
{
    char path[512];
    long long deadline = now_ms() + timeout_ms;
    run_file(b, n, name, path, sizeof path);
    while (!exists(path)) {
        if (now_ms() > deadline) {
            return 0;
        }
        sleep_ms(1);
    }
    return 1;
}

/* The motor device's last byte is the stop command, and nothing was written
   after it in the following 100 ms. */
static void check_motor_stop_last(const brick *b)
{
    char text[65536];
    long size = read_file(b->pwm, text, sizeof text);
    CHECK(size > 0 && (unsigned char)text[size - 1] == 0x02, "the motor device should end with the stop command (size %ld)", size);
    sleep_ms(100);
    CHECK(file_size(b->pwm) == size, "nothing may drive the motors after the stop");
}

static void check_markers_gone(const brick *b)
{
    CHECK(!exists(b->run_marker), ".run should be removed");
    CHECK(!exists(b->bak), ".bak should be removed");
}

/* --- cases ----------------------------------------------------------------- */

static void test_exit_code_output_and_launch_id(void)
{
    brick b;
    char res[512], text[256];
    pid_t sup;
    setup(&b, "exit3");
    write_file(b.req, "launch-7\n");
    write_file(b.bak, "");

    sup = start(&b, "exit3", "keep=5", "cap=65536", 0);
    CHECK(finish(sup, 5000) == 0, "supervisor should exit 0");
    read_run_file(&b, 1, "res", res, sizeof res);
    CHECK(has_line(res, "run=1") && has_line(res, "end=program") && has_line(res, "exit=3"), "res:\n%s", res);
    CHECK(has_line(res, "out_truncated=0") && has_line(res, "err_truncated=0"), "res:\n%s", res);
    CHECK(has_line(res, "launch=launch-7"), "res should carry the launch ID:\n%s", res);
    CHECK(!has_key(res, "signal"), "res:\n%s", res);
    read_run_file(&b, 1, "out", text, sizeof text);
    CHECK(strcmp(text, "out line\n") == 0, "out: '%s'", text);
    read_run_file(&b, 1, "err", text, sizeof text);
    CHECK(strcmp(text, "err line\n") == 0, "err: '%s'", text);
    CHECK(!exists(b.req), ".req should be deleted");
    check_markers_gone(&b);
    check_motor_stop_last(&b);
}

static void test_crash_signal_and_retention(void)
{
    brick b;
    char res[512], path[512];
    long n;
    pid_t sup;
    setup(&b, "segv");
    mkdir(b.runs, 0777);
    for (n = 1; n <= 5; n++) {
        snprintf(path, sizeof path, "%s/%ld", b.runs, n);
        mkdir(path, 0777);
    }

    sup = start(&b, "segv", "keep=5", "cap=65536", 0);
    CHECK(finish(sup, 5000) == 0, "supervisor should exit 0");
    read_run_file(&b, 6, "res", res, sizeof res);
    CHECK(has_line(res, "run=6") && has_line(res, "end=program") && has_line(res, "signal=11"), "res:\n%s", res);
    CHECK(!has_key(res, "exit") && !has_key(res, "launch"), "res:\n%s", res);
    snprintf(path, sizeof path, "%s/1", b.runs);
    CHECK(!exists(path), "keep=5: run 1 should be deleted when run 6 starts");
    snprintf(path, sizeof path, "%s/2", b.runs);
    CHECK(exists(path), "keep=5: run 2 should stay");
    check_motor_stop_last(&b);
}

static void test_sigkill(void)
{
    brick b;
    char res[512];
    pid_t sup;
    setup(&b, "sigkill");
    sup = start(&b, "sigkill", "keepmb=1", "cap=4096", 0);
    CHECK(finish(sup, 5000) == 0, "supervisor should exit 0");
    read_run_file(&b, 1, "res", res, sizeof res);
    CHECK(has_line(res, "end=program") && has_line(res, "signal=9"), "res:\n%s", res);
    check_markers_gone(&b);
    check_motor_stop_last(&b);
}

/* Started from the brick menu: relative folder with a trailing '/'. The
   program sees absolute folders, only 0-2 open, its own process group, and its project
   folder as the current folder. */
static void test_program_environment(void)
{
    brick b;
    char text[4096], expected[PATH_MAX + 32], project[PATH_MAX];
    pid_t sup;
    setup(&b, "env");
    sup = start(&b, "env", "keep=5", "cap=65536", 1);
    CHECK(finish(sup, 5000) == 0, "supervisor should exit 0");
    CHECK(realpath(b.project, project) != NULL, "realpath");
    read_run_file(&b, 1, "env", text, sizeof text);
    snprintf(expected, sizeof expected, "project=%s", project);
    CHECK(has_line(text, expected), "program environment:\n%s", text);
    snprintf(expected, sizeof expected, "run=%s/runs/1", project);
    CHECK(has_line(text, expected), "program environment:\n%s", text);
    CHECK(has_line(text, "fds=0,1,2"), "the program should inherit only 0, 1 and 2:\n%s", text);
    CHECK(has_line(text, "own_group=1"), "the program should lead its own process group:\n%s", text);
    snprintf(expected, sizeof expected, "cwd=%s", project);
    CHECK(has_line(text, expected), "the program should start in its project folder:\n%s", text);
}

static void test_slot_end_with_sigterm_ignored(void)
{
    brick b;
    char res[512];
    long long removed, elapsed;
    pid_t sup;
    setup(&b, "slot-end");
    sup = start(&b, "ignore-term", "keep=5", "cap=65536", 0);
    CHECK(wait_run_file(&b, 1, "ready", 5000), "program did not start");
    unlink(b.fd_link);
    removed = now_ms();
    CHECK(finish(sup, 5000) == 0, "supervisor should exit 0");
    elapsed = now_ms() - removed;
    CHECK(elapsed >= 450 && elapsed <= 1200, "slot end to finish took %lld ms, expected about 600", elapsed);
    read_run_file(&b, 1, "res", res, sizeof res);
    CHECK(has_line(res, "end=slot") && has_line(res, "signal=9"), "res:\n%s", res);
    check_markers_gone(&b);
    check_motor_stop_last(&b);
}

static void test_slot_end_kills_whole_group(void)
{
    brick b;
    char res[512], text[64];
    pid_t sup, grandchild;
    setup(&b, "group");
    sup = start(&b, "fork-sleep", "keep=5", "cap=65536", 0);
    CHECK(wait_run_file(&b, 1, "ready", 5000), "program did not start");
    read_run_file(&b, 1, "gpid", text, sizeof text);
    grandchild = (pid_t)atoi(text);
    CHECK(grandchild > 0 && !process_gone(grandchild), "grandchild should be running");
    unlink(b.fd_link);
    CHECK(finish(sup, 5000) == 0, "the supervisor should survive the group kill and exit 0");
    read_run_file(&b, 1, "res", res, sizeof res);
    CHECK(has_line(res, "end=slot") && has_line(res, "signal=15"), "res:\n%s", res);
    CHECK(grandchild > 0 && process_gone(grandchild), "the grandchild should be gone");
    check_motor_stop_last(&b);
}

static void test_program_end_kills_motor_grandchild(void)
{
    brick b;
    char res[512], text[64], path[512];
    pid_t sup, grandchild = 0;
    int gone_at_res;
    setup(&b, "motor-grandchild");
    sup = start(&b, "motor-grandchild", "keep=5", "cap=65536", 0);
    CHECK(wait_run_file(&b, 1, "gpid", 5000), "grandchild did not start");
    read_run_file(&b, 1, "gpid", text, sizeof text);
    grandchild = (pid_t)atoi(text);
    run_file(&b, 1, "res", path, sizeof path);
    while (!exists(path) && still_running(sup)) {
        /* poll as fast as possible, to look at the grandchild right when res appears */
    }
    gone_at_res = grandchild > 0 && process_gone(grandchild);
    CHECK(gone_at_res, "the grandchild should be gone before res is written");
    CHECK(finish(sup, 5000) == 0, "supervisor should exit 0");
    read_run_file(&b, 1, "res", res, sizeof res);
    CHECK(has_line(res, "end=program") && has_line(res, "exit=0"), "res:\n%s", res);
    read_file(b.pwm, text, sizeof text);
    CHECK((unsigned char)text[0] == 0xA4, "the grandchild should have driven the motors");
    check_motor_stop_last(&b);
}

static void test_output_cap(void)
{
    brick b;
    char res[512], path[512];
    pid_t sup;
    setup(&b, "cap");
    sup = start(&b, "flood", "keep=5", "cap=1000", 0);
    CHECK(finish(sup, 5000) == 0, "supervisor should exit 0");
    read_run_file(&b, 1, "res", res, sizeof res);
    CHECK(has_line(res, "out_truncated=1") && has_line(res, "err_truncated=0"), "res:\n%s", res);
    run_file(&b, 1, "out", path, sizeof path);
    CHECK(file_size(path) == 1000, "out should hold exactly the cap, holds %ld", file_size(path));
    run_file(&b, 1, "err", path, sizeof path);
    CHECK(file_size(path) == 1000, "err holds exactly the cap without being truncated, holds %ld", file_size(path));
}

static void test_slot_ended_before_start(void)
{
    brick b;
    char res[512], path[512];
    pid_t sup;
    setup(&b, "before-start");
    unlink(b.fd_link);
    write_file(b.req, "launch-9");
    sup = start(&b, "env", "keep=5", "cap=65536", 0);
    CHECK(finish(sup, 5000) == 0, "supervisor should exit 0");
    read_run_file(&b, 1, "res", res, sizeof res);
    CHECK(has_line(res, "end=before-start") && has_line(res, "launch=launch-9"), "res:\n%s", res);
    CHECK(!has_key(res, "exit") && !has_key(res, "signal"), "res:\n%s", res);
    run_file(&b, 1, "env", path, sizeof path);
    CHECK(!exists(path), "no program may start");
    check_markers_gone(&b);
}

static void test_recreated_token_is_slot_end(void)
{
    brick b;
    char res[512];
    pid_t sup;
    int old;
    setup(&b, "recreated-tok");
    sup = start(&b, "sleep:20000", "keep=5", "cap=65536", 0);
    CHECK(wait_run_file(&b, 1, "ready", 5000), "program did not start");
    /* Holding the old file open keeps its inode from being reused. */
    old = open(b.tok, O_RDONLY);
    unlink(b.tok);
    write_file(b.tok, "");
    CHECK(finish(sup, 5000) == 0, "supervisor should exit 0");
    close(old);
    read_run_file(&b, 1, "res", res, sizeof res);
    CHECK(has_line(res, "end=slot") && has_line(res, "signal=15"), "res:\n%s", res);
}

static int hold_lock(const brick *b)
{
    int fd = open(b->lock, O_RDWR | O_CREAT, 0666);
    if (fd < 0 || flock(fd, LOCK_EX) != 0) {
        fprintf(stderr, "cannot lock %s\n", b->lock);
        exit(2);
    }
    return fd;
}

static void test_waits_for_lock_and_marker_holds_pid(void)
{
    brick b;
    char res[512], text[64], expected[32];
    pid_t sup;
    int lock;
    setup(&b, "lock-wait");
    lock = hold_lock(&b);
    sup = start(&b, "sleep:300", "keep=5", "cap=65536", 0);
    sleep_ms(1000);
    CHECK(!exists(b.run_marker), "no .run while another run holds the lock");
    CHECK(!exists(b.runs), "no run folder while another run holds the lock");
    close(lock);
    while (!exists(b.run_marker) && still_running(sup)) {
        /* catch .run the moment it appears */
    }
    read_file(b.run_marker, text, sizeof text);
    snprintf(expected, sizeof expected, "%d", (int)sup);
    CHECK(strcmp(text, expected) == 0, ".run holds '%s', expected '%s' without a newline", text, expected);
    CHECK(finish(sup, 5000) == 0, "supervisor should exit 0 after the lock came free");
    read_run_file(&b, 1, "res", res, sizeof res);
    CHECK(has_line(res, "end=program") && has_line(res, "exit=0"), "res:\n%s", res);
}

static void test_gives_up_on_lock(void)
{
    brick b;
    long long started, elapsed;
    pid_t sup;
    int lock, status;
    setup(&b, "lock-timeout");
    lock = hold_lock(&b);
    started = now_ms();
    sup = start(&b, "env", "keep=5", "cap=65536", 0);
    status = finish(sup, 8000);
    elapsed = now_ms() - started;
    close(lock);
    CHECK(status == 3, "supervisor should give up with exit 3, got %d", status);
    CHECK(elapsed >= 2900 && elapsed < 5000, "gave up after %lld ms, expected about 3000", elapsed);
    CHECK(!exists(b.runs), "no run folder without the lock");
    CHECK(!exists(b.run_marker), "no .run without the lock");
    CHECK(file_size(b.pwm) == 0, "no motor stop without the lock");
}

static void test_bad_arguments(void)
{
    static const char *const bad[][2] = {
        { "keep=0", "cap=100" }, { "keep=-1", "cap=100" }, { "keepmb=x", "cap=100" },
        { "keep=5", "cap=0" }, { "keep=5", "cap=2147483648" }, { "retain=5", "cap=100" },
    };
    brick b;
    size_t i;
    setup(&b, "bad-args");
    for (i = 0; i < sizeof bad / sizeof bad[0]; i++) {
        pid_t sup = start(&b, "env", bad[i][0], bad[i][1], 0);
        CHECK(finish(sup, 5000) == 2, "%s %s should be refused with exit 2", bad[i][0], bad[i][1]);
    }
    CHECK(!exists(b.runs) && !exists(b.run_marker), "refused arguments must leave nothing behind");
}

int main(int argc, char **argv)
{
    const char *scenario = getenv("SUPTEST_SCENARIO");
    const char *tmp = getenv("TMPDIR");
    if (scenario != NULL && strcmp(argv[0], "prog") == 0) {
        return play(scenario);
    }
    if (argc != 2 || realpath(argv[1], sup_path) == NULL) {
        fprintf(stderr, "usage: test_supervisor <supervisor executable>\n");
        return 2;
    }
    if (realpath("/proc/self/exe", self_path) == NULL) {
        fprintf(stderr, "cannot find this test binary\n");
        return 2;
    }
    snprintf(root, sizeof root, "%s/test_supervisor.XXXXXX", tmp != NULL && tmp[0] != '\0' ? tmp : "/tmp");
    if (mkdtemp(root) == NULL) {
        fprintf(stderr, "cannot create a temporary folder: %s\n", strerror(errno));
        return 2;
    }

    test_exit_code_output_and_launch_id();
    test_crash_signal_and_retention();
    test_sigkill();
    test_program_environment();
    test_slot_end_with_sigterm_ignored();
    test_slot_end_kills_whole_group();
    test_program_end_kills_motor_grandchild();
    test_output_cap();
    test_slot_ended_before_start();
    test_recreated_token_is_slot_end();
    test_waits_for_lock_and_marker_holds_pid();
    test_gives_up_on_lock();
    test_bad_arguments();

    if (failures > 0) {
        fprintf(stderr, "test_supervisor: %d check(s) failed; files kept in %s\n", failures, root);
        return 1;
    }
    {
        char command[300];
        snprintf(command, sizeof command, "rm -rf '%s'", root);
        if (system(command) != 0) {
            fprintf(stderr, "could not remove %s\n", root);
        }
    }
    printf("test_supervisor: all checks passed\n");
    return 0;
}
