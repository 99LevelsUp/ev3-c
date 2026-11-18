/*
 * Brick spike probe: a stand-in supervisor started by the probe launcher.
 *
 * Usage: probe_sup <project-folder> [lcd] [back]
 *
 * It records, with millisecond timestamps, into <project-folder>/sup.log:
 *   - its process identity before and after setsid(), and the descriptors it inherited;
 *   - the lms2012 process and whether that process holds <folder>/.tok open (device + inode);
 *   - how long until the VM stops holding that inode (slot end);
 * It writes its PID into <folder>/.run (the launcher polls for it), starts a child in its own
 * process group that keeps setting motor A to 40 % power, and on slot end kills that group,
 * stops all motors through /dev/lms_pwm, removes .run and .bak, and exits.
 * With "back" it creates <folder>/.bak for 20 s so the launcher blocks the Back button.
 * With "lcd" the child also draws a pattern into /dev/fb0 so the operator can see whether
 * the firmware UI draws over it while the slot runs.
 * It gives up after 120 s on its own.
 */
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <signal.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static FILE *logf;
static struct timespec t0;

static long ms(void)
{
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (t.tv_sec - t0.tv_sec) * 1000L + (t.tv_nsec - t0.tv_nsec) / 1000000L;
}

static void say(const char *fmt, ...)
{
    va_list ap;
    fprintf(logf, "%6ld ", ms());
    va_start(ap, fmt);
    vfprintf(logf, fmt, ap);
    va_end(ap);
    fputc('\n', logf);
    fflush(logf);
}

static pid_t find_vm(void)
{
    DIR *d = opendir("/proc");
    struct dirent *e;
    pid_t found = -1;
    if (!d) {
        return -1;
    }
    while ((e = readdir(d)) != NULL) {
        char path[64], comm[64];
        FILE *f;
        if (e->d_name[0] < '0' || e->d_name[0] > '9') {
            continue;
        }
        snprintf(path, sizeof path, "/proc/%s/stat", e->d_name);
        f = fopen(path, "r");
        if (!f) {
            continue;
        }
        if (fscanf(f, "%*d (%63[^)])", comm) == 1 && strcmp(comm, "lms2012") == 0) {
            found = (pid_t)atoi(e->d_name);
        }
        fclose(f);
        if (found > 0) {
            break;
        }
    }
    closedir(d);
    return found;
}

/* Does process pid hold an open descriptor on (dev, ino)? Logs the descriptor list when verbose. */
static int holds(pid_t pid, dev_t dev, ino_t ino, int verbose)
{
    char dir[64];
    DIR *d;
    struct dirent *e;
    int held = 0;
    snprintf(dir, sizeof dir, "/proc/%d/fd", (int)pid);
    d = opendir(dir);
    if (!d) {
        if (verbose) {
            say("cannot open %s: %s", dir, strerror(errno));
        }
        return -1;
    }
    while ((e = readdir(d)) != NULL) {
        char path[128], target[256];
        struct stat st;
        ssize_t n;
        if (e->d_name[0] == '.') {
            continue;
        }
        snprintf(path, sizeof path, "%s/%s", dir, e->d_name);
        if (verbose) {
            n = readlink(path, target, sizeof target - 1);
            target[n > 0 ? n : 0] = 0;
            say("  fd %s -> %s", e->d_name, target);
        }
        if (stat(path, &st) == 0 && st.st_dev == dev && st.st_ino == ino) {
            held = 1;
        }
    }
    closedir(d);
    return held;
}

static void pwm(const unsigned char *cmd, size_t n)
{
    int fd = open("/dev/lms_pwm", O_WRONLY);
    if (fd >= 0) {
        if (write(fd, cmd, n) < 0) {
            say("pwm write failed: %s", strerror(errno));
        }
        close(fd);
    } else {
        say("cannot open /dev/lms_pwm: %s", strerror(errno));
    }
}

static void child_loop(int lcd)
{
    /* opOUTPUT_POWER(0xA4) port mask 0x01 power 40, then opOUTPUT_START(0xA6) mask 0x01. */
    const unsigned char power[] = { 0xA4, 0x01, 40 };
    const unsigned char start[] = { 0xA6, 0x01 };
    int fb = lcd ? open("/dev/fb0", O_WRONLY) : -1;
    unsigned char row[24];
    int i = 0;
    for (;;) {
        pwm(power, sizeof power);
        pwm(start, sizeof start);
        if (fb >= 0) {
            memset(row, (i++ & 1) ? 0xAA : 0x55, sizeof row);
            lseek(fb, 0, SEEK_SET);
            for (int y = 0; y < 64; y++) {
                if (write(fb, row, sizeof row) < 0) {
                    break;
                }
            }
        }
        usleep(100000);
    }
}

