/*
 * Acceptance: ev3lib's sensors on a brick, step by step. Touch on 1, color on 2,
 * ultrasonic on 3, gyro on 4.
 *
 * Each step beeps as many times as its number and lights the LEDs orange. Press
 * Enter to start it: the LEDs flash green while it measures. Work the sensor,
 * then press Enter again to end the step. It prints the smallest and largest
 * readings of the whole window, so the timing does not matter:
 *   1  touch      press and release the touch sensor a few times
 *   2  reflected  hold something light, then something dark, close to the color sensor
 *   3  color      show the color sensor a few colored bricks
 *   4  distance   move a hand towards and away from the ultrasonic sensor
 *   5  gyro       turn the gyro by hand, about a quarter turn each way
 * Then the gyro is reset: keep it still. The try variants are used, so a
 * missing sensor does not stop the other steps.
 */
#include <fcntl.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>
#include <unistd.h>

#include "ev3.h"

static const char *const colors[] = { "none", "black", "blue", "green", "yellow", "red", "white", "brown" };

/* The raw analog values behind the touch reading, straight from /dev/lms_analog (lms2012.h ANALOG). */
static int raw_pin6(void)
{
    static volatile const int16_t *analog;
    if (analog == NULL) {
        int fd = open("/dev/lms_analog", O_RDONLY);
        void *mapped = fd < 0 ? MAP_FAILED : mmap(NULL, 5172, PROT_READ, MAP_SHARED, fd, 0);
        if (fd >= 0) {
            close(fd);
        }
        if (mapped == MAP_FAILED) {
            return -1;
        }
        analog = mapped;
    }
    return analog[4];
}

static void report(const char *what, Ev3Result result)
{
    printf("  %s: %s (%s)\n", what, ev3_error_string(result), ev3_error_message());
}

/* Announces step n and waits for Enter to start it. */
static void begin(int n, const char *what)
{
    printf("step %d: %s\n", n, what);
    ev3_led_set(EV3_LED_ORANGE);
    for (int i = 0; i < n; i++) {
        ev3_sound_play_tone(880, 120);
        ev3_sleep_ms(250);
    }
    ev3_button_wait_press(EV3_BUTTON_ENTER, 0);
    ev3_button_wait_release(EV3_BUTTON_ENTER);
    ev3_led_set(EV3_LED_GREEN_FLASH);
}

/* Whether Enter ends the step now. */
static bool done(void)
{
    if (!ev3_button_is_pressed(EV3_BUTTON_ENTER)) {
        return false;
    }
    ev3_button_wait_release(EV3_BUTTON_ENTER);
    ev3_led_set(EV3_LED_OFF);
    ev3_sound_play_tone(440, 150);
    return true;
}

int main(void)
{
    int readings, low, high, failed;
    Ev3Result result;

    begin(1, "touch on port 1: press and release it a few times");
    {
        int pressed = 0, released = 0, pin_low = 32767, pin_high = -32768;
        failed = 0;
        for (readings = 0; !done(); readings++) {
            bool touch = false;
            int pin = raw_pin6();
            result = ev3_sensor_try_read_touch(EV3_PORT_1, &touch);
            if (result != EV3_OK && failed++ == 0) {
                report("touch", result);
            }
            touch ? pressed++ : released++;
            pin_low = pin < pin_low ? pin : pin_low;
            pin_high = pin > pin_high ? pin : pin_high;
            ev3_sleep_ms(20);
        }
        printf("  %d readings: %d pressed, %d released; pin 6 from %d to %d\n", readings, pressed, released, pin_low, pin_high);
    }

    begin(2, "reflected light on port 2: something light, then something dark");
    low = 1000, high = -1000, failed = 0;
    for (readings = 0; !done(); readings++) {
        int value = 0;
        result = ev3_sensor_try_read_reflected(EV3_PORT_2, &value);
        if (result != EV3_OK) {
            failed++ == 0 ? report("reflected", result) : (void)0;
            continue;
        }
        low = value < low ? value : low;
        high = value > high ? value : high;
        ev3_sleep_ms(20);
    }
    printf("  %d readings: reflected from %d to %d\n", readings, low, high);

    begin(3, "color on port 2: show it a few colored bricks");
    {
        int seen[8] = { 0 };
        failed = 0;
        for (readings = 0; !done(); readings++) {
            Ev3Color color = EV3_COLOR_NONE;
            result = ev3_sensor_try_read_color(EV3_PORT_2, &color);
            if (result != EV3_OK) {
                failed++ == 0 ? report("color", result) : (void)0;
                continue;
            }
            seen[color]++;
            ev3_sleep_ms(20);
        }
        printf("  %d readings:", readings);
        for (int c = 0; c < 8; c++) {
            if (seen[c] > 0) {
                printf(" %s=%d", colors[c], seen[c]);
            }
        }
        printf("\n");
    }

    begin(4, "distance on port 3: move a hand towards and away");
    {
        float near = 1000.0f, far = -1.0f;
        failed = 0;
        for (readings = 0; !done(); readings++) {
            float cm = 0.0f;
            result = ev3_sensor_try_read_distance_cm(EV3_PORT_3, &cm);
            if (result != EV3_OK) {
                failed++ == 0 ? report("distance", result) : (void)0;
                continue;
            }
            near = cm < near ? cm : near;
            far = cm > far ? cm : far;
            ev3_sleep_ms(20);
        }
        printf("  %d readings: distance from %.1f to %.1f cm\n", readings, (double)near, (double)far);
    }

    begin(5, "gyro on port 4: turn it about a quarter turn each way");
    {
        int angle_low = 100000, angle_high = -100000, rate_low = 100000, rate_high = -100000;
        failed = 0;
        for (readings = 0; !done(); readings++) {
            int angle = 0, rate = 0;
            result = ev3_sensor_try_read_gyro_angle(EV3_PORT_4, &angle);
            if (result == EV3_OK) {
                result = ev3_sensor_try_read_gyro_rate(EV3_PORT_4, &rate);
            }
            if (result != EV3_OK) {
                failed++ == 0 ? report("gyro", result) : (void)0;
                continue;
            }
            angle_low = angle < angle_low ? angle : angle_low;
            angle_high = angle > angle_high ? angle : angle_high;
            rate_low = rate < rate_low ? rate : rate_low;
            rate_high = rate > rate_high ? rate : rate_high;
            ev3_sleep_ms(20);
        }
        printf("  %d readings: angle from %d to %d, rate from %d to %d\n", readings, angle_low, angle_high, rate_low, rate_high);
    }

    printf("gyro reset: keep it still\n");
    {
        int before = 0, after = 0;
        ev3_sensor_try_read_gyro_angle(EV3_PORT_4, &before);
        result = ev3_sensor_try_gyro_reset(EV3_PORT_4);
        if (result != EV3_OK) {
            report("gyro reset", result);
        }
        ev3_sleep_ms(500);
        ev3_sensor_try_read_gyro_angle(EV3_PORT_4, &after);
        printf("  angle %d before the reset, %d after\n", before, after);
    }
    ev3_led_set(EV3_LED_GREEN);
    ev3_sound_play_tone(1320, 300);
    printf("done\n");
    return 0;
}
