/*
 * ev3lib display: 178 by 128 pixels, black on white. x runs from 0 (left) to
 * 177, y from 0 (top) to 127.
 *
 * Drawing goes into a picture in memory. By default every drawing function
 * then shows the picture at once; ev3_lcd_set_auto_update(false) turns that
 * off, and ev3_lcd_update() shows the picture when the program is ready.
 * Anything drawn partly or wholly outside the display is cut off at its edge.
 *
 * Text uses the brick's own fonts, which ev3lib reads from the firmware on the
 * brick. They have the ASCII characters only: any other character, such as a
 * letter with a diacritic, shows as one '?'.
 *
 * Each function comes as a try variant, which returns an Ev3Result, and as a
 * plain variant, which stops the program on an error (see ev3_error.h).
 */
#ifndef EV3_LCD_H
#define EV3_LCD_H

#include <stdbool.h>
#include <stdint.h>

#include "ev3_error.h"

#ifdef __cplusplus
extern "C" {
#endif

#define EV3_LCD_WIDTH 178
#define EV3_LCD_HEIGHT 128

/*
 * The brick's own fonts, as the firmware shows its menus. Each character takes
 * a cell of the same size.
 */
typedef enum {
    EV3_FONT_NORMAL = 0, /* 8 by 9 pixels: 22 characters by 14 lines */
    EV3_FONT_SMALL = 1,  /* 8 by 8 pixels: 22 characters by 16 lines */
    EV3_FONT_LARGE = 2,  /* 16 by 16 pixels: 11 characters by 8 lines */
    EV3_FONT_TINY = 3,   /* 5 by 7 pixels: 35 characters by 18 lines */
} Ev3Font;

#if defined(__GNUC__) || defined(__clang__)
#define EV3_PRINTF_LIKE(format_index) __attribute__((format(printf, format_index, format_index + 1)))
#else
#define EV3_PRINTF_LIKE(format_index)
#endif

/* --- the whole display -------------------------------------------------------- */

/* Makes the display white and puts the text cursor in the top left corner. */
Ev3Result ev3_lcd_try_clear(void);
void ev3_lcd_clear_at_(const char *file, int line);
#define ev3_lcd_clear() ev3_lcd_clear_at_(__FILE__, __LINE__)

/* Shows what was drawn. Needed only after ev3_lcd_set_auto_update(false). */
Ev3Result ev3_lcd_try_update(void);
void ev3_lcd_update_at_(const char *file, int line);
#define ev3_lcd_update() ev3_lcd_update_at_(__FILE__, __LINE__)

/*
 * Whether every drawing function shows its result at once (true, the start)
 * or only ev3_lcd_update() does (false), for a picture drawn in many steps
 * that should appear whole.
 */
Ev3Result ev3_lcd_try_set_auto_update(bool on);
void ev3_lcd_set_auto_update_at_(const char *file, int line, bool on);
#define ev3_lcd_set_auto_update(on) ev3_lcd_set_auto_update_at_(__FILE__, __LINE__, (on))

/* --- text --------------------------------------------------------------------- */

/* The font for the text drawn from now on; EV3_FONT_NORMAL at the start. */
Ev3Result ev3_lcd_try_set_font(Ev3Font font);
void ev3_lcd_set_font_at_(const char *file, int line, Ev3Font font);
#define ev3_lcd_set_font(font) ev3_lcd_set_font_at_(__FILE__, __LINE__, (font))

/*
 * Draws text with its top left corner at x, y. The characters' cells are
 * drawn whole, white around black, so new text covers the old. A line break
 * continues one line lower at x.
 */
Ev3Result ev3_lcd_try_draw_text(int x, int y, const char *text);
void ev3_lcd_draw_text_at_(const char *file, int line, int x, int y, const char *text);
#define ev3_lcd_draw_text(x, y, text) ev3_lcd_draw_text_at_(__FILE__, __LINE__, (x), (y), (text))

/* As ev3_lcd_draw_text(), with the text made as printf() makes it. */
Ev3Result ev3_lcd_try_draw_textf(int x, int y, const char *format, ...) EV3_PRINTF_LIKE(3);
void ev3_lcd_draw_textf_at_(const char *file, int line, int x, int y, const char *format, ...) EV3_PRINTF_LIKE(5);
#define ev3_lcd_draw_textf(x, y, ...) ev3_lcd_draw_textf_at_(__FILE__, __LINE__, (x), (y), __VA_ARGS__)

/*
 * Writes text at the text cursor as printf() makes it and moves the cursor on,
 * like a terminal: a long line wraps, and when the text reaches the bottom the
 * display scrolls up. ev3_lcd_println() then starts a new line.
 */
Ev3Result ev3_lcd_try_print(const char *format, ...) EV3_PRINTF_LIKE(1);
void ev3_lcd_print_at_(const char *file, int line, const char *format, ...) EV3_PRINTF_LIKE(3);
#define ev3_lcd_print(...) ev3_lcd_print_at_(__FILE__, __LINE__, __VA_ARGS__)

Ev3Result ev3_lcd_try_println(const char *format, ...) EV3_PRINTF_LIKE(1);
void ev3_lcd_println_at_(const char *file, int line, const char *format, ...) EV3_PRINTF_LIKE(3);
#define ev3_lcd_println(...) ev3_lcd_println_at_(__FILE__, __LINE__, __VA_ARGS__)

/* Puts the text cursor at x, y: ev3_lcd_set_cursor(0, 0) is the top left corner. */
Ev3Result ev3_lcd_try_set_cursor(int x, int y);
void ev3_lcd_set_cursor_at_(const char *file, int line, int x, int y);
#define ev3_lcd_set_cursor(x, y) ev3_lcd_set_cursor_at_(__FILE__, __LINE__, (x), (y))

/* --- drawing ------------------------------------------------------------------ */

/* One pixel, black (true) or white (false). */
Ev3Result ev3_lcd_try_draw_pixel(int x, int y, bool black);
void ev3_lcd_draw_pixel_at_(const char *file, int line, int x, int y, bool black);
#define ev3_lcd_draw_pixel(x, y, black) ev3_lcd_draw_pixel_at_(__FILE__, __LINE__, (x), (y), (black))

/* A black line from x1, y1 to x2, y2. */
Ev3Result ev3_lcd_try_draw_line(int x1, int y1, int x2, int y2);
void ev3_lcd_draw_line_at_(const char *file, int line, int x1, int y1, int x2, int y2);
#define ev3_lcd_draw_line(x1, y1, x2, y2) ev3_lcd_draw_line_at_(__FILE__, __LINE__, (x1), (y1), (x2), (y2))

/*
 * Rectangles with the top left corner at x, y, width w and height h. draw
 * draws the black outline, fill fills it black, clear fills it white, and
 * invert swaps black and white inside it.
 */
Ev3Result ev3_lcd_try_draw_rect(int x, int y, int w, int h);
void ev3_lcd_draw_rect_at_(const char *file, int line, int x, int y, int w, int h);
#define ev3_lcd_draw_rect(x, y, w, h) ev3_lcd_draw_rect_at_(__FILE__, __LINE__, (x), (y), (w), (h))

Ev3Result ev3_lcd_try_fill_rect(int x, int y, int w, int h);
void ev3_lcd_fill_rect_at_(const char *file, int line, int x, int y, int w, int h);
#define ev3_lcd_fill_rect(x, y, w, h) ev3_lcd_fill_rect_at_(__FILE__, __LINE__, (x), (y), (w), (h))

Ev3Result ev3_lcd_try_clear_rect(int x, int y, int w, int h);
void ev3_lcd_clear_rect_at_(const char *file, int line, int x, int y, int w, int h);
#define ev3_lcd_clear_rect(x, y, w, h) ev3_lcd_clear_rect_at_(__FILE__, __LINE__, (x), (y), (w), (h))

Ev3Result ev3_lcd_try_invert_rect(int x, int y, int w, int h);
void ev3_lcd_invert_rect_at_(const char *file, int line, int x, int y, int w, int h);
#define ev3_lcd_invert_rect(x, y, w, h) ev3_lcd_invert_rect_at_(__FILE__, __LINE__, (x), (y), (w), (h))

/* Circles around cx, cy with radius r: the black outline, or filled black. */
Ev3Result ev3_lcd_try_draw_circle(int cx, int cy, int r);
void ev3_lcd_draw_circle_at_(const char *file, int line, int cx, int cy, int r);
#define ev3_lcd_draw_circle(cx, cy, r) ev3_lcd_draw_circle_at_(__FILE__, __LINE__, (cx), (cy), (r))

Ev3Result ev3_lcd_try_fill_circle(int cx, int cy, int r);
void ev3_lcd_fill_circle_at_(const char *file, int line, int cx, int cy, int r);
#define ev3_lcd_fill_circle(cx, cy, r) ev3_lcd_fill_circle_at_(__FILE__, __LINE__, (cx), (cy), (r))

/* --- pictures ----------------------------------------------------------------- */

/*
 * An XBM picture from the program itself, w by h pixels with the top left
 * corner at x, y: rows of (w + 7) / 8 bytes, the leftmost pixel in the lowest
 * bit, 1 black and 0 white. A .xbm file saved by an image editor is C code
 * that gives exactly this array.
 */
Ev3Result ev3_lcd_try_draw_xbm(int x, int y, int w, int h, const uint8_t *data);
void ev3_lcd_draw_xbm_at_(const char *file, int line, int x, int y, int w, int h, const uint8_t *data);
#define ev3_lcd_draw_xbm(x, y, w, h, data) ev3_lcd_draw_xbm_at_(__FILE__, __LINE__, (x), (y), (w), (h), (data))

/*
 * An RGF picture file (the EV3 software's format: one byte width, one byte
 * height, then rows as in XBM) with the top left corner at x, y. A relative
 * path is found from the project folder on the brick.
 */
Ev3Result ev3_lcd_try_draw_image(int x, int y, const char *path);
void ev3_lcd_draw_image_at_(const char *file, int line, int x, int y, const char *path);
#define ev3_lcd_draw_image(x, y, path) ev3_lcd_draw_image_at_(__FILE__, __LINE__, (x), (y), (path))

#ifdef __cplusplus
}
#endif

#endif /* EV3_LCD_H */
