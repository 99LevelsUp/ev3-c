/*
 * EVƎ C supervisor: runs one student program per run on the EV3 brick, records
 * how it ended, stops the motors afterwards, and tells the launcher when the
 * run is over.
 *
 * The launcher in the brick's user slot starts it in the background:
 *
 *     <folder>/sup <folder> keep=<n> cap=<bytes>
 *     <folder>/sup <folder> keepmb=<m> cap=<bytes>
 *
 *   <folder>     the project folder as the VM reports it; relative to the
 *                current directory when the run was started from the brick
 *                menu, so it is resolved to an absolute path first
 *   keep=<n>     keep the last n run folders, counting the new one
 *   keepmb=<m>   keep old run folders while their total size plus room for the
 *                new run's two capped streams stays within m MiB
 *   cap=<bytes>  the most bytes kept of each of stdout and stderr
 *
 * One run:
 *   1. Leave the VM's session and process group, point stdin, stdout and
 *      stderr at /dev/null, and close every other descriptor inherited from
 *      the VM.
 *   2. Read and delete <folder>/.req (the launch ID EVƎ C wrote before the
 *      run), and record which .tok the VM holds open for the slot.
 *   3. Take the brick-wide lock, waiting up to 3 s; without it, give up and
 *      leave no trace, so a still-finishing run's motor stop can never hit
 *      this one. The lock is held until the result is written.
 *   4. If the VM still holds that .tok, write <folder>/.run with this PID.
 *   5. Apply the retention rule, take the next run number, create
 *      <folder>/runs/<n>/ (see runfolder.h).
 *   6. If the slot has already ended, record that and start nothing.
 *      Otherwise start <folder>/prog with argv[0] "prog" in its own process
 *      group, stdout and stderr captured into out and err up to the cap, and
 *      EV3C_PROJECT_DIR and EV3C_RUN_DIR set to the two absolute folders.
 *   7. Every 100 ms check the program and the slot token.
 *        - The program ended: SIGKILL its whole group, so a helper process it
 *          forked cannot keep driving motors after the run.
 *        - The slot ended: SIGTERM to the group, 500 ms grace, then SIGKILL.
 *      Either way wait until the whole group is gone, then drain the output
 *      for at most 1 s and stop all motors once more.
 *   8. Write res through res.tmp and a rename (the completion record), remove
 *      .bak and .run, release the lock.
 *
 * Environment, for tests on a PC (the defaults are the brick's):
 *   EV3C_PROC_ROOT   where to look for the VM process, default /proc
 *   EV3C_PWM_DEVICE  motor device, default /dev/lms_pwm
 *   EV3C_LOCK_FILE   brick-wide run lock, default /home/root/lms2012/prjs/.ev3c.lock
 * The program inherits the supervisor's environment.
 *
 * Exit status: 0 when res was written; 2 for bad arguments; 3 when the lock
 * did not come free in time; 4 when the run folder could not be created;
 * 5 when res could not be written.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <poll.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/file.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#include "runfolder.h"
#include "slotwatch.h"

#define TICK_MS 100
#define GRACE_MS 500
#define LOCK_WAIT_MS 3000
#define LOCK_RETRY_MS 50
#define DRAIN_MS 1000
#define KILL_RETRY_MS 10

/* firmware opPROGRAM_STOP: brake or float every motor, as the firmware does
   when a program ends */
#define PWM_PROGRAM_STOP 0x02

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

static const char *env_or(const char *name, const char *fallback)
{
    const char *value = getenv(name);
    return value != NULL && value[0] != '\0' ? value : fallback;
}

/* "<prefix><digits>" with a value from 1 to INT_MAX. */
static int parse_number(const char *text, const char *prefix, long long *value)
{
    size_t length = strlen(prefix);
    const char *digits = text + length;
    long long n = 0;
    if (strncmp(text, prefix, length) != 0 || *digits == '\0') {
        return -1;
    }
    for (; *digits != '\0'; digits++) {
        if (*digits < '0' || *digits > '9') {
            return -1;
        }
        n = n * 10 + (*digits - '0');
        if (n > INT_MAX) {
            return -1;
        }
    }
    if (n < 1) {
        return -1;
    }
    *value = n;
    return 0;
}

