/*
 * ev3lib: files a program hands back to its author. Each marked file's full
 * path goes on its own line of the run folder's list, "ret", written with one
 * append per path, so the list is complete up to the last mark even when the
 * program crashes. EVƎ C reads the list after the run and downloads the files.
 */
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

#include "ev3.h"
#include "ev3_core_internal.h"

#define RETURN_LIST "ret"

static int has_line_break(const char *path)
{
    return strchr(path, '\n') != NULL || strchr(path, '\r') != NULL;
}

/*
 * The full path of `path` without resolving links: the path itself when it is
 * absolute, else the current folder joined to it. Links are kept so the path
 * reads as the program wrote it.
 */
static int full_path(const char *path, char *full, size_t size)
{
    char cwd[PATH_MAX];
    int written;

    if (path[0] == '/') {
        written = snprintf(full, size, "%s", path);
    } else {
        if (getcwd(cwd, sizeof cwd) == NULL) {
            return -1;
        }
        written = snprintf(full, size, "%s/%s", cwd, path);
    }
    if (written < 0 || (size_t)written >= size) {
        errno = ENAMETOOLONG;
        return -1;
    }
    return 0;
}

/* Appends the full path of an existing file to the run's list. */
static int record(const char *path)
{
    const char *run_dir = ev3_core_run_dir();
    char list[PATH_MAX];
    char line[PATH_MAX + 1];
    struct stat info;
    ssize_t written;
    int length;
    int fd;
    int saved;

    if (run_dir == NULL) {
        errno = ENOENT;
        return -1;
    }
    if (stat(path, &info) != 0 || full_path(path, line, sizeof line - 1) != 0) {
        return -1;
    }
    length = (int)strlen(line);
    line[length++] = '\n';
    if (snprintf(list, sizeof list, "%s/%s", run_dir, RETURN_LIST) >= (int)sizeof list) {
        errno = ENAMETOOLONG;
        return -1;
    }
    fd = open(list, O_WRONLY | O_APPEND | O_CREAT | O_CLOEXEC, 0644);
    if (fd < 0) {
        return -1;
    }
    /* One write per line: an append of this size lands whole. */
    written = write(fd, line, (size_t)length);
    saved = errno;
    close(fd);
    if (written != length) {
        errno = written < 0 ? saved : EIO;
        return -1;
    }
    return 0;
}

static Ev3Result check_path(const char *path)
{
    if (path == NULL) {
        return ev3_fail(EV3_ERR_PARAM, "the path points nowhere (NULL)");
    }
    if (has_line_break(path)) {
        return ev3_fail(EV3_ERR_PARAM, "the path \"%s\" contains a line break", path);
    }
    return EV3_OK;
}

Ev3Result ev3_return_try_fopen(const char *path, const char *mode, FILE **opened)
{
    FILE *file;
    Ev3Result result = check_path(path);

    if (result == EV3_OK && (mode == NULL || opened == NULL)) {
        result = ev3_fail(EV3_ERR_PARAM, "the mode or the file points nowhere (NULL)");
    }
    if (result != EV3_OK) {
        return result;
    }
    file = fopen(path, mode);
    if (file == NULL) {
        return ev3_fail(EV3_ERR_IO, "%s cannot be opened: %s", path, strerror(errno));
    }
    if (ev3_core_run_dir() != NULL && record(path) != 0) {
        /* Marked for return but not on the list: say so rather than lose it in silence. */
        int saved = errno;
        fclose(file);
        return ev3_fail(EV3_ERR_IO, "%s cannot be marked for return: %s", path, strerror(saved));
    }
    *opened = file;
    return EV3_OK;
}

Ev3Result ev3_return_try_file(const char *path)
{
    Ev3Result result = check_path(path);

    if (result != EV3_OK) {
        return result;
    }
    if (ev3_core_run_dir() == NULL) {
        return ev3_fail(EV3_ERR_NOT_SUPPORTED,
                        "files can be returned only from a run started by EVƎ C (EV3C_RUN_DIR is not set)");
    }
    if (record(path) != 0) {
        return ev3_fail(EV3_ERR_IO, "%s cannot be marked for return: %s", path, strerror(errno));
    }
    return EV3_OK;
}

FILE *ev3_return_fopen_at_(EV3_AT, const char *path, const char *mode)
{
    FILE *file = NULL;
    EV3_MUST("ev3_return_fopen", ev3_return_try_fopen(path, mode, &file));
    return file;
}

void ev3_return_file_at_(EV3_AT, const char *path)
{
    EV3_MUST("ev3_return_file", ev3_return_try_file(path));
}
