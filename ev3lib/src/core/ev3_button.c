/*
 * ev3lib buttons: reading every button and taking over the Back button.
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
#include <time.h>
#include <unistd.h>

#include "ev3.h"
#include "ev3_core_internal.h"

/*
 * Button state as the LEGO firmware sources (1.09D developer edition)
 * define it:
 *   - lms2012/source/lms2012.h:1257-1261: the UI device shares
 *     "typedef struct { DATA8 Pressed[BUTTONS]; } UI;" with BUTTONS = 6;
 *   - d_ui/Linuxmod_AM1808/d_ui.c:100-105: the order is up, enter, down,
 *     right, left, back, so Back is Pressed[5]; EV3_BUTTON_* are 1 << index;
 *   - d_ui/Linuxmod_AM1808/d_ui.c:383-397: a 10 ms timer sets each byte to
 *     1 while the button is down and 0 otherwise;
 *   - d_ui/Linuxmod_AM1808/d_ui.c:467-474: read() returns the hardware
 *     version text, not the buttons, so the state is only reachable through
 *     mmap(), which is how the firmware's own UI reads it
 *     (c_ui/source/c_ui.c:708-716).
 */
#define EV3_BUTTON_COUNT 6
#define EV3_BUTTON_POLL_MS 10

static Ev3Result check_buttons(Ev3Button buttons)
{
    if ((int)buttons <= 0 || ((int)buttons & ~EV3_BUTTON_ANY) != 0) {
        return ev3_fail(EV3_ERR_PARAM, "%#x names no button; use EV3_BUTTON_UP ... EV3_BUTTON_BACK or EV3_BUTTON_ANY",
                        (unsigned int)buttons);
    }
    return EV3_OK;
}

/* The buttons held down now, as a mask. */
static Ev3Result held(Ev3Button *down)
{
    void *mapped;
    int fd = open(ev3_core_ui_device(), O_RDONLY);
    int mask = 0;

    if (fd < 0) {
        return ev3_fail_errno(EV3_ERR_IO, "the button device cannot be opened");
    }
    mapped = mmap(NULL, EV3_BUTTON_COUNT, PROT_READ, MAP_SHARED, fd, 0);
    close(fd);
    if (mapped == MAP_FAILED) {
        return ev3_fail_errno(EV3_ERR_IO, "the button state cannot be mapped");
    }
    for (int i = 0; i < EV3_BUTTON_COUNT; i++) {
        if (((volatile const unsigned char *)mapped)[i] != 0) {
            mask |= 1 << i;
        }
    }
    munmap(mapped, EV3_BUTTON_COUNT);
    *down = (Ev3Button)mask;
    return EV3_OK;
}

Ev3Result ev3_button_try_is_pressed(Ev3Button buttons, bool *pressed)
{
    Ev3Button down;
    Ev3Result result = check_buttons(buttons);

    if (result == EV3_OK && pressed == NULL) {
        result = ev3_fail(EV3_ERR_PARAM, "pressed points nowhere (NULL)");
    }
    if (result == EV3_OK) {
        result = held(&down);
    }
    if (result == EV3_OK) {
        *pressed = ((int)down & (int)buttons) != 0;
    }
    return result;
}

static long long now_ms(void)
{
    struct timespec now;
    clock_gettime(CLOCK_MONOTONIC, &now);
    return (long long)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

Ev3Result ev3_button_try_wait_press(Ev3Button buttons, int timeout_ms, Ev3Button *pressed)
{
    long long deadline;
    Ev3Button down;
    Ev3Result result = check_buttons(buttons);

    if (result != EV3_OK) {
        return result;
    }
    if (timeout_ms < 0) {
        return ev3_fail(EV3_ERR_PARAM, "timeout %d ms is negative; 0 waits without a limit", timeout_ms);
    }
    deadline = now_ms() + timeout_ms;
    for (;;) {
        result = held(&down);
        if (result != EV3_OK) {
            return result;
        }
        down = (Ev3Button)((int)down & (int)buttons);
        if (down != EV3_BUTTON_NONE) {
            /* The lowest of several held at once. */
            if (pressed != NULL) {
                *pressed = (Ev3Button)((int)down & -(int)down);
            }
            return EV3_OK;
        }
        if (timeout_ms > 0 && now_ms() >= deadline) {
            if (pressed != NULL) {
                *pressed = EV3_BUTTON_NONE;
            }
            return ev3_fail(EV3_ERR_TIMEOUT, "no button was pressed within %d ms", timeout_ms);
        }
        ev3_core_sleep(EV3_BUTTON_POLL_MS);
    }
}

Ev3Result ev3_button_try_wait_release(Ev3Button buttons)
{
    Ev3Button down;
    Ev3Result result = check_buttons(buttons);

    while (result == EV3_OK && (result = held(&down)) == EV3_OK && ((int)down & (int)buttons) != 0) {
        ev3_core_sleep(EV3_BUTTON_POLL_MS);
    }
    return result;
}

static Ev3Result back_request_path(char *path, size_t size)
{
    const char *dir = ev3_core_project_dir();

    if (dir == NULL) {
        return ev3_fail(EV3_ERR_NOT_SUPPORTED,
                        "the Back button can be taken over only in a run started by EVƎ C (EV3C_PROJECT_DIR is not set)");
    }
    if ((size_t)snprintf(path, size, "%s/.bak", dir) >= size) {
        return ev3_fail(EV3_ERR_OVERFLOW, "the project folder's path is too long");
    }
    return EV3_OK;
}

Ev3Result ev3_button_try_take_back(void)
{
    char path[PATH_MAX];
    int fd;
    Ev3Result result = back_request_path(path, sizeof path);

    if (result != EV3_OK) {
        return result;
    }
    fd = open(path, O_WRONLY | O_CREAT, 0666);
    if (fd < 0 || close(fd) != 0) {
        return ev3_fail_errno(EV3_ERR_IO, "the Back request cannot be written");
    }
    return EV3_OK;
}

Ev3Result ev3_button_try_release_back(void)
{
    char path[PATH_MAX];
    Ev3Result result = back_request_path(path, sizeof path);

    if (result != EV3_OK) {
        return result;
    }
    if (unlink(path) != 0 && errno != ENOENT) {
        return ev3_fail_errno(EV3_ERR_IO, "the Back request cannot be removed");
    }
    return EV3_OK;
}

/* --- plain variants ------------------------------------------------------------ */

bool ev3_button_is_pressed_at_(EV3_AT, Ev3Button buttons)
{
    bool pressed = false;
    EV3_MUST("ev3_button_is_pressed", ev3_button_try_is_pressed(buttons, &pressed));
    return pressed;
}

Ev3Button ev3_button_wait_press_at_(EV3_AT, Ev3Button buttons, int timeout_ms)
{
    Ev3Button pressed = EV3_BUTTON_NONE;
    Ev3Result result;

    ev3_error_clear();
    result = ev3_button_try_wait_press(buttons, timeout_ms, &pressed);
    if (result != EV3_OK && result != EV3_ERR_TIMEOUT) {
        ev3_stop_on_error_(file_, line_, "ev3_button_wait_press", result);
    }
    return pressed;
}

void ev3_button_wait_release_at_(EV3_AT, Ev3Button buttons)
{
    EV3_MUST("ev3_button_wait_release", ev3_button_try_wait_release(buttons));
}

void ev3_button_take_back_at_(EV3_AT)
{
    EV3_MUST("ev3_button_take_back", ev3_button_try_take_back());
}

void ev3_button_release_back_at_(EV3_AT)
{
    EV3_MUST("ev3_button_release_back", ev3_button_try_release_back());
}