static void close_from(int lowest)
{
    struct rlimit limit;
    long highest = 1024;
    long fd;
    if (getrlimit(RLIMIT_NOFILE, &limit) == 0 && limit.rlim_cur != RLIM_INFINITY) {
        highest = (long)limit.rlim_cur;
    }
    if (highest > 65536) {
        highest = 65536;
    }
    for (fd = lowest; fd < highest; fd++) {
        close((int)fd);
    }
}

/* The supervisor inherits every descriptor the VM has open: devices, internal
   pipes, the slot token. None of them is its business. */
static void detach_from_vm(void)
{
    int null_fd = open("/dev/null", O_RDWR);
    if (null_fd >= 0) {
        dup2(null_fd, 0);
        dup2(null_fd, 1);
        dup2(null_fd, 2);
    }
    close_from(3);
}

static void stop_motors(const char *device)
{
    const unsigned char command = PWM_PROGRAM_STOP;
    int fd = open(device, O_WRONLY | O_APPEND | O_CLOEXEC);
    if (fd >= 0) {
        /* A failed write leaves nothing more to try; the firmware's own stop
           at slot end remains. */
        ssize_t written = write(fd, &command, 1);
        (void)written;
        close(fd);
    }
}

static int take_lock(const char *path)
{
    long long deadline = now_ms() + LOCK_WAIT_MS;
    int fd = open(path, O_RDWR | O_CREAT | O_CLOEXEC, 0666);
    if (fd < 0) {
        return -1;
    }
    for (;;) {
        if (flock(fd, LOCK_EX | LOCK_NB) == 0) {
            return fd;
        }
        if ((errno != EWOULDBLOCK && errno != EINTR) || now_ms() >= deadline) {
            close(fd);
            return -1;
        }
        sleep_ms(LOCK_RETRY_MS);
    }
}

/* --- output capture --------------------------------------------------------- */

typedef struct {
    int pipe;          /* read end, -1 once at end of file */
    int file;
    long long kept;
    long long cap;
    int truncated;
} stream;

/* Moves what the pipe holds into the file, up to the cap. Bytes beyond the
   cap are read and dropped, so the program never blocks on a full pipe. */
static void pump(stream *s)
{
    char buf[4096];
    while (s->pipe >= 0) {
        ssize_t n = read(s->pipe, buf, sizeof buf);
        if (n > 0) {
            long long room = s->cap - s->kept;
            long long keep = n < room ? n : room;
            if (keep > 0) {
                ssize_t written = write(s->file, buf, (size_t)keep);
                (void)written;
                s->kept += keep;
            }
            if (keep < n) {
                s->truncated = 1;
            }
        } else if (n < 0 && errno == EINTR) {
            continue;
        } else if (n < 0 && errno == EAGAIN) {
            return;
        } else {
            close(s->pipe);
            s->pipe = -1;
        }
    }
}

/* --- the program ------------------------------------------------------------ */

static void exec_program(const char *program, int out, int err)
{
    struct sigaction action;
    sigset_t none;
    char *argv[] = { "prog", NULL };
    int sig;

    setpgid(0, 0);
    memset(&action, 0, sizeof action);
    action.sa_handler = SIG_DFL;
    for (sig = 1; sig < 65; sig++) {
        sigaction(sig, &action, NULL);
    }
    sigemptyset(&none);
    sigprocmask(SIG_SETMASK, &none, NULL);
    dup2(out, 1);
    dup2(err, 2);
    close_from(3);
    execv(program, argv);
    _exit(127);
}

static int group_gone(pid_t group)
{
    return kill(-group, 0) != 0 && errno == ESRCH;
}

typedef struct {
    const char *proc_root;
    pid_t vm;
    dev_t tok_dev;
    ino_t tok_ino;
} slot_token;

/*
 * Runs the program and fills in how it ended. Returns once the program's
 * whole process group is gone and its output is drained.
 */
