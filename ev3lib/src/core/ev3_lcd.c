/*
 * ev3lib display. The facts it rests on, from the LEGO firmware sources
 * (1.09D developer edition):
 *   - c_ui/source/d_lcd.c:61-147, 179-196: the screen is /dev/fb0, 128 rows of
 *     fix.line_length (60) bytes. Each byte holds three pixels, the leftmost in
 *     bits 7-5, the next in bits 4-2, the third in bits 1-0, all of a pixel's
 *     bits set for black (the PixelTab table); 60 bytes cover 180 pixels, the
 *     last two beyond the 178 of the display;
 *   - c_ui/source/c_ui.c:1976-2125: while a program runs, the firmware draws a
 *     run animation about once a second and then writes its whole screen to
 *     the framebuffer, over anything else. Any UI_DRAW from the program's slot
 *     ends the animation (cUiDraw, c_ui.c:5690-5698). ev3lib cannot send one
 *     itself, so it asks EVƎ C's launcher: it creates ".lcd" in the project
 *     folder, and the launcher sends UI_DRAW UPDATE and removes ".lcd" as the
 *     answer;
 *   - d_lcd.c:752-951: the firmware's four fonts are XBM sheets of 16
 *     characters per row for the characters 0x20 to 0x7F, each character
 *     (width + 7) / 8 bytes wide, the leftmost pixel in the lowest bit:
 *     normal 8 by 9, small 8 by 8, large 16 by 16, tiny 5 by 7.
 *
 * The fonts are not copied into ev3lib: on first use it reads them from the
 * brick's own firmware, /home/root/lms2012/sys/lib/libc_ui.so, which keeps its
 * symbol table, through the symbols normal_font_bits, small_font_bits,
 * large_font_bits and tiny_font_bits.
 *
 * The picture is kept here as one byte per pixel and converted on each update.
 */
#include <errno.h>
#include <fcntl.h>
#include <linux/fb.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ioctl.h>
#include <unistd.h>

#include "ev3.h"
#include "ev3_core_internal.h"

#define FB_ROW_BYTES 60   /* d_lcd.c: (178 + 2) / 3 */
#define CLAIM_WAIT_MS 1000
#define TEXT_MAX 512

#define FONT_COUNT 4
#define FONT_SHEET_COLUMNS 16
#define FONT_FIRST 0x20
#define FONT_LAST 0x7F
#define FONT_SHEET_ROWS ((FONT_LAST - FONT_FIRST + 1) / FONT_SHEET_COLUMNS)
#define FONT_LARGEST (FONT_SHEET_COLUMNS * 2 * 16 * FONT_SHEET_ROWS)

/* d_lcd.c FontInfo, in the order of Ev3Font. */
static const struct {
    const char *symbol;
    int width;
    int height;
} font_facts[FONT_COUNT] = {
    { "normal_font_bits", 8, 9 },
    { "small_font_bits", 8, 8 },
    { "large_font_bits", 16, 16 },
    { "tiny_font_bits", 5, 7 },
};

static unsigned char font_bits[FONT_COUNT][FONT_LARGEST];
static bool fonts_loaded;

static unsigned char screen[EV3_LCD_HEIGHT][EV3_LCD_WIDTH]; /* 1 is black */
static bool auto_update = true;
static Ev3Font font = EV3_FONT_NORMAL;
static int cursor_x, cursor_y;
static int fb_fd = -1;
static size_t fb_line = FB_ROW_BYTES;

/* --- the framebuffer ----------------------------------------------------------- */

/*
 * Asks the launcher to end the firmware's run animation and waits for the
 * answer. Outside a run started by EVƎ C there is no launcher to ask; when no
 * answer comes (an older launcher), the program draws anyway.
 */
static void claim_screen(void)
{
    const char *project = ev3_core_project_dir();
    char marker[1024];
    int fd;

    if (project == NULL || (size_t)snprintf(marker, sizeof marker, "%s/.lcd", project) >= sizeof marker) {
        return;
    }
    fd = open(marker, O_WRONLY | O_CREAT, 0644);
    if (fd < 0) {
        return;
    }
    close(fd);
    for (int waited = 0; waited < CLAIM_WAIT_MS && access(marker, F_OK) == 0; waited += 10) {
        ev3_core_sleep(10);
    }
}

