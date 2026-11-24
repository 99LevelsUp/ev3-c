# ev3lib: C on the EV3 brick

ev3lib is the C library EVƎ C links into every program. `#include "ev3.h"` gives all of it.

## Two variants of every function

Every function comes twice:

- **Plain**, for most programs: `ev3_motor_start(EV3_PORT_A, 50);`.
  - It returns the value it reads, or nothing.
  - When something goes wrong, it prints where and why to stderr and ends the program with exit code 70. The motors stop, as at every end of a program:

    ```
    main.c:12: ev3_motor_start: port 1 is a sensor port; motors are on ports A to D
    ev3lib stopped the program because of this error.
    ```

    `ev3 c run` shows this line as how the run ended (`result.libraryError` with `--json`).
- **Try**, for a program that handles errors itself: `ev3_motor_try_start(EV3_PORT_A, 50)`.
  - It always returns an `Ev3Result` (`EV3_OK` or an error code).
  - A value it reads comes back through a pointer: `ev3_sensor_try_read_reflected(EV3_PORT_2, &percent)`.
  - `ev3_error_message()` says what went wrong, and `ev3_error_string()` names the code.

The try variant puts `try_` between the module and the action: `ev3_motor_try_start`, `ev3_sensor_try_read_color`.

## Ports

| Ports | Values | Can be combined |
|---|---|---|
| Motors A to D | `EV3_PORT_A` … `EV3_PORT_D` | Yes, with `\|` for commands that drive several motors at once: `EV3_PORT_B \| EV3_PORT_C`. |
| Sensors 1 to 4 | `EV3_PORT_1` … `EV3_PORT_4` | No |

Motor and sensor ports have values of their own, so a sensor port given to a motor is caught and named.

## Motors (`ev3/ev3_motor.h`)

| Function | What it does |
|---|---|
| `ev3_motor_start(ports, speed)` | Runs the motors at a regulated speed from -100 to 100 until they are stopped. |
| `ev3_motor_set_power(ports, power)` | Runs them at a power without regulation. |
| `ev3_motor_stop(ports, EV3_STOP_BRAKE \| EV3_STOP_COAST)` | Stops them. |
| `ev3_motor_run_degrees(ports, speed, degrees)` | Turns them by an amount and returns when they are done; they brake at the end. |
| `ev3_motor_run_rotations(ports, speed, rotations)` | The same, in rotations. |
| `ev3_motor_run_seconds(ports, speed, seconds)` | The same, for a time. |
| `ev3_motor_start_degrees(ports, speed, degrees)`<br>`ev3_motor_start_seconds(ports, speed, seconds)` | Start the same commands and return at once. `ev3_motor_wait(ports, timeout_ms)` waits for the end and `ev3_motor_is_busy(ports)` asks whether it has come. |
| `ev3_motor_get_position(port)`<br>`ev3_motor_get_speed(port)`<br>`ev3_motor_reset_position(ports)` | Read and reset a motor. |
| `ev3_motor_set_polarity(ports, 1 \| -1 \| 0)` | Sets which way a positive speed turns. |
| `ev3_motor_run_degrees_ramp(ports, speed, up, steady, down, mode)`<br>`ev3_motor_run_seconds_ramp(...)` | Turns them with a smooth start and end: speeding up over `up`, steady over `steady`, slowing down over `down`, then stopping as `mode` says. `ev3_motor_start_degrees_ramp` and `ev3_motor_start_seconds_ramp` return at once. |

### Two motors together

The `ev3_motor_sync_*` functions drive two motors as a pair, such as the wheels of a robot. The firmware keeps them in step, so the robot goes straight or turns evenly.

```c
ev3_motor_sync_run_rotations(EV3_PORT_B, EV3_PORT_C, 50, 0, 2.0f);   /* two turns straight on */
ev3_motor_sync_run_degrees(EV3_PORT_B, EV3_PORT_C, 40, 100, 360);    /* around the right wheel */
```

