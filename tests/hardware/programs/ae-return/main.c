/* Acceptance: a program hands files back; EVƎ C downloads them after the run. */
#include <stdio.h>

#include "ev3.h"

int main(void)
{
    /* Opened through ev3lib: marked for return as it is created. */
    FILE *log = ev3_return_fopen("log.csv", "w");
    FILE *note;

    if (log == NULL) {
        perror("log.csv");
        return 1;
    }
    for (int i = 1; i <= 3; i++) {
        fprintf(log, "%d,%d\n", i, i * i);
    }
    fclose(log);

    /* Created by stdio, marked afterwards. */
    note = fopen("note.txt", "w");
    if (note == NULL || fputs("returned by ae-return\n", note) < 0 || fclose(note) != 0 || ev3_return_file("note.txt") != 0) {
        perror("note.txt");
        return 2;
    }
    printf("returned two files\n");
    return 0;
}
