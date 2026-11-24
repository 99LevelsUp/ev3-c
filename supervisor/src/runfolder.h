/*
 * Files the supervisor keeps in the project folder.
 *
 *   <project>/.run          the supervisor's PID as decimal digits, no newline
 *   <project>/.req          launch ID written by EVƎ C before it starts a run
 *   <project>/runs/next     number of the next run; never goes back, so a run
 *                           number is never reused after its folder is deleted
 *   <project>/runs/<n>/out  the program's stdout, at most the output cap
 *   <project>/runs/<n>/err  the program's stderr, at most the output cap
 *   <project>/runs/<n>/res  the run result, renamed into place as the last
 *                           write; its existence means the run is complete
 *
 * The name "ret" inside a run folder is reserved for the list of files a
 * program marks for return. It is not created here; retention deletes it with
 * its folder.
 */
#ifndef SUPERVISOR_RUNFOLDER_H
#define SUPERVISOR_RUNFOLDER_H

#include <stddef.h>
#include <sys/types.h>

/* Longest launch ID kept from .req; the rest of the line is dropped. */
#define RF_LAUNCH_ID_MAX 63

/* How many old run folders survive when a new run starts. Exactly one of the
   two limits is positive. */
typedef struct {
    /* Keep at most this many run folders, counting the new one. */
    int keep_runs;
    /* Keep old run folders while their total size plus room for the new run
       stays within this many bytes. */
    long long keep_bytes;
} rf_retention;

/*
 * How a run ended, as recorded in res. res is key=value text, one pair per
 * line, LF line ends:
 *   run=<n>                          always
 *   end=program|slot|before-start    always
 *   exit=<code>                      when the program exited
 *   signal=<number>                  when a signal ended the program
 *   out_truncated=0|1                always
 *   err_truncated=0|1                always
 *   launch=<id>                      only when .req was present
 */
typedef enum {
    RF_END_PROGRAM,      /* the program ended on its own: end=program */
    RF_END_SLOT,         /* the user slot ended and the program was stopped: end=slot */
    RF_END_SLOT_BEFORE   /* the slot had ended before the program could start: end=before-start */
} rf_end;

typedef struct {
    long number;
    rf_end end;
    /* For RF_END_PROGRAM and RF_END_SLOT: the program's wait status. */
    int have_status;
    int status;
    int out_truncated;
    int err_truncated;
    /* Launch ID from .req, or NULL when the run had none. */
    const char *launch_id;
} rf_result;

/*
 * Reads the first line of <project>/.req into id (at most RF_LAUNCH_ID_MAX
 * bytes, without the line end) and deletes the file. Returns 1 when .req was
 * there, 0 when it was not.
 */
int rf_take_launch_id(const char *project, char *id, size_t size);

/*
 * Writes <dir>/launch with the launch ID and a line end, through a temporary
 * file and a rename. The supervisor writes it as soon as the run folder exists,
 * before the program starts, so EVƎ C can tell which folder its run took and
 * follow out and err while the program runs. Returns 0, or -1 with errno set.
 */
int rf_write_launch(const char *dir, const char *launch_id);

/* Writes <project>/.run with the PID, through a temporary file and a rename. */
int rf_write_pid_marker(const char *project, pid_t pid);

/*
 * Starts a run: creates <runs> when missing, takes the next run number,
 * advances the counter, applies the retention rule to the existing run
 * folders (oldest first), and creates the new run folder. reserve is the most
 * the new run can write, used by the size rule. Writes the folder's path to
 * dir and its number to *number. Returns 0, or -1 with errno set.
 */
int rf_start_run(const char *runs, const rf_retention *rule, long long reserve,
                 char *dir, size_t size, long *number);

/* Formats the result as key=value lines. Returns the length, or -1 if it does
   not fit. */
int rf_format_result(const rf_result *result, char *text, size_t size);

/* Writes the result to <dir>/res.tmp. res itself does not exist yet. */
int rf_write_result_temp(const char *dir, const rf_result *result);

/* Renames <dir>/res.tmp to <dir>/res: the run's completion record. */
int rf_commit_result(const char *dir);

#endif