static void run_program(const char *project, const char *run_dir, long long cap,
                        const slot_token *token, rf_result *result)
{
    enum { RUNNING, GRACE, EMPTYING, DRAINING, DONE } state = RUNNING;
    char program[PATH_MAX + 8], path[PATH_MAX + 8];
    stream streams[2];
    int out_pipe[2], err_pipe[2];
    long long next_tick, grace_end = 0, drain_end = 0;
    pid_t child;
    int reaped = 0, status = 0, i;

    result->end = RF_END_PROGRAM;
    result->have_status = 1;
    result->status = 127 << 8;

    snprintf(program, sizeof program, "%s/prog", project);
    for (i = 0; i < 2; i++) {
        snprintf(path, sizeof path, "%s/%s", run_dir, i == 0 ? "out" : "err");
        streams[i].file = open(path, O_WRONLY | O_CREAT | O_TRUNC | O_CLOEXEC, 0666);
        streams[i].pipe = -1;
        streams[i].kept = 0;
        streams[i].cap = cap;
        streams[i].truncated = 0;
    }
    if (pipe2(out_pipe, O_CLOEXEC) != 0) {
        goto close_files;
    }
    if (pipe2(err_pipe, O_CLOEXEC) != 0) {
        close(out_pipe[0]);
        close(out_pipe[1]);
        goto close_files;
    }
    setenv("EV3C_PROJECT_DIR", project, 1);
    setenv("EV3C_RUN_DIR", run_dir, 1);

    child = fork();
    if (child == 0) {
        exec_program(program, out_pipe[1], err_pipe[1]);
    }
    close(out_pipe[1]);
    close(err_pipe[1]);
    streams[0].pipe = out_pipe[0];
    streams[1].pipe = err_pipe[0];
    if (child < 0) {
        close(out_pipe[0]);
        close(err_pipe[0]);
        goto close_files;
    }
    /* Also from this side, so the group exists before any signal is sent. */
    setpgid(child, child);
    for (i = 0; i < 2; i++) {
        fcntl(streams[i].pipe, F_SETFL, fcntl(streams[i].pipe, F_GETFL) | O_NONBLOCK);
    }

    next_tick = now_ms() + TICK_MS;
    while (state != DONE) {
        struct pollfd fds[2];
        int count = 0, timeout;
        long long now = now_ms();

        if (state == RUNNING) {
            timeout = next_tick > now ? (int)(next_tick - now) : 0;
        } else if (state == DRAINING) {
            timeout = drain_end > now ? (int)(drain_end - now) : 0;
        } else {
            timeout = KILL_RETRY_MS;
        }
        for (i = 0; i < 2; i++) {
            if (streams[i].pipe >= 0) {
                fds[count].fd = streams[i].pipe;
                fds[count].events = POLLIN;
                fds[count].revents = 0;
                count++;
            }
        }
        if (state == DRAINING && count == 0) {
            break;
        }
        poll(fds, (nfds_t)count, timeout);
        pump(&streams[0]);
        pump(&streams[1]);

        if (!reaped && waitpid(child, &status, WNOHANG) == child) {
            reaped = 1;
            result->status = status;
        }
        now = now_ms();
        switch (state) {
        case RUNNING:
            if (reaped) {
                kill(-child, SIGKILL);
                state = EMPTYING;
            } else if (now >= next_tick) {
                next_tick += TICK_MS;
                if (sw_holds(token->proc_root, token->vm, token->tok_dev, token->tok_ino) != 1) {
                    result->end = RF_END_SLOT;
                    kill(-child, SIGTERM);
                    grace_end = now + GRACE_MS;
                    state = GRACE;
                }
            }
            break;
        case GRACE:
            if (reaped && group_gone(child)) {
                state = DRAINING;
                drain_end = now + DRAIN_MS;
            } else if (now >= grace_end) {
                kill(-child, SIGKILL);
                state = EMPTYING;
            }
            break;
        case EMPTYING:
            if (reaped && group_gone(child)) {
                state = DRAINING;
                drain_end = now + DRAIN_MS;
            } else {
                kill(-child, SIGKILL);
            }
            break;
        case DRAINING:
            if (now >= drain_end) {
                state = DONE;
            }
            break;
        case DONE:
            break;
        }
    }
    for (i = 0; i < 2; i++) {
        if (streams[i].pipe >= 0) {
            close(streams[i].pipe);
        }
    }

close_files:
    for (i = 0; i < 2; i++) {
        if (streams[i].file >= 0) {
            close(streams[i].file);
        }
    }
    result->out_truncated = streams[0].truncated;
    result->err_truncated = streams[1].truncated;
}

