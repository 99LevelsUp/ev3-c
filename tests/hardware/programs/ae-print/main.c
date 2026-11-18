/* Acceptance: endless printing is captured only up to the output cap and marked truncated. */
#include <stdio.h>

int main(void)
{
    for (unsigned long line = 1;; line++) {
        printf("line %lu: the quick brown fox jumps over the lazy dog\n", line);
    }
}
