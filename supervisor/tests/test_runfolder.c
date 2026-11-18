/*
 * Host tests for the supervisor's run folders: run numbers, retention, the
 * run result and its atomic completion record, the launch ID, and the PID
 * marker. Built together with supervisor/src/runfolder.c.
 *
 * Needs POSIX; run it on Linux or macOS. Works in a temporary folder under
 * TMPDIR (default /tmp).
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

#include "runfolder.h"

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

/* --- helpers -------------------------------------------------------------- */

static void path_of(char *out, size_t size, const char *dir, const char *name)
{
    snprintf(out, size, "%s/%s", dir, name);
}

/* A fresh folder for one test case. */
static void fresh_dir(char *out, size_t size, const char *what)
{
    snprintf(out, size, "%s/%02d-%s", root, ++case_no, what);
    if (mkdir(out, 0777) != 0) {
        fprintf(stderr, "cannot create %s: %s\n", out, strerror(errno));
        exit(2);
    }
}

static void write_file(const char *path, const char *text, size_t length)
{
    FILE *f = fopen(path, "wb");
    if (f == NULL || fwrite(text, 1, length, f) != length || fclose(f) != 0) {
        fprintf(stderr, "cannot write %s\n", path);
        exit(2);
    }
}

/* Reads a whole small file; returns its length, or -1 if it is missing. */
static long read_file(const char *path, char *buf, size_t size)
{
    FILE *f = fopen(path, "rb");
    size_t n;
    if (f == NULL) {
        return -1;
    }
    n = fread(buf, 1, size - 1, f);
    buf[n] = '\0';
    fclose(f);
    return (long)n;
}

static int exists(const char *path)
{
    struct stat st;
    return lstat(path, &st) == 0;
}

/* Creates <runs>/<n>/out with the given number of bytes. */
static void make_run(const char *runs, long n, size_t bytes)
{
    char dir[400], file[420];
    char *data = calloc(1, bytes + 1);
    snprintf(dir, sizeof dir, "%s/%ld", runs, n);
    if (mkdir(dir, 0777) != 0 || data == NULL) {
        fprintf(stderr, "cannot create %s\n", dir);
        exit(2);
    }
    memset(data, 'x', bytes);
    path_of(file, sizeof file, dir, "out");
    write_file(file, data, bytes);
    free(data);
}

static int run_exists(const char *runs, long n)
{
    char dir[400];
    snprintf(dir, sizeof dir, "%s/%ld", runs, n);
    return exists(dir);
}

static long counter(const char *runs)
{
    char path[400], text[64];
    path_of(path, sizeof path, runs, "next");
    if (read_file(path, text, sizeof text) < 0) {
        return -1;
    }
    return strtol(text, NULL, 10);
}

static rf_retention keep_runs(int n)
{
    rf_retention rule = { n, 0 };
    return rule;
}

/* --- run numbers and retention -------------------------------------------- */

static void test_keep_last_five(void)
{
    char project[300], runs[340], dir[400];
    rf_retention rule = keep_runs(5);
    long number = 0;
    long n;
    fresh_dir(project, sizeof project, "keep5");
    path_of(runs, sizeof runs, project, "runs");
    mkdir(runs, 0777);
    for (n = 1; n <= 5; n++) {
        make_run(runs, n, 10);
    }
    write_file(strcat(strcpy(dir, runs), "/next"), "6", 1);

    CHECK(rf_start_run(runs, &rule, 0, dir, sizeof dir, &number) == 0, "start run failed: %s", strerror(errno));
    CHECK(number == 6, "run number %ld, expected 6", number);
    CHECK(!run_exists(runs, 1), "run 1 should be deleted");
    for (n = 2; n <= 6; n++) {
        CHECK(run_exists(runs, n), "run %ld should exist", n);
    }
    CHECK(strstr(dir, "/runs/6") != NULL, "run folder path %s", dir);
    CHECK(counter(runs) == 7, "counter %ld, expected 7", counter(runs));
}