static Ev3Result open_screen(void)
{
    struct fb_fix_screeninfo fix;

    if (fb_fd >= 0) {
        return EV3_OK;
    }
    claim_screen();
    fb_fd = open(ev3_core_lcd_device(), O_RDWR);
    if (fb_fd < 0) {
        return ev3_fail_errno(EV3_ERR_IO, "the display cannot be opened");
    }
    /* A plain file stands in for the display in tests; it has no screen info. */
    if (ioctl(fb_fd, FBIOGET_FSCREENINFO, &fix) == 0 && fix.line_length >= FB_ROW_BYTES
        && fix.line_length <= 4 * FB_ROW_BYTES) {
        fb_line = fix.line_length;
    }
    return EV3_OK;
}

static Ev3Result show(void)
{
    static unsigned char frame[EV3_LCD_HEIGHT * 4 * FB_ROW_BYTES];
    size_t size;
    Ev3Result result = open_screen();

    if (result != EV3_OK) {
        return result;
    }
    size = fb_line * EV3_LCD_HEIGHT;
    memset(frame, 0, size);
    for (int y = 0; y < EV3_LCD_HEIGHT; y++) {
        unsigned char *row = frame + (size_t)y * fb_line;
        for (int x = 0; x < EV3_LCD_WIDTH; x++) {
            if (screen[y][x]) {
                static const unsigned char bits[3] = { 0xE0, 0x1C, 0x03 };
                row[x / 3] |= bits[x % 3];
            }
        }
    }
    if (pwrite(fb_fd, frame, size, 0) != (ssize_t)size) {
        return ev3_fail_errno(EV3_ERR_IO, "the display cannot be written");
    }
    return EV3_OK;
}

/* Ends a drawing function: shows the picture when auto update is on. */
static Ev3Result drawn(void)
{
    return auto_update ? show() : EV3_OK;
}

/* --- pixels and shapes ----------------------------------------------------------- */

static void put(int x, int y, unsigned char black)
{
    if (x >= 0 && x < EV3_LCD_WIDTH && y >= 0 && y < EV3_LCD_HEIGHT) {
        screen[y][x] = black;
    }
}

static void span(int x1, int x2, int y, unsigned char black)
{
    if (y < 0 || y >= EV3_LCD_HEIGHT) {
        return;
    }
    x1 = x1 < 0 ? 0 : x1;
    x2 = x2 >= EV3_LCD_WIDTH ? EV3_LCD_WIDTH - 1 : x2;
    for (int x = x1; x <= x2; x++) {
        screen[y][x] = black;
    }
}

static Ev3Result check_size(const char *what, int w, int h)
{
    if (w < 0 || h < 0) {
        return ev3_fail(EV3_ERR_PARAM, "%s %d by %d: width and height cannot be negative", what, w, h);
    }
    return EV3_OK;
}

static Ev3Result check_radius(int r)
{
    if (r < 0) {
        return ev3_fail(EV3_ERR_PARAM, "radius %d is negative", r);
    }
    return EV3_OK;
}

/* Limits a coordinate so arithmetic on it cannot overflow; the screen cuts the rest off. */
static int bound(int value)
{
    return value < -100000 ? -100000 : value > 100000 ? 100000 : value;
}

/* What a rectangle function does to each pixel inside. */
enum fill { FILL_BLACK, FILL_WHITE, FILL_INVERT };

static void fill(int x, int y, int w, int h, enum fill how)
{
    x = bound(x), y = bound(y), w = bound(w), h = bound(h);
    for (int row = y < 0 ? 0 : y; row < y + h && row < EV3_LCD_HEIGHT; row++) {
        for (int col = x < 0 ? 0 : x; col < x + w && col < EV3_LCD_WIDTH; col++) {
            screen[row][col] = how == FILL_BLACK ? 1 : how == FILL_WHITE ? 0 : !screen[row][col];
        }
    }
}

/* --- text ---------------------------------------------------------------------- */

/* The next UTF-8 character of *text; a broken sequence reads as U+FFFD. */
static unsigned long next_code(const char **text)
{
    const unsigned char *s = (const unsigned char *)*text;
    unsigned long code;
    int more;

    if (s[0] < 0x80) {
        *text += 1;
        return s[0];
    }
    if ((s[0] & 0xE0) == 0xC0) {
        code = s[0] & 0x1F, more = 1;
    } else if ((s[0] & 0xF0) == 0xE0) {
        code = s[0] & 0x0F, more = 2;
    } else if ((s[0] & 0xF8) == 0xF0) {
        code = s[0] & 0x07, more = 3;
    } else {
        *text += 1;
        return 0xFFFD;
    }
    for (int i = 1; i <= more; i++) {
        if ((s[i] & 0xC0) != 0x80) {
            *text += i;
            return 0xFFFD;
        }
        code = code << 6 | (s[i] & 0x3F);
    }
    *text += more + 1;
    return code;
}

