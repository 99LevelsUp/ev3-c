/*
 * Diagnostic: the color sensor on port 2 in RGB-RAW. Hold something white in
 * front of it, then press Enter. It reads the raw colors five times and prints
 * them, with the raw bytes the UART driver keeps for the port
 * (lms2012.h UART: Raw[port][Actual[port]]).
 */
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

#include "ev3.h"

int main(void)
{
    volatile const unsigned char *uart;
    int fd = open("/dev/lms_uart", O_RDONLY);
    void *mapped = fd < 0 ? MAP_FAILED : mmap(NULL, 42744, PROT_READ, MAP_SHARED, fd, 0);

    if (mapped == MAP_FAILED) {
        printf("/dev/lms_uart cannot be mapped\n");
        return 1;
    }
    uart = mapped;
    ev3_sound_play_tone(880, 150);
    ev3_button_wait_press(EV3_BUTTON_ENTER, 0);
    ev3_button_wait_release(EV3_BUTTON_ENTER);
    for (int i = 0; i < 5; i++) {
        int reflected = ev3_sensor_read_reflected(EV3_PORT_2);
        Ev3ColorRgb rgb = ev3_sensor_read_rgb(EV3_PORT_2);
        printf("reflected=%d ", reflected);
        unsigned int actual = *(volatile const uint16_t *)(uart + 42592 + 2);
        volatile const unsigned char *raw = uart + 4192 + (1 * 300 + actual % 300) * 32;
        printf("rgb=%d/%d/%d status=%#x actual=%u raw=", rgb.red, rgb.green, rgb.blue, uart[42608 + 1], actual);
        for (int b = 0; b < 8; b++) {
            printf("%02x ", raw[b]);
        }
        printf("\n");
        ev3_sleep_ms(300);
    }
    printf("reflected=%d\n", ev3_sensor_read_reflected(EV3_PORT_2));
    ev3_sound_play_tone(440, 150);
    return 0;
}
