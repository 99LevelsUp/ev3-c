/*
 * ev3lib buttons: taking over the Back button and reading its state.
 *
 * Only bytecode in the user slot can block Back, so a program asks for it:
 * it creates the file ".bak" in its project folder, and the launcher that
 * keeps the slot blocks short presses of Back while that file exists.
 */
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

#include "ev3.h"
#include "ev3_core_internal.h"

/*
 * Button state as the LEGO firmware sources (1.09D developer edition)
 * define it:
 *   - lms2012/source/lms2012.h:1257-1261: the UI device shares
 *     "typedef struct { DATA8 Pressed[BUTTONS]; } UI;" with BUTTONS = 6;
 *   - d_ui/Linuxmod_AM1808/d_ui.c:100-105: the order is up, enter, down,
 *     right, left, back, so Back is Pressed[5];
 *   - d_ui/Linuxmod_AM1808/d_ui.c:383-397: a 10 ms timer sets each byte to
 *     1 while the button is down and 0 otherwise;
 *   - d_ui/Linuxmod_AM1808/d_ui.c:467-474: read() returns the hardware
 *     version text, not the buttons, so the state is only reachable through
 *     mmap(), which is how the firmware's own UI reads it
 *     (c_ui/source/c_ui.c:708-716).
 */
#define EV3_BUTTON_COUNT 6
#define EV3_BUTTON_BACK  5

static int back_request_path(char *path, size_t size)
{
    const char *dir = ev3_core_project_dir();

    if (dir == NULL) {
        errno = ENOENT;
        return -1;
    }
    if ((size_t)snprintf(path, size, "%s/.bak", dir) >= size) {
        errno = ENAMETOOLONG;
        return -1;
    }
    return 0;
}

int ev3_button_take_back(void)
{
    char path[PATH_MAX];
    int fd;

    if (back_request_path(path, sizeof path) != 0) {
        return -1;
    }
    fd = open(path, O_WRONLY | O_CREAT, 0666);
    if (fd < 0) {
        return -1;
    }
    return close(fd);
}

int ev3_button_release_back(void)
{
    char path[PATH_MAX];

    if (back_request_path(path, sizeof path) != 0) {
        return -1;
    }
    if (unlink(path) != 0 && errno != ENOENT) {
        return -1;
    }
    return 0;
}

int ev3_button_back_pressed(void)
{
    int fd = open(ev3_core_ui_device(), O_RDONLY);
    void *mapped;
    int pressed;

    if (fd < 0) {
        return -1;
    }
    mapped = mmap(NULL, EV3_BUTTON_COUNT, PROT_READ, MAP_SHARED, fd, 0);
    close(fd);
    if (mapped == MAP_FAILED) {
        return -1;
    }
    pressed = ((volatile const unsigned char *)mapped)[EV3_BUTTON_BACK] != 0;
    munmap(mapped, EV3_BUTTON_COUNT);
    return pressed;
}