static unsigned long le16(const unsigned char *p)
{
    return (unsigned long)p[0] | (unsigned long)p[1] << 8;
}

static unsigned long le32(const unsigned char *p)
{
    return le16(p) | le16(p + 2) << 16;
}

/* The bytes of a font sheet: one row of characters is FONT_SHEET_COLUMNS cells wide. */
static size_t sheet_size(int index)
{
    return (size_t)FONT_SHEET_COLUMNS * ((font_facts[index].width + 7) / 8) * font_facts[index].height * FONT_SHEET_ROWS;
}

/*
 * Finds the four font sheets through the symbol table of the firmware's
 * libc_ui.so (a 32-bit little-endian ELF file) and copies them. Every symbol
 * must have the size its font's FontInfo gives.
 */
static Ev3Result load_fonts_from(const unsigned char *elf, size_t size)
{
    unsigned long shoff, shentsize, shnum;
    const unsigned char *symtab = NULL, *strtab;
    unsigned long symtab_size = 0, strtab_size;
    int found = 0;

    if (size < 52 || memcmp(elf, "\177ELF\001\001", 6) != 0) {
        return EV3_ERR_DEVICE;
    }
    shoff = le32(elf + 0x20), shentsize = le16(elf + 0x2E), shnum = le16(elf + 0x30);
    if (shentsize < 40 || shoff > size || shnum > (size - shoff) / shentsize) {
        return EV3_ERR_DEVICE;
    }
    for (unsigned long i = 0; i < shnum; i++) {
        const unsigned char *section = elf + shoff + i * shentsize;
        unsigned long link = le32(section + 24);
        const unsigned char *strings;
        if (le32(section + 4) != 2 /* SHT_SYMTAB */ || link >= shnum) {
            continue;
        }
        strings = elf + shoff + link * shentsize;
        if (le32(section + 16) > size || le32(section + 20) > size - le32(section + 16)
            || le32(strings + 16) > size || le32(strings + 20) > size - le32(strings + 16)) {
            return EV3_ERR_DEVICE;
        }
        symtab = elf + le32(section + 16), symtab_size = le32(section + 20);
        strtab = elf + le32(strings + 16), strtab_size = le32(strings + 20);
        for (unsigned long at = 0; at + 16 <= symtab_size; at += 16) {
            const unsigned char *symbol = symtab + at;
            unsigned long name = le32(symbol), value = le32(symbol + 4), length = le32(symbol + 8);
            unsigned long shndx = le16(symbol + 14);
            const unsigned char *home;
            unsigned long offset;
            if (name >= strtab_size || shndx == 0 || shndx >= shnum) {
                continue;
            }
            for (int f = 0; f < FONT_COUNT; f++) {
                const char *wanted = font_facts[f].symbol;
                if (strnlen((const char *)strtab + name, strtab_size - name) != strlen(wanted)
                    || strcmp((const char *)strtab + name, wanted) != 0) {
                    continue;
                }
                home = elf + shoff + shndx * shentsize;
                offset = le32(home + 16) + (value - le32(home + 12));
                if (length != sheet_size(f) || value < le32(home + 12) || offset > size || length > size - offset) {
                    return EV3_ERR_DEVICE;
                }
                memcpy(font_bits[f], elf + offset, length);
                found |= 1 << f;
            }
        }
    }
    return symtab != NULL && found == (1 << FONT_COUNT) - 1 ? EV3_OK : EV3_ERR_DEVICE;
}

