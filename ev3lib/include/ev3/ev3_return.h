/*
 * ev3lib: files a program hands back to its author.
 *
 * EVƎ C downloads the files a run marks into the author's workspace after the
 * run ends. Marking writes the file's full path to the run folder's list at
 * once, so the list survives a crash; what the file holds is read when the run
 * has ended. A relative path is taken from the current folder, which is the
 * project folder when EVƎ C's supervisor starts the program. Paths may not
 * contain a line break.
 *
 * Each function comes as a try variant, which returns an Ev3Result, and as a
 * plain variant, which stops the program on an error (see ev3_error.h).
 */
#ifndef EV3_RETURN_H
#define EV3_RETURN_H

#include <stdio.h>

#include "ev3_error.h"

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Opens a file like fopen() and marks it for return. Outside a run (no
 * EV3C_RUN_DIR) the file opens and nothing is marked.
 */
Ev3Result ev3_return_try_fopen(const char *path, const char *mode, FILE **file);
FILE *ev3_return_fopen_at_(const char *file, int line, const char *path, const char *mode);
#define ev3_return_fopen(path, mode) ev3_return_fopen_at_(__FILE__, __LINE__, (path), (mode))

/* Marks a file the program already created for return; needs a run (EV3C_RUN_DIR). */
Ev3Result ev3_return_try_file(const char *path);
void ev3_return_file_at_(const char *file, int line, const char *path);
#define ev3_return_file(path) ev3_return_file_at_(__FILE__, __LINE__, (path))

#ifdef __cplusplus
}
#endif

#endif /* EV3_RETURN_H */
