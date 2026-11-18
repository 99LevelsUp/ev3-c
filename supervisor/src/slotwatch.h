/*
 * Noticing the end of the brick's user slot without writing anything.
 *
 * The launcher keeps <project>/.tok open for as long as its slot runs, and
 * the VM (the lms2012 process) closes it when the slot ends. The supervisor
 * records the device and inode of .tok once, then checks whether the VM still
 * has a descriptor open on that inode. A .tok deleted and recreated under the
 * same path is a different inode and so counts as the slot having ended.
 */
#ifndef SUPERVISOR_SLOTWATCH_H
#define SUPERVISOR_SLOTWATCH_H

#include <sys/types.h>

/* Finds the process named lms2012 under proc_root (normally /proc) by the
   name in <pid>/stat. Returns its PID, or -1 if there is none. */
pid_t sw_find_vm(const char *proc_root);

/* Returns 1 if process vm has a descriptor open on (dev, ino), 0 if it has
   not, and -1 if its descriptors cannot be read (the process is gone). */
int sw_holds(const char *proc_root, pid_t vm, dev_t dev, ino_t ino);

#endif