`left` and `right` are one port each. `turn` goes from -200 to 200 and says how the right motor follows the left one: 0 straight on, 50 the right motor at half speed, 100 the right motor standing still, 200 the right motor backwards as fast (a turn on the spot). A negative turn swaps the roles. Degrees, rotations and seconds count the faster motor.

| Function | What it does |
|---|---|
| `ev3_motor_sync_start(left, right, speed, turn)` | Drives the pair until it is stopped with `ev3_motor_sync_stop(left, right, mode)`. Do not wait for it with `ev3_motor_wait()`: it counts as busy until it is stopped. |
| `ev3_motor_sync_run_degrees(left, right, speed, turn, degrees)`<br>`ev3_motor_sync_run_rotations(...)`<br>`ev3_motor_sync_run_seconds(...)` | Drive the pair by an amount and return when it is done; the motors brake at the end. |
| `ev3_motor_sync_start_degrees(...)`<br>`ev3_motor_sync_start_seconds(...)` | Start the same and return at once; wait with `ev3_motor_wait(left \| right, timeout_ms)`. |

Only one pair runs at a time: starting a pair with other motors stops the pair that ran.

A run by degrees waits until the motor has turned that far. If no motor is plugged in, nothing turns and the run never ends; give `ev3_motor_wait()` a timeout where that matters.

## Sensors (`ev3/ev3_sensor.h`)

A reading switches the sensor to the mode it needs; the first reading in a new mode takes a moment.

| Sensor | Functions |
|---|---|
| Touch | `ev3_sensor_read_touch(port)`: pressed or not |
| Color | `ev3_sensor_read_color(port)` (`EV3_COLOR_BLACK` …)<br>`ev3_sensor_read_reflected(port)` (0 to 100)<br>`ev3_sensor_read_ambient(port)` (0 to 100)<br>`ev3_sensor_read_rgb(port)` |
| Ultrasonic | `ev3_sensor_read_distance_cm(port)`<br>`ev3_sensor_read_distance_in(port)`<br>`ev3_sensor_ultrasonic_hears_other(port)` |
| Gyro | `ev3_sensor_read_gyro_angle(port)`<br>`ev3_sensor_read_gyro_rate(port)`<br>`ev3_sensor_gyro_reset(port)` |
| Infrared | `ev3_sensor_read_ir_proximity(port)`<br>`ev3_sensor_read_ir_seek(port, channel)`<br>`ev3_sensor_read_ir_remote(port, channel)` |
| Any | `ev3_sensor_get_type(port)` says what is plugged in. |

### Reset the gyro before you work with it

Reset the gyro before the program starts working with it, with the robot standing still:

```c
ev3_sensor_gyro_reset(EV3_PORT_4);
ev3_motor_start(EV3_PORT_B, 30);
ev3_motor_start(EV3_PORT_C, -30);
while (ev3_sensor_read_gyro_angle(EV3_PORT_4) < 90) {
    ev3_sleep_ms(10);
}
ev3_motor_stop(EV3_PORT_B | EV3_PORT_C, EV3_STOP_BRAKE);
```

- **Why:** the gyro keeps counting its angle from when it was plugged in, across programs. Without a reset the angle would start wherever an earlier program left it. ev3lib does start it at 0 at the program's first reading, but a reset says clearly where 0 is.
- **When to repeat it:** before each turn that needs its own 0.
- **Direction:** clockwise is positive.

## Buttons, LEDs, sound (`ev3/ev3_ui.h`)

**Buttons**

| Function | What it does |
|---|---|
| `ev3_button_is_pressed(EV3_BUTTON_ENTER)` | Whether a button is held down. |
| `ev3_button_wait_press(buttons, timeout_ms)` | Waits until one of the buttons is pressed. |
| `ev3_button_wait_release(buttons)` | Waits until none of them is held down. |
| `ev3_button_take_back()` | After it a short press of Back no longer stops the program; holding Back always does. |

