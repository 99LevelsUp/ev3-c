/*
 * Acceptance: the program's output while it runs (#28). For 30 seconds it
 * writes one line a second to stdout and one to stderr; at second 15 it also
 * writes 60 KiB to stdout in one go (lines of 63 dashes and a line end). Run
 * it with `ev3 c run`: the lines should appear about a second apart, the burst
 * all at once, and the result should not repeat the output.
 */
#include <stdio.h>
#include <string.h>

#include "ev3.h"

int main(void)
{
    static char line[64];

    memset(line, '-', sizeof line - 1);
    line[sizeof line - 1] = '\0';
    for (int second = 1; second <= 30; second++) {
        printf("out %2d: a line on stdout, \xC5\xBEluva\n", second);
        fprintf(stderr, "err %2d: a line on stderr\n", second);
        if (second == 15) {
            for (int i = 0; i < 60 * 1024 / 64; i++) {
                puts(line);
            }
            printf("burst done: 60 KiB\n");
        }
        ev3_sleep_ms(1000);
    }
    printf("done\n");
    return 0;
}