static Ev3Result load_fonts(void)
{
    const char *path = ev3_core_ui_library();
    unsigned char *elf;
    long size;
    FILE *file;
    Ev3Result result;

    if (fonts_loaded) {
        return EV3_OK;
    }
    file = fopen(path, "rb");
    if (file == NULL) {
        return ev3_fail(EV3_ERR_DEVICE, "the brick's fonts cannot be read: %s: %s", path, strerror(errno));
    }
    if (fseek(file, 0, SEEK_END) != 0 || (size = ftell(file)) <= 0 || fseek(file, 0, SEEK_SET) != 0
        || (elf = malloc((size_t)size)) == NULL) {
        fclose(file);
        return ev3_fail(EV3_ERR_DEVICE, "the brick's fonts cannot be read from %s", path);
    }
    result = fread(elf, 1, (size_t)size, file) == (size_t)size ? load_fonts_from(elf, (size_t)size) : EV3_ERR_DEVICE;
    fclose(file);
    free(elf);
    if (result != EV3_OK) {
        return ev3_fail(EV3_ERR_DEVICE, "the brick's fonts were not found in %s; is this the LEGO firmware?", path);
    }
    fonts_loaded = true;
    return EV3_OK;
}

/*
 * Draws one character cell, black on white, with its top left corner at x, y,
 * as d_lcd.c dLcdDrawChar does. A character the fonts do not have shows as '?'.
 */
static void draw_char(int x, int y, unsigned long code)
{
    int width = font_facts[font].width, height = font_facts[font].height;
    int bytes = (width + 7) / 8;
    const unsigned char *cell;
    unsigned long index;

    if (code < FONT_FIRST || code > FONT_LAST) {
        code = '?';
    }
    index = code - FONT_FIRST;
    cell = font_bits[font] + (index % FONT_SHEET_COLUMNS) * bytes
        + (index / FONT_SHEET_COLUMNS) * bytes * height * FONT_SHEET_COLUMNS;
    for (int row = 0; row < height; row++) {
        const unsigned char *line = cell + (size_t)row * bytes * FONT_SHEET_COLUMNS;
        for (int col = 0; col < width; col++) {
            put(x + col, y + row, (line[col / 8] >> (col % 8)) & 1);
        }
    }
}

/* Moves everything up by `rows` pixels and clears the rows that come free at the bottom. */
static void scroll(int rows)
{
    if (rows >= EV3_LCD_HEIGHT) {
        memset(screen, 0, sizeof screen);
        return;
    }
    memmove(screen[0], screen[rows], (size_t)(EV3_LCD_HEIGHT - rows) * EV3_LCD_WIDTH);
    memset(screen[EV3_LCD_HEIGHT - rows], 0, (size_t)rows * EV3_LCD_WIDTH);
}

/* Starts a new line at the cursor, scrolling when it would not fit. */
static void new_line(void)
{
    int height = font_facts[font].height;

    cursor_x = 0;
    cursor_y += height;
    if (cursor_y + height > EV3_LCD_HEIGHT) {
        scroll(cursor_y + height - EV3_LCD_HEIGHT);
        cursor_y = EV3_LCD_HEIGHT - height;
    }
}

static Ev3Result check_text(const char *text)
{
    if (text == NULL) {
        return ev3_fail(EV3_ERR_PARAM, "text points nowhere (NULL)");
    }
    return EV3_OK;
}

static Ev3Result text_at(int x, int y, const char *text)
{
    const int width = font_facts[font].width, height = font_facts[font].height;
    int left;
    Ev3Result result = load_fonts();

    if (result != EV3_OK) {
        return result;
    }
    x = bound(x), y = bound(y), left = x;
    while (*text != '\0') {
        unsigned long code = next_code(&text);
        if (code == '\n') {
            x = left;
            y += height;
            continue;
        }
        if (x < EV3_LCD_WIDTH) {
            draw_char(x, y, code);
        }
        x += width;
    }
    return EV3_OK;
}

static Ev3Result text_at_cursor(const char *text)
{
    const int width = font_facts[font].width;
    Ev3Result result = load_fonts();

    if (result != EV3_OK) {
        return result;
    }
    while (*text != '\0') {
        unsigned long code = next_code(&text);
        if (code == '\n') {
            new_line();
            continue;
        }
        if (cursor_x + width > EV3_LCD_WIDTH) {
            new_line();
        }
        draw_char(cursor_x, cursor_y, code);
        cursor_x += width;
    }
    return EV3_OK;
}

/* Formats text as printf() does; a longer result is cut at TEXT_MAX - 1 bytes. */
static Ev3Result format_text(char *out, const char *format, va_list args)
{
    if (format == NULL) {
        return ev3_fail(EV3_ERR_PARAM, "format points nowhere (NULL)");
    }
    if (vsnprintf(out, TEXT_MAX, format, args) < 0) {
        return ev3_fail(EV3_ERR_PARAM, "the format \"%s\" cannot be used", format);
    }
    return EV3_OK;
}