**LEDs**

| Function | What it does |
|---|---|
| `ev3_led_set(EV3_LED_GREEN_FLASH)` | Shows a pattern on the LEDs. |

**Sound**

| Function | What it does |
|---|---|
| `ev3_sound_play_tone(frequency, ms)` | Plays a tone and returns at once. |
| `ev3_sound_play_tone_volume(frequency, ms, volume)` | The same at a set volume. |
| `ev3_sound_stop()` | Stops the sound. |

## Display (`ev3/ev3_lcd.h`)

The display has 178 by 128 pixels, black on white. x runs from 0 at the left to 177, y from 0 at the top to 127.

```c
ev3_lcd_clear();
ev3_lcd_set_font(EV3_FONT_LARGE);
ev3_lcd_draw_text(0, 0, "Hello!");
ev3_lcd_set_font(EV3_FONT_NORMAL);
ev3_lcd_draw_textf(0, 30, "distance: %d cm", distance);
```

| Function | What it does |
|---|---|
| `ev3_lcd_clear()` | Makes the display white and puts the text cursor in the top left corner. |
| `ev3_lcd_set_font(font)` | `EV3_FONT_NORMAL` (8×9 pixels, the start), `EV3_FONT_SMALL` (8×8), `EV3_FONT_LARGE` (16×16) or `EV3_FONT_TINY` (5×7). |
| `ev3_lcd_draw_text(x, y, text)`<br>`ev3_lcd_draw_textf(x, y, format, ...)` | Text with its top left corner at x, y; `textf` makes it as `printf()` does. Each character's cell is drawn whole, so new text covers the old. |
| `ev3_lcd_print(format, ...)`<br>`ev3_lcd_println(format, ...)` | Text at the text cursor, like a terminal: long lines wrap, and at the bottom the display scrolls up by whole lines. `ev3_lcd_set_cursor(x, y)` moves the cursor. |
| `ev3_lcd_draw_pixel(x, y, black)` | One pixel, black (`true`) or white (`false`). |
| `ev3_lcd_draw_line(x1, y1, x2, y2)` | A black line. |
| `ev3_lcd_draw_rect(x, y, w, h)`<br>`ev3_lcd_fill_rect(...)`<br>`ev3_lcd_clear_rect(...)`<br>`ev3_lcd_invert_rect(...)` | A rectangle: the outline, filled black, filled white, or black and white swapped inside it. |
| `ev3_lcd_draw_circle(cx, cy, r)`<br>`ev3_lcd_fill_circle(cx, cy, r)` | A circle: the outline or filled black. |
| `ev3_lcd_draw_xbm(x, y, w, h, data)` | A picture from the program: XBM data, as an image editor saves it. |
| `ev3_lcd_draw_image(x, y, path)` | A picture file in the EV3 software's RGF format. |
| `ev3_lcd_set_auto_update(false)`<br>`ev3_lcd_update()` | Every drawing function shows its result at once. To show a picture drawn in many steps all at once, turn that off and call `ev3_lcd_update()` when the picture is ready. |

Anything drawn partly outside the display is cut off at its edge.

The text uses the brick's own fonts, the ones its menus use. ev3lib does not carry them: on first use it reads them from the brick's firmware. They have the ASCII characters only, so a letter with a diacritic, such as č, shows as one `?`.

While a program runs, the firmware shows a small animation. A program that draws on the display asks EVƎ C's launcher to end that animation the first time it draws, so it has the display to itself.

## Waiting and files

| Function | What it does |
|---|---|
| `ev3_sleep_ms(ms)`<br>`ev3_sleep_sec(seconds)` | Wait. |
| `ev3_return_fopen(path, mode)`<br>`ev3_return_file(path)` | Mark files that EVƎ C downloads into `returned/run-<n>/` after the run (see [cli.md](cli.md)). |
