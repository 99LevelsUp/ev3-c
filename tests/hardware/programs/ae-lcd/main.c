/*
 * Acceptance: the display. Four screens, each shown until Enter is pressed
 * (a beep says the next one is up):
 *   1  the four fonts, each with its name
 *   2  shapes: a frame around the display, a line across, a circle, a filled
 *      circle, and a filled rectangle with a wider inverted band through it:
 *      white inside the rectangle, black where the band reaches past it
 *   3  a counter that counts to 50 in the large font, in place
 *   4  print and println: 30 numbered lines, so the display scrolls; the last
 *      line on the display is "line 30", the top line is whole, and a letter
 *      with a diacritic shows as "?"
 * It also prints to stdout what it drew.
 */
#include <stdio.h>

#include "ev3.h"

static void next(void)
{
    ev3_sound_play_tone(880, 120);
    ev3_button_wait_press(EV3_BUTTON_ENTER, 0);
    ev3_button_wait_release(EV3_BUTTON_ENTER);
    ev3_lcd_clear();
}

int main(void)
{
    ev3_lcd_clear();
    ev3_lcd_set_font(EV3_FONT_LARGE);
    ev3_lcd_draw_text(0, 0, "LARGE");
    ev3_lcd_set_font(EV3_FONT_NORMAL);
    ev3_lcd_draw_text(0, 20, "NORMAL font 8x9");
    ev3_lcd_set_font(EV3_FONT_SMALL);
    ev3_lcd_draw_text(0, 34, "SMALL font 8x8");
    ev3_lcd_set_font(EV3_FONT_TINY);
    ev3_lcd_draw_text(0, 48, "TINY font 5x7, 35 characters a line");
    ev3_lcd_set_font(EV3_FONT_NORMAL);
    ev3_lcd_draw_textf(0, 70, "%d + %d = %d", 2, 3, 2 + 3);
    printf("screen 1: fonts\n");
    next();

    ev3_lcd_set_auto_update(false);
    ev3_lcd_draw_rect(0, 0, EV3_LCD_WIDTH, EV3_LCD_HEIGHT);
    ev3_lcd_draw_line(0, 0, EV3_LCD_WIDTH - 1, EV3_LCD_HEIGHT - 1);
    ev3_lcd_draw_circle(45, 64, 30);
    ev3_lcd_fill_circle(130, 40, 18);
    ev3_lcd_fill_rect(100, 80, 60, 30);
    ev3_lcd_invert_rect(90, 90, 80, 10);
    ev3_lcd_update();
    ev3_lcd_set_auto_update(true);
    printf("screen 2: shapes\n");
    next();

    ev3_lcd_set_font(EV3_FONT_NORMAL);
    ev3_lcd_draw_text(0, 0, "Counting:");
    ev3_lcd_set_font(EV3_FONT_LARGE);
    for (int i = 1; i <= 50; i++) {
        ev3_lcd_draw_textf(40, 50, "%3d", i);
        ev3_sleep_ms(60);
    }
    printf("screen 3: counter\n");
    next();

    ev3_lcd_set_font(EV3_FONT_NORMAL);
    ev3_lcd_print("Hello, ");
    ev3_lcd_println("robot! \xC5\x99"); /* "ř" shows as '?' */
    for (int i = 2; i <= 30; i++) {
        ev3_lcd_println("line %d", i);
        ev3_sleep_ms(100);
    }
    printf("screen 4: print and scrolling\n");
    next();
    return 0;
}