static void test_counter_survives_deleted_folders(void)
{
    char project[300], runs[340], dir[400], path[400];
    rf_retention rule = keep_runs(5);
    long number = 0;
    fresh_dir(project, sizeof project, "counter12");
    path_of(runs, sizeof runs, project, "runs");
    mkdir(runs, 0777);
    path_of(path, sizeof path, runs, "next");
    write_file(path, "12", 2);

    CHECK(rf_start_run(runs, &rule, 0, dir, sizeof dir, &number) == 0, "start run failed: %s", strerror(errno));
    CHECK(number == 12, "run number %ld, expected 12", number);
    CHECK(run_exists(runs, 12), "run 12 should exist");
    CHECK(counter(runs) == 13, "counter %ld, expected 13", counter(runs));
}

static void test_first_run_creates_runs_folder(void)
{
    char project[300], runs[340], dir[400];
    rf_retention rule = keep_runs(5);
    long number = 0;
    fresh_dir(project, sizeof project, "first");
    path_of(runs, sizeof runs, project, "runs");

    CHECK(rf_start_run(runs, &rule, 0, dir, sizeof dir, &number) == 0, "start run failed: %s", strerror(errno));
    CHECK(number == 1, "run number %ld, expected 1", number);
    CHECK(run_exists(runs, 1), "run 1 should exist");
    CHECK(counter(runs) == 2, "counter %ld, expected 2", counter(runs));
}

/* Without a counter the number continues after the highest folder, compared
   as numbers (10 after 9), and retention deletes the numerically oldest. */
static void test_missing_counter_and_numeric_order(void)
{
    char project[300], runs[340], dir[400];
    rf_retention rule = keep_runs(2);
    long number = 0;
    fresh_dir(project, sizeof project, "numeric");
    path_of(runs, sizeof runs, project, "runs");
    mkdir(runs, 0777);
    make_run(runs, 3, 1);
    make_run(runs, 9, 1);
    make_run(runs, 10, 1);

    CHECK(rf_start_run(runs, &rule, 0, dir, sizeof dir, &number) == 0, "start run failed: %s", strerror(errno));
    CHECK(number == 11, "run number %ld, expected 11", number);
    CHECK(!run_exists(runs, 3) && !run_exists(runs, 9), "runs 3 and 9 should be deleted");
    CHECK(run_exists(runs, 10) && run_exists(runs, 11), "runs 10 and 11 should exist");
}

/* A counter behind the existing folders never makes a run reuse a folder. */
static void test_counter_behind_folders(void)
{
    char project[300], runs[340], dir[400], path[400];
    rf_retention rule = keep_runs(5);
    long number = 0;
    fresh_dir(project, sizeof project, "behind");
    path_of(runs, sizeof runs, project, "runs");
    mkdir(runs, 0777);
    make_run(runs, 4, 1);
    path_of(path, sizeof path, runs, "next");
    write_file(path, "2", 1);

    CHECK(rf_start_run(runs, &rule, 0, dir, sizeof dir, &number) == 0, "start run failed: %s", strerror(errno));
    CHECK(number == 5, "run number %ld, expected 5", number);
}

static void test_keep_megabytes(void)
{
    char project[300], runs[340], dir[400];
    rf_retention rule = { 0, 1024 * 1024 };
    long number = 0;
    fresh_dir(project, sizeof project, "keepmb");
    path_of(runs, sizeof runs, project, "runs");
    mkdir(runs, 0777);
    make_run(runs, 1, 300 * 1024);
    make_run(runs, 2, 300 * 1024);
    make_run(runs, 3, 300 * 1024);

    /* Room for two 256 KiB streams of the new run: 900 KiB + 512 KiB does not
       fit 1 MiB, 600 KiB + 512 KiB neither, 300 KiB + 512 KiB does. */
    CHECK(rf_start_run(runs, &rule, 512 * 1024, dir, sizeof dir, &number) == 0, "start run failed: %s", strerror(errno));
    CHECK(number == 4, "run number %ld, expected 4", number);
    CHECK(!run_exists(runs, 1) && !run_exists(runs, 2), "runs 1 and 2 should be deleted");
    CHECK(run_exists(runs, 3) && run_exists(runs, 4), "runs 3 and 4 should exist");
}

