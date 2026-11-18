/* Slot-end detection through the VM's open descriptors. See slotwatch.h. */
#define _GNU_SOURCE
#include "slotwatch.h"

#include <dirent.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>

pid_t sw_find_vm(const char *proc_root)
{
    DIR *d = opendir(proc_root);
    struct dirent *e;
    pid_t found = -1;
    if (d == NULL) {
        return -1;
    }
    while (found < 0 && (e = readdir(d)) != NULL) {
        char path[4096], text[128];
        const char *open_paren, *close_paren;
        ssize_t n;
        int fd;
        if (e->d_name[0] < '0' || e->d_name[0] > '9') {
            continue;
        }
        snprintf(path, sizeof path, "%s/%s/stat", proc_root, e->d_name);
        fd = open(path, O_RDONLY | O_CLOEXEC);
        if (fd < 0) {
            continue;
        }
        n = read(fd, text, sizeof text - 1);
        close(fd);
        if (n <= 0) {
            continue;
        }
        text[n] = '\0';
        /* "<pid> (<name>) <state> ..." */
        open_paren = strchr(text, '(');
        close_paren = open_paren != NULL ? strchr(open_paren, ')') : NULL;
        if (close_paren != NULL && close_paren - open_paren - 1 == 7
            && strncmp(open_paren + 1, "lms2012", 7) == 0) {
            found = (pid_t)strtol(e->d_name, NULL, 10);
        }
    }
    closedir(d);
    return found;
}

int sw_holds(const char *proc_root, pid_t vm, dev_t dev, ino_t ino)
{
    char dir[4096];
    DIR *d;
    struct dirent *e;
    int held = 0;
    snprintf(dir, sizeof dir, "%s/%d/fd", proc_root, (int)vm);
    d = opendir(dir);
    if (d == NULL) {
        return -1;
    }
    while (!held && (e = readdir(d)) != NULL) {
        char path[4352];
        struct stat st;
        if (e->d_name[0] == '.') {
            continue;
        }
        snprintf(path, sizeof path, "%s/%s", dir, e->d_name);
        if (stat(path, &st) == 0 && st.st_dev == dev && st.st_ino == ino) {
            held = 1;
        }
    }
    closedir(d);
    return held;
}
