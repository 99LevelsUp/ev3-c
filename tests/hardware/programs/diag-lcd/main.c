/*
 * Diagnostic: whether the firmware draws over a program's screen while the
 * program runs. It writes stripes straight to the framebuffer (/dev/fb0),
 * then reads the framebuffer back once a second for 8 seconds and prints how
 * many bytes differ from what it wrote.
 */
#include <fcntl.h>
#include <linux/fb.h>
#include <stdio.h>
#include <string.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <unistd.h>

#include "ev3.h"

int main(void)
{
    struct fb_fix_screeninfo fix;
    struct fb_var_screeninfo var;
    static unsigned char wrote[60 * 128];
    unsigned char *fb;
    size_t size;
    int fd = open("/dev/fb0", O_RDWR);

    if (fd < 0 || ioctl(fd, FBIOGET_FSCREENINFO, &fix) != 0 || ioctl(fd, FBIOGET_VSCREENINFO, &var) != 0) {
        printf("/dev/fb0 cannot be opened or asked\n");
        return 1;
    }
    printf("xres=%u yres=%u bpp=%u line_length=%u smem_len=%u\n", var.xres, var.yres, var.bits_per_pixel,
           fix.line_length, fix.smem_len);
    size = (size_t)fix.line_length * var.yres;
    if (size > sizeof wrote) {
        printf("larger than expected\n");
        return 1;
    }
    fb = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (fb == MAP_FAILED) {
        printf("/dev/fb0 cannot be mapped\n");
        return 1;
    }
    /* Horizontal stripes, 8 rows black, 8 rows white. */
    for (size_t row = 0; row < var.yres; row++) {
        memset(wrote + row * fix.line_length, (row / 8) % 2 ? 0x00 : 0xFF, fix.line_length);
    }
    memcpy(fb, wrote, size);
    ev3_sound_play_tone(880, 150);
    for (int second = 1; second <= 8; second++) {
        size_t changed = 0;
        ev3_sleep_ms(1000);
        for (size_t i = 0; i < size; i++) {
            changed += fb[i] != wrote[i];
        }
        printf("after %d s: %zu of %zu bytes changed\n", second, changed, size);
    }
    ev3_sound_play_tone(440, 150);
    return 0;
}
