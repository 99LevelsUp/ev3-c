/*
 * Brick spike probe: proves a static Zig + musl binary runs on the stock
 * firmware, and reports the kernel and its own process identity.
 * Exits with code 3 so the caller can see the exit status survives.
 */
#include <stdio.h>
#include <unistd.h>
#include <sys/utsname.h>

int main(void)
{
    struct utsname u;
    if (uname(&u) == 0) {
        printf("uname: %s %s %s %s\n", u.sysname, u.release, u.version, u.machine);
    }
    printf("pid=%d ppid=%d pgid=%d sid=%d\n",
           (int)getpid(), (int)getppid(), (int)getpgid(0), (int)getsid(0));
    printf("hello from musl\n");
    return 3;
}