/* --- try variants ---------------------------------------------------------------- */

Ev3Result ev3_lcd_try_clear(void)
{
    memset(screen, 0, sizeof screen);
    cursor_x = cursor_y = 0;
    return drawn();
}

Ev3Result ev3_lcd_try_update(void)
{
    return show();
}

Ev3Result ev3_lcd_try_set_auto_update(bool on)
{
    auto_update = on;
    return EV3_OK;
}

Ev3Result ev3_lcd_try_set_font(Ev3Font new_font)
{
    if ((int)new_font < 0 || (int)new_font >= FONT_COUNT) {
        return ev3_fail(EV3_ERR_PARAM, "font %d is none of EV3_FONT_NORMAL, _SMALL, _LARGE, _TINY", (int)new_font);
    }
    font = new_font;
    return EV3_OK;
}

Ev3Result ev3_lcd_try_draw_text(int x, int y, const char *text)
{
    Ev3Result result = check_text(text);

    if (result == EV3_OK) {
        result = text_at(x, y, text);
    }
    return result == EV3_OK ? drawn() : result;
}

static Ev3Result draw_textv(int x, int y, const char *format, va_list args)
{
    char text[TEXT_MAX];
    Ev3Result result = format_text(text, format, args);

    if (result == EV3_OK) {
        result = text_at(x, y, text);
    }
    return result == EV3_OK ? drawn() : result;
}

Ev3Result ev3_lcd_try_draw_textf(int x, int y, const char *format, ...)
{
    va_list args;
    Ev3Result result;

    va_start(args, format);
    result = draw_textv(x, y, format, args);
    va_end(args);
    return result;
}

static Ev3Result printv(bool line_after, const char *format, va_list args)
{
    char text[TEXT_MAX];
    Ev3Result result = format_text(text, format, args);

    if (result == EV3_OK) {
        result = text_at_cursor(text);
    }
    if (result != EV3_OK) {
        return result;
    }
    if (line_after) {
        new_line();
    }
    return drawn();
}

Ev3Result ev3_lcd_try_print(const char *format, ...)
{
    va_list args;
    Ev3Result result;

    va_start(args, format);
    result = printv(false, format, args);
    va_end(args);
    return result;
}

Ev3Result ev3_lcd_try_println(const char *format, ...)
{
    va_list args;
    Ev3Result result;

    va_start(args, format);
    result = printv(true, format, args);
    va_end(args);
    return result;
}

Ev3Result ev3_lcd_try_set_cursor(int x, int y)
{
    if (x < 0 || x >= EV3_LCD_WIDTH || y < 0 || y >= EV3_LCD_HEIGHT) {
        return ev3_fail(EV3_ERR_PARAM, "cursor %d, %d is off the display (x 0 to %d, y 0 to %d)", x, y,
                        EV3_LCD_WIDTH - 1, EV3_LCD_HEIGHT - 1);
    }
    cursor_x = x;
    cursor_y = y;
    return EV3_OK;
}

Ev3Result ev3_lcd_try_draw_pixel(int x, int y, bool black)
{
    put(x, y, black ? 1 : 0);
    return drawn();
}

Ev3Result ev3_lcd_try_draw_line(int x1, int y1, int x2, int y2)
{
    int dx, dy, sx, sy, error;

    x1 = bound(x1), y1 = bound(y1), x2 = bound(x2), y2 = bound(y2);
    dx = abs(x2 - x1), dy = -abs(y2 - y1);
    sx = x1 < x2 ? 1 : -1, sy = y1 < y2 ? 1 : -1;
    error = dx + dy;
    for (;;) {
        put(x1, y1, 1);
        if (x1 == x2 && y1 == y2) {
            break;
        }
        if (2 * error >= dy) {
            error += dy;
            x1 += sx;
        }
        if (2 * error <= dx) {
            error += dx;
            y1 += sy;
        }
    }
    return drawn();
}

Ev3Result ev3_lcd_try_draw_rect(int x, int y, int w, int h)
{
    Ev3Result result = check_size("rectangle", w, h);

    if (result != EV3_OK || w == 0 || h == 0) {
        return result == EV3_OK ? drawn() : result;
    }
    fill(x, y, w, 1, FILL_BLACK);
    fill(x, bound(y) + bound(h) - 1, w, 1, FILL_BLACK);
    fill(x, y, 1, h, FILL_BLACK);
    fill(bound(x) + bound(w) - 1, y, 1, h, FILL_BLACK);
    return drawn();
}