/* Retention deletes whole run folders, subfolders included, and nothing else:
   not other names in runs/, and not what a symlink points to. */
static void test_retention_touches_only_run_folders(void)
{
    char project[300], runs[340], dir[400], path[400], outside[340], keep[400];
    rf_retention rule = keep_runs(1);
    long number = 0;
    fresh_dir(project, sizeof project, "only-runs");
    path_of(runs, sizeof runs, project, "runs");
    path_of(outside, sizeof outside, project, "outside");
    mkdir(runs, 0777);
    mkdir(outside, 0777);
    path_of(keep, sizeof keep, outside, "keep");
    write_file(keep, "k", 1);
    make_run(runs, 1, 5);
    snprintf(path, sizeof path, "%s/1/sub", runs);
    mkdir(path, 0777);
    snprintf(path, sizeof path, "%s/1/sub/deep", runs);
    write_file(path, "d", 1);
    snprintf(path, sizeof path, "%s/1/link", runs);
    CHECK(symlink(outside, path) == 0, "symlink: %s", strerror(errno));
    path_of(path, sizeof path, runs, "notes");
    write_file(path, "n", 1);
    path_of(path, sizeof path, runs, "2");
    CHECK(symlink(outside, path) == 0, "symlink: %s", strerror(errno));

    CHECK(rf_start_run(runs, &rule, 0, dir, sizeof dir, &number) == 0, "start run failed: %s", strerror(errno));
    CHECK(number == 3, "run number %ld, expected 3", number);
    CHECK(!run_exists(runs, 1), "run 1 should be deleted with its subfolder");
    CHECK(exists(keep), "a file reached only through a symlink must survive");
    path_of(path, sizeof path, runs, "notes");
    CHECK(exists(path), "runs/notes must survive");
    path_of(path, sizeof path, runs, "2");
    CHECK(exists(path), "a symlink named like a run is not a run folder and must survive");
}

/* --- result ---------------------------------------------------------------- */

static void test_result_format(void)
{
    char text[512];
    rf_result exited = { 7, RF_END_PROGRAM, 1, 3 << 8, 0, 1, "abc-123" };
    rf_result killed = { 8, RF_END_SLOT, 1, 9, 1, 0, NULL };
    rf_result before = { 9, RF_END_SLOT_BEFORE, 0, 0, 0, 0, NULL };

    CHECK(rf_format_result(&exited, text, sizeof text) > 0, "format failed");
    CHECK(strcmp(text, "run=7\nend=program\nexit=3\nout_truncated=0\nerr_truncated=1\nlaunch=abc-123\n") == 0,
          "exited result:\n%s", text);
    CHECK(rf_format_result(&killed, text, sizeof text) > 0, "format failed");
    CHECK(strcmp(text, "run=8\nend=slot\nsignal=9\nout_truncated=1\nerr_truncated=0\n") == 0,
          "killed result:\n%s", text);
    CHECK(rf_format_result(&before, text, sizeof text) > 0, "format failed");
    CHECK(strcmp(text, "run=9\nend=before-start\nout_truncated=0\nerr_truncated=0\n") == 0,
          "slot-ended-before-start result:\n%s", text);
    CHECK(rf_format_result(&exited, text, 10) == -1, "a result that does not fit must fail");
}

static void test_result_is_atomic(void)
{
    char dir[300], res[340], tmp[340], text[512];
    rf_result result = { 1, RF_END_PROGRAM, 1, 0, 0, 0, NULL };
    fresh_dir(dir, sizeof dir, "res");
    path_of(res, sizeof res, dir, "res");
    path_of(tmp, sizeof tmp, dir, "res.tmp");

    CHECK(rf_write_result_temp(dir, &result) == 0, "temp write failed: %s", strerror(errno));
    CHECK(!exists(res), "res must not exist before the commit");
    CHECK(exists(tmp), "res.tmp should exist");
    CHECK(rf_commit_result(dir) == 0, "commit failed: %s", strerror(errno));
    CHECK(read_file(res, text, sizeof text) > 0 && strncmp(text, "run=1\n", 6) == 0, "res content:\n%s", text);
    CHECK(!exists(tmp), "res.tmp should be gone after the commit");
}