int main(int argc, char **argv)
{
    char project[PATH_MAX], path[PATH_MAX + 16], run_dir[PATH_MAX + 32];
    char tok[PATH_MAX + 8], run_marker[PATH_MAX + 8], bak[PATH_MAX + 8];
    char launch[RF_LAUNCH_ID_MAX + 1];
    const char *pwm, *lock_path;
    struct sigaction action;
    struct stat tok_st;
    slot_token token;
    rf_retention rule = { 0, 0 };
    rf_result result;
    long long value, cap;
    int have_launch, token_seen, slot_alive, lock;

    setsid();
    detach_from_vm();
    /* An inherited "ignore" on SIGCHLD would hide the program's exit status. */
    memset(&action, 0, sizeof action);
    action.sa_handler = SIG_DFL;
    sigaction(SIGCHLD, &action, NULL);

    if (argc != 4) {
        return 2;
    }
    if (parse_number(argv[2], "keep=", &value) == 0) {
        rule.keep_runs = (int)value;
    } else if (parse_number(argv[2], "keepmb=", &value) == 0) {
        rule.keep_bytes = value * 1024 * 1024;
    } else {
        return 2;
    }
    if (parse_number(argv[3], "cap=", &cap) != 0 || realpath(argv[1], project) == NULL) {
        return 2;
    }
    token.proc_root = env_or("EV3C_PROC_ROOT", "/proc");
    pwm = env_or("EV3C_PWM_DEVICE", "/dev/lms_pwm");
    lock_path = env_or("EV3C_LOCK_FILE", "/home/root/lms2012/prjs/.ev3c.lock");
    snprintf(tok, sizeof tok, "%s/.tok", project);
    snprintf(run_marker, sizeof run_marker, "%s/.run", project);
    snprintf(bak, sizeof bak, "%s/.bak", project);

    /* Taken before the lock wait, so a run that gives up on the lock does not
       leave its launch ID for a later, unrelated run. */
    have_launch = rf_take_launch_id(project, launch, sizeof launch);

    token.vm = sw_find_vm(token.proc_root);
    token_seen = token.vm > 0 && stat(tok, &tok_st) == 0
        && sw_holds(token.proc_root, token.vm, tok_st.st_dev, tok_st.st_ino) == 1;
    if (token_seen) {
        token.tok_dev = tok_st.st_dev;
        token.tok_ino = tok_st.st_ino;
    }

    lock = take_lock(lock_path);
    if (lock < 0) {
        return 3;
    }

    /* The slot may have ended during the lock wait. Without a slot no .run is
       written: a newer launcher could otherwise take this PID for its own
       supervisor's. */
    slot_alive = token_seen && sw_holds(token.proc_root, token.vm, token.tok_dev, token.tok_ino) == 1;
    if (slot_alive) {
        rf_write_pid_marker(project, getpid());
    }

    snprintf(path, sizeof path, "%s/runs", project);
    if (rf_start_run(path, &rule, 2 * cap, run_dir, sizeof run_dir, &result.number) != 0) {
        unlink(bak);
        unlink(run_marker);
        return 4;
    }
    result.end = RF_END_SLOT_BEFORE;
    result.have_status = 0;
    result.status = 0;
    result.out_truncated = 0;
    result.err_truncated = 0;
    result.launch_id = have_launch ? launch : NULL;

    if (slot_alive) {
        run_program(project, run_dir, cap, &token, &result);
        stop_motors(pwm);
    }

    if (rf_write_result_temp(run_dir, &result) != 0 || rf_commit_result(run_dir) != 0) {
        unlink(bak);
        unlink(run_marker);
        return 5;
    }
    unlink(bak);
    unlink(run_marker);
    close(lock);
    return 0;
}