static Ev3Result rect(int x, int y, int w, int h, enum fill how)
{
    Ev3Result result = check_size("rectangle", w, h);

    if (result != EV3_OK) {
        return result;
    }
    fill(x, y, w, h, how);
    return drawn();
}

Ev3Result ev3_lcd_try_fill_rect(int x, int y, int w, int h)
{
    return rect(x, y, w, h, FILL_BLACK);
}

Ev3Result ev3_lcd_try_clear_rect(int x, int y, int w, int h)
{
    return rect(x, y, w, h, FILL_WHITE);
}

Ev3Result ev3_lcd_try_invert_rect(int x, int y, int w, int h)
{
    return rect(x, y, w, h, FILL_INVERT);
}

/* Midpoint circle: eight mirrored points, or four spans for a filled one, per step. */
static void circle(int cx, int cy, int r, bool filled)
{
    int x, y = 0, error;

    cx = bound(cx), cy = bound(cy), r = bound(r);
    x = r, error = 1 - r;
    while (x >= y) {
        if (filled) {
            span(cx - x, cx + x, cy + y, 1);
            span(cx - x, cx + x, cy - y, 1);
            span(cx - y, cx + y, cy + x, 1);
            span(cx - y, cx + y, cy - x, 1);
        } else {
            put(cx + x, cy + y, 1), put(cx - x, cy + y, 1), put(cx + x, cy - y, 1), put(cx - x, cy - y, 1);
            put(cx + y, cy + x, 1), put(cx - y, cy + x, 1), put(cx + y, cy - x, 1), put(cx - y, cy - x, 1);
        }
        y++;
        if (error < 0) {
            error += 2 * y + 1;
        } else {
            x--;
            error += 2 * (y - x) + 1;
        }
    }
}

Ev3Result ev3_lcd_try_draw_circle(int cx, int cy, int r)
{
    Ev3Result result = check_radius(r);

    if (result != EV3_OK) {
        return result;
    }
    circle(cx, cy, r, false);
    return drawn();
}

Ev3Result ev3_lcd_try_fill_circle(int cx, int cy, int r)
{
    Ev3Result result = check_radius(r);

    if (result != EV3_OK) {
        return result;
    }
    circle(cx, cy, r, true);
    return drawn();
}

/* Rows of (w + 7) / 8 bytes, the leftmost pixel in the lowest bit. */
static void bitmap(int x, int y, int w, int h, const uint8_t *data)
{
    int stride = (w + 7) / 8;

    x = bound(x), y = bound(y);
    for (int row = 0; row < h; row++) {
        for (int col = 0; col < w; col++) {
            put(x + col, y + row, (data[row * stride + col / 8] >> (col % 8)) & 1);
        }
    }
}

Ev3Result ev3_lcd_try_draw_xbm(int x, int y, int w, int h, const uint8_t *data)
{
    Ev3Result result = check_size("picture", w, h);

    if (result != EV3_OK) {
        return result;
    }
    if (data == NULL) {
        return ev3_fail(EV3_ERR_PARAM, "the picture's data points nowhere (NULL)");
    }
    bitmap(x, y, w, h, data);
    return drawn();
}

Ev3Result ev3_lcd_try_draw_image(int x, int y, const char *path)
{
    /* The largest RGF: 255 by 255 pixels. */
    static uint8_t data[2 + 32 * 255 + 1];
    size_t length;
    FILE *file;

    if (path == NULL) {
        return ev3_fail(EV3_ERR_PARAM, "the picture's path points nowhere (NULL)");
    }
    file = fopen(path, "rb");
    if (file == NULL) {
        return ev3_fail(EV3_ERR_IO, "the picture %s cannot be opened: %s", path, strerror(errno));
    }
    length = fread(data, 1, sizeof data, file);
    fclose(file);
    if (length < 2 || length != 2 + (size_t)(data[0] + 7) / 8 * data[1]) {
        return ev3_fail(EV3_ERR_PARAM, "%s is not an RGF picture (%zu bytes)", path, length);
    }
    bitmap(x, y, data[0], data[1], data + 2);
    return drawn();
}

/* --- plain variants -------------------------------------------------------------- */

void ev3_lcd_clear_at_(EV3_AT)
{
    EV3_MUST("ev3_lcd_clear", ev3_lcd_try_clear());
}