/* A supervisor that dies in the middle of writing the result leaves only the
   temporary file. The child is killed by SIGXFSZ after the first bytes. */
static void test_interrupted_result_write(void)
{
    char dir[300], res[340], tmp[340];
    rf_result result = { 1, RF_END_PROGRAM, 1, 0, 0, 0, "launch" };
    pid_t child;
    int status = 0;
    fresh_dir(dir, sizeof dir, "res-cut");
    path_of(res, sizeof res, dir, "res");
    path_of(tmp, sizeof tmp, dir, "res.tmp");

    child = fork();
    if (child == 0) {
        /* Limit file size to 8 bytes; growing res.tmp past it raises SIGXFSZ. */
        struct rlimit limit = { 8, 8 };
        setrlimit(RLIMIT_FSIZE, &limit);
        rf_write_result_temp(dir, &result);
        rf_commit_result(dir);
        _exit(0);
    }
    waitpid(child, &status, 0);
    CHECK(WIFSIGNALED(status) && WTERMSIG(status) == SIGXFSZ, "the writer should die on SIGXFSZ (status %d)", status);
    CHECK(exists(tmp), "the partial res.tmp should exist");
    CHECK(!exists(res), "res must not exist after an interrupted write");
}

/* --- project-folder markers ------------------------------------------------ */

static void test_launch_id(void)
{
    char project[300], req[340], id[RF_LAUNCH_ID_MAX + 1];
    fresh_dir(project, sizeof project, "req");
    path_of(req, sizeof req, project, ".req");
    write_file(req, "4f2a-launch\r\nignored\n", 21);

    CHECK(rf_take_launch_id(project, id, sizeof id) == 1, "launch ID should be found");
    CHECK(strcmp(id, "4f2a-launch") == 0, "launch ID '%s'", id);
    CHECK(!exists(req), ".req should be deleted");
    CHECK(rf_take_launch_id(project, id, sizeof id) == 0, "no .req means no launch ID");
}

static void test_pid_marker(void)
{
    char project[300], run[340], text[64];
    fresh_dir(project, sizeof project, "run-marker");
    path_of(run, sizeof run, project, ".run");

    CHECK(rf_write_pid_marker(project, 4321) == 0, "marker write failed: %s", strerror(errno));
    CHECK(read_file(run, text, sizeof text) == 4 && strcmp(text, "4321") == 0, ".run holds '%s'", text);
}

int main(void)
{
    const char *tmp = getenv("TMPDIR");
    snprintf(root, sizeof root, "%s/test_runfolder.XXXXXX", tmp != NULL && tmp[0] != '\0' ? tmp : "/tmp");
    if (mkdtemp(root) == NULL) {
        fprintf(stderr, "cannot create a temporary folder: %s\n", strerror(errno));
        return 2;
    }

    test_keep_last_five();
    test_counter_survives_deleted_folders();
    test_first_run_creates_runs_folder();
    test_missing_counter_and_numeric_order();
    test_counter_behind_folders();
    test_keep_megabytes();
    test_retention_touches_only_run_folders();
    test_result_format();
    test_result_is_atomic();
    test_interrupted_result_write();
    test_launch_id();
    test_pid_marker();

    if (failures > 0) {
        fprintf(stderr, "test_runfolder: %d check(s) failed; files kept in %s\n", failures, root);
        return 1;
    }
    {
        char command[300];
        snprintf(command, sizeof command, "rm -rf '%s'", root);
        if (system(command) != 0) {
            fprintf(stderr, "could not remove %s\n", root);
        }
    }
    printf("test_runfolder: all checks passed\n");
    return 0;
}