int main(int argc, char **argv)
{
    char logpath[256], tok[256], run[256], bak[256];
    struct stat tokst;
    pid_t vm, child;
    int lcd = 0, back = 0, i, held, limit = 1200;
    long slot_end = -1;

    clock_gettime(CLOCK_MONOTONIC, &t0);
    if (argc < 2) {
        return 2;
    }
    for (i = 2; i < argc; i++) {
        lcd |= strcmp(argv[i], "lcd") == 0;
        back |= strcmp(argv[i], "back") == 0;
    }
    {
        /* The probe launcher passes only the folder; options come from <folder>/opts. */
        char optpath[256], opts[64] = "";
        FILE *o;
        snprintf(optpath, sizeof optpath, "%s/opts", argv[1]);
        o = fopen(optpath, "r");
        if (o) {
            if (fgets(opts, sizeof opts, o) == NULL) {
                opts[0] = 0;
            }
            fclose(o);
        }
        lcd |= strstr(opts, "lcd") != NULL;
        back |= strstr(opts, "back") != NULL;
        if (strstr(opts, "short") != NULL) {
            limit = 50; /* about 5 s, then behave as if the program ended on its own */
        }
    }
    snprintf(logpath, sizeof logpath, "%s/sup.log", argv[1]);
    snprintf(tok, sizeof tok, "%s/.tok", argv[1]);
    snprintf(run, sizeof run, "%s/.run", argv[1]);
    snprintf(bak, sizeof bak, "%s/.bak", argv[1]);
    logf = fopen(logpath, "w");
    if (!logf) {
        return 2;
    }

    say("start folder=%s", argv[1]);
    say("before setsid: pid=%d ppid=%d pgid=%d sid=%d", (int)getpid(), (int)getppid(), (int)getpgid(0), (int)getsid(0));
    say("inherited descriptors:");
    holds(getpid(), 0, 0, 1);
    if (setsid() < 0) {
        say("setsid failed: %s", strerror(errno));
    }
    say("after setsid: pid=%d pgid=%d sid=%d", (int)getpid(), (int)getpgid(0), (int)getsid(0));

    vm = find_vm();
    say("lms2012 pid=%d", (int)vm);
    if (stat(tok, &tokst) != 0) {
        say("stat %s failed: %s", tok, strerror(errno));
        return 2;
    }
    say("token dev=%lu ino=%lu, VM descriptors:", (unsigned long)tokst.st_dev, (unsigned long)tokst.st_ino);
    held = holds(vm, tokst.st_dev, tokst.st_ino, 1);
    say("VM holds token at start: %d", held);

    {
        /* Write to a temporary name and rename, so the launcher never reads a half-written PID. */
        char tmp[260];
        FILE *r;
        snprintf(tmp, sizeof tmp, "%s.tmp", run);
        r = fopen(tmp, "w");
        if (r) {
            fprintf(r, "%d", (int)getpid());
            fclose(r);
            rename(tmp, run);
            say("wrote .run");
        } else {
            say("cannot write .run: %s", strerror(errno));
        }
    }
    if (back) {
        FILE *b = fopen(bak, "w");
        if (b) {
            fclose(b);
            say("created .bak (Back should now be blocked for short presses)");
        }
    }

    child = fork();
    if (child == 0) {
        setpgid(0, 0);
        child_loop(lcd);
        _exit(0);
    }
    setpgid(child, child);
    say("child pid=%d pgid=%d", (int)child, (int)getpgid(child));

    for (i = 0; i < limit; i++) {
        usleep(100000);
        if (back && i == 200) {
            unlink(bak);
            say("removed .bak (Back should stop the program again)");
        }
        held = holds(vm, tokst.st_dev, tokst.st_ino, 0);
        if (held != 1) {
            slot_end = ms();
            say("slot end detected (held=%d)", held);
            break;
        }
    }
    if (slot_end < 0) {
        say("loop limit reached, slot still running: ending as if the program ended");
    }

    kill(-child, SIGTERM);
    usleep(500000);
    kill(-child, SIGKILL);
    waitpid(child, NULL, 0);
    {
        const unsigned char stop_all[] = { 0x02 }; /* opPROGRAM_STOP: brake or float every motor */
        pwm(stop_all, sizeof stop_all);
    }
    unlink(bak);
    unlink(run);
    say("motors stopped, markers removed, exiting");
    fclose(logf);
    return 0;
}