void ev3_lcd_update_at_(EV3_AT)
{
    EV3_MUST("ev3_lcd_update", ev3_lcd_try_update());
}

void ev3_lcd_set_auto_update_at_(EV3_AT, bool on)
{
    EV3_MUST("ev3_lcd_set_auto_update", ev3_lcd_try_set_auto_update(on));
}

void ev3_lcd_set_font_at_(EV3_AT, Ev3Font new_font)
{
    EV3_MUST("ev3_lcd_set_font", ev3_lcd_try_set_font(new_font));
}

void ev3_lcd_draw_text_at_(EV3_AT, int x, int y, const char *text)
{
    EV3_MUST("ev3_lcd_draw_text", ev3_lcd_try_draw_text(x, y, text));
}

void ev3_lcd_draw_textf_at_(EV3_AT, int x, int y, const char *format, ...)
{
    va_list args;

    va_start(args, format);
    ev3_error_clear();
    Ev3Result result = draw_textv(x, y, format, args);
    va_end(args);
    if (result != EV3_OK) {
        ev3_stop_on_error_(file_, line_, "ev3_lcd_draw_textf", result);
    }
}

void ev3_lcd_print_at_(EV3_AT, const char *format, ...)
{
    va_list args;

    va_start(args, format);
    ev3_error_clear();
    Ev3Result result = printv(false, format, args);
    va_end(args);
    if (result != EV3_OK) {
        ev3_stop_on_error_(file_, line_, "ev3_lcd_print", result);
    }
}

void ev3_lcd_println_at_(EV3_AT, const char *format, ...)
{
    va_list args;

    va_start(args, format);
    ev3_error_clear();
    Ev3Result result = printv(true, format, args);
    va_end(args);
    if (result != EV3_OK) {
        ev3_stop_on_error_(file_, line_, "ev3_lcd_println", result);
    }
}

void ev3_lcd_set_cursor_at_(EV3_AT, int x, int y)
{
    EV3_MUST("ev3_lcd_set_cursor", ev3_lcd_try_set_cursor(x, y));
}

void ev3_lcd_draw_pixel_at_(EV3_AT, int x, int y, bool black)
{
    EV3_MUST("ev3_lcd_draw_pixel", ev3_lcd_try_draw_pixel(x, y, black));
}

void ev3_lcd_draw_line_at_(EV3_AT, int x1, int y1, int x2, int y2)
{
    EV3_MUST("ev3_lcd_draw_line", ev3_lcd_try_draw_line(x1, y1, x2, y2));
}

void ev3_lcd_draw_rect_at_(EV3_AT, int x, int y, int w, int h)
{
    EV3_MUST("ev3_lcd_draw_rect", ev3_lcd_try_draw_rect(x, y, w, h));
}

void ev3_lcd_fill_rect_at_(EV3_AT, int x, int y, int w, int h)
{
    EV3_MUST("ev3_lcd_fill_rect", ev3_lcd_try_fill_rect(x, y, w, h));
}

void ev3_lcd_clear_rect_at_(EV3_AT, int x, int y, int w, int h)
{
    EV3_MUST("ev3_lcd_clear_rect", ev3_lcd_try_clear_rect(x, y, w, h));
}

void ev3_lcd_invert_rect_at_(EV3_AT, int x, int y, int w, int h)
{
    EV3_MUST("ev3_lcd_invert_rect", ev3_lcd_try_invert_rect(x, y, w, h));
}

void ev3_lcd_draw_circle_at_(EV3_AT, int cx, int cy, int r)
{
    EV3_MUST("ev3_lcd_draw_circle", ev3_lcd_try_draw_circle(cx, cy, r));
}

void ev3_lcd_fill_circle_at_(EV3_AT, int cx, int cy, int r)
{
    EV3_MUST("ev3_lcd_fill_circle", ev3_lcd_try_fill_circle(cx, cy, r));
}

void ev3_lcd_draw_xbm_at_(EV3_AT, int x, int y, int w, int h, const uint8_t *data)
{
    EV3_MUST("ev3_lcd_draw_xbm", ev3_lcd_try_draw_xbm(x, y, w, h, data));
}

void ev3_lcd_draw_image_at_(EV3_AT, int x, int y, const char *path)
{
    EV3_MUST("ev3_lcd_draw_image", ev3_lcd_try_draw_image(x, y, path));
}
