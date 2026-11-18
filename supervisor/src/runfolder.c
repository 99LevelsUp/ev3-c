/*
 * Run folders, the run result, and the markers in the project folder. See
 * runfolder.h for the layout.
 */
#define _GNU_SOURCE
#include "runfolder.h"

#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <ftw.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

/* --- small file helpers ---------------------------------------------------- */

/* Writes all of text, continuing after short writes. */
static int write_all(int fd, const char *text, size_t length)
{
    while (length > 0) {
        ssize_t n = write(fd, text, length);
        if (n < 0) {
            if (errno == EINTR) {
                continue;
            }
            return -1;
        }
        text += n;
        length -= (size_t)n;
    }
    return 0;
}

/* Writes text to path through <path>.tmp and a rename, so readers never see
   a half-written file. */
static int replace_file(const char *path, const char *text)
{
    char tmp[4096];
    size_t length = strlen(text);
    int fd;
    if (snprintf(tmp, sizeof tmp, "%s.tmp", path) >= (int)sizeof tmp) {
        errno = ENAMETOOLONG;
        return -1;
    }
    fd = open(tmp, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0666);
    if (fd < 0) {
        return -1;
    }
    if (write_all(fd, text, length) != 0) {
        int saved = errno;
        close(fd);
        errno = saved;
        return -1;
    }
    if (close(fd) != 0) {
        return -1;
    }
    return rename(tmp, path);
}

/* A run folder name: 1 to 9 decimal digits, value above zero. Returns the
   number, or 0 for any other name. */
static long run_number(const char *name)
{
    size_t length = strlen(name);
    size_t i;
    if (length == 0 || length > 9) {
        return 0;
    }
    for (i = 0; i < length; i++) {
        if (name[i] < '0' || name[i] > '9') {
            return 0;
        }
    }
    return strtol(name, NULL, 10);
}

/* --- markers --------------------------------------------------------------- */

int rf_take_launch_id(const char *project, char *id, size_t size)
{
    char path[4096], line[RF_LAUNCH_ID_MAX + 1];
    ssize_t n;
    size_t end = 0;
    int fd;
    snprintf(path, sizeof path, "%s/.req", project);
    fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd < 0) {
        return 0;
    }
    n = read(fd, line, sizeof line - 1);
    close(fd);
    unlink(path);
    if (n > 0) {
        while (end < (size_t)n && line[end] != '\n' && line[end] != '\r') {
            end++;
        }
    }
    line[end] = '\0';
    snprintf(id, size, "%s", line);
    return 1;
}

int rf_write_pid_marker(const char *project, pid_t pid)
{
    char path[4096], text[32];
    snprintf(path, sizeof path, "%s/.run", project);
    snprintf(text, sizeof text, "%d", (int)pid);
    return replace_file(path, text);
}

/* --- run folders ----------------------------------------------------------- */

typedef struct {
    long number;
    long long bytes;
} run_entry;

static long long tree_bytes;

static int add_size(const char *path, const struct stat *st, int type, struct FTW *ftw)
{
    (void)path;
    (void)ftw;
    if (type == FTW_F) {
        tree_bytes += (long long)st->st_size;
    }
    return 0;
}

static int remove_entry(const char *path, const struct stat *st, int type, struct FTW *ftw)
{
    (void)st;
    (void)type;
    (void)ftw;
    remove(path);
    return 0;
}

static int by_number(const void *a, const void *b)
{
    long x = ((const run_entry *)a)->number;
    long y = ((const run_entry *)b)->number;
    return (x > y) - (x < y);
}

/* Lists the run folders in runs, oldest first, and the highest number used by
   any entry that is named like a run. Only real directories are run folders;
   a symlink is never followed. */
static int list_runs(const char *runs, run_entry **list, size_t *count, long *highest)
{
    DIR *d = opendir(runs);
    struct dirent *e;
    size_t capacity = 0;
    *list = NULL;
    *count = 0;
    *highest = 0;
    if (d == NULL) {
        return -1;
    }
    while ((e = readdir(d)) != NULL) {
        char path[4096];
        struct stat st;
        long n = run_number(e->d_name);
        if (n <= 0) {
            continue;
        }
        if (n > *highest) {
            *highest = n;
        }
        snprintf(path, sizeof path, "%s/%s", runs, e->d_name);
        if (lstat(path, &st) != 0 || !S_ISDIR(st.st_mode)) {
            continue;
        }
        if (*count == capacity) {
            size_t grown = capacity == 0 ? 16 : capacity * 2;
            run_entry *bigger = realloc(*list, grown * sizeof **list);
            if (bigger == NULL) {
                closedir(d);
                free(*list);
                *list = NULL;
                return -1;
            }
            *list = bigger;
            capacity = grown;
        }
        tree_bytes = 0;
        nftw(path, add_size, 8, FTW_PHYS);
        (*list)[*count].number = n;
        (*list)[*count].bytes = tree_bytes;
        (*count)++;
    }
    closedir(d);
    if (*count > 1) {
        qsort(*list, *count, sizeof **list, by_number);
    }
    return 0;
}

static void delete_run(const char *runs, long number)
{
    char path[4096];
    snprintf(path, sizeof path, "%s/%ld", runs, number);
    nftw(path, remove_entry, 8, FTW_DEPTH | FTW_PHYS);
}

int rf_start_run(const char *runs, const rf_retention *rule, long long reserve,
                 char *dir, size_t size, long *number)
{
    char path[4096], text[32];
    run_entry *list;
    size_t count, first = 0;
    long highest, next = 0;
    long long total = 0;
    int fd;
    size_t i;

    if (mkdir(runs, 0777) != 0 && errno != EEXIST) {
        return -1;
    }
    if (list_runs(runs, &list, &count, &highest) != 0) {
        return -1;
    }

    /* The counter decides; a folder above it (counter lost or rolled back)
       moves the number past that folder. */
    snprintf(path, sizeof path, "%s/next", runs);
    fd = open(path, O_RDONLY | O_CLOEXEC);
    if (fd >= 0) {
        ssize_t n = read(fd, text, sizeof text - 1);
        close(fd);
        text[n > 0 ? n : 0] = '\0';
        next = strtol(text, NULL, 10);
    }
    if (next <= highest) {
        next = highest + 1;
    }
    if (next < 1) {
        next = 1;
    }
    snprintf(text, sizeof text, "%ld", next + 1);
    if (replace_file(path, text) != 0) {
        free(list);
        return -1;
    }

    /* Retention, oldest first, before the new folder exists. */
    for (i = 0; i < count; i++) {
        total += list[i].bytes;
    }
    while (first < count) {
        int too_many = rule->keep_runs > 0 && (long long)(count - first) >= rule->keep_runs;
        int too_big = rule->keep_bytes > 0 && total + reserve > rule->keep_bytes;
        if (!too_many && !too_big) {
            break;
        }
        delete_run(runs, list[first].number);
        total -= list[first].bytes;
        first++;
    }
    free(list);

    if (snprintf(dir, size, "%s/%ld", runs, next) >= (int)size) {
        errno = ENAMETOOLONG;
        return -1;
    }
    if (mkdir(dir, 0777) != 0) {
        return -1;
    }
    *number = next;
    return 0;
}

/* --- result ---------------------------------------------------------------- */

int rf_format_result(const rf_result *result, char *text, size_t size)
{
    static const char *const ends[] = { "program", "slot", "before-start" };
    size_t used = 0;
    int n;

#define APPEND(...)                                                        \
    do {                                                                   \
        n = snprintf(text + used, size - used, __VA_ARGS__);               \
        if (n < 0 || (size_t)n >= size - used) {                           \
            return -1;                                                     \
        }                                                                  \
        used += (size_t)n;                                                 \
    } while (0)

    if (size == 0) {
        return -1;
    }
    APPEND("run=%ld\nend=%s\n", result->number, ends[result->end]);
    if (result->have_status) {
        if (WIFEXITED(result->status)) {
            APPEND("exit=%d\n", WEXITSTATUS(result->status));
        } else if (WIFSIGNALED(result->status)) {
            APPEND("signal=%d\n", WTERMSIG(result->status));
        }
    }
    APPEND("out_truncated=%d\nerr_truncated=%d\n", result->out_truncated ? 1 : 0, result->err_truncated ? 1 : 0);
    if (result->launch_id != NULL) {
        APPEND("launch=%s\n", result->launch_id);
    }
#undef APPEND
    return (int)used;
}

int rf_write_result_temp(const char *dir, const rf_result *result)
{
    char path[4096], text[512];
    int length = rf_format_result(result, text, sizeof text);
    int fd;
    if (length < 0) {
        errno = EOVERFLOW;
        return -1;
    }
    snprintf(path, sizeof path, "%s/res.tmp", dir);
    fd = open(path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0666);
    if (fd < 0) {
        return -1;
    }
    if (write_all(fd, text, (size_t)length) != 0) {
        int saved = errno;
        close(fd);
        errno = saved;
        return -1;
    }
    return close(fd);
}

int rf_commit_result(const char *dir)
{
    char from[4096], to[4096];
    snprintf(from, sizeof from, "%s/res.tmp", dir);
    snprintf(to, sizeof to, "%s/res", dir);
    return rename(from, to);
}
