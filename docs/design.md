# EVƎ C design

This page records why EVƎ C is shaped the way it is. For how to use it, see the
[README](../README.md), [cli.md](cli.md), [mcp.md](mcp.md) and [ev3lib.md](ev3lib.md).
Open work lives in the GitHub issues, not here.

## 1. Vision

EVƎ C aims to be the most approachable and most complete way to program a LEGO MINDSTORMS EV3
in C. It is made of:

- **`ev3 c`**, a command-line tool that builds, uploads and runs a project and brings back its
  output, its result and the files it wrote. It is a subcommand of EVƎ Cockpit's `ev3` command.
- **An MCP server**, `ev3 c mcp`, so an AI agent can do the same next to EVƎ Cockpit's own tools.
- **A VS Code extension**, a thin UI that runs `ev3-c` for every command.
- **ev3lib**, the C library every program links.
- **A runtime on the brick**: a small launcher in the brick's user slot and a supervisor that
  runs the program, stops the motors when it ends, and keeps each run's output and result.

EVƎ C never talks to a brick itself. EVƎ Cockpit connects to the bricks over USB, Bluetooth or
Wi-Fi, and `ev3-c` reaches the cockpit's daemon through the Rust crate `ev3-client`.

The compiler is a pinned Zig release that `ev3-c` downloads and checks by SHA-256. It builds
static ARMv5TE binaries with musl, so a program needs nothing on the brick but itself.

### Principles

| Principle | Meaning |
|---|---|
| Consistency | Every function follows one naming pattern, `ev3_<module>_<action>`, and one error rule. |
| Completeness | What the original firmware can do, ev3lib should expose. This is a goal; today's coverage is in ev3lib.md. |
| Approachability | The level of the EV3 Classroom and Lab blocks, with the full power of C. |
| One platform | The original LEGO firmware only (section 2). |
| Full Linux | A program can use any C, POSIX or Linux function the brick offers. |

### The error rule

ev3lib is for children and junior programmers, and C has no exceptions. So every function that
can fail comes twice:

- the **plain** variant returns its value, or nothing. On an error it prints `file:line`, the
  function and the reason, and ends the program with exit code 70; the motors stop.
- the **try** variant (`ev3_motor_try_start`) returns an `Ev3Result`, and hands a reading back
  through a pointer.

A function that cannot fail exists once.

### What EVƎ C is not

- Not visual programming.
- Not a bytecode compiler. Programs are native ARM code. EVƎ C does generate one small `.rbf`
  launcher per project, so the program shows up in the brick's own file menu and can be started
  from there.
- Not a runtime for several EV3 operating systems.

## 2. The original firmware only

ev3lib is written for the original LEGO firmware and its device files: `/dev/lms_pwm`,
`/dev/lms_motor`, `/dev/lms_analog`, `/dev/lms_uart`, `/dev/lms_iic`, `/dev/lms_ui`,
`/dev/lms_sound` and the display. It does not work on ev3dev or any other system for the EV3,
and it will not.

### Why

The strength of the project is a full and exact mapping of what the original firmware can do,
without a lowest common denominator across systems. That lets us:

- expose firmware-specific features without conditional support;
- keep the API free of "not supported on this platform" branches;
- keep the implementation, the tests and the documentation simple.

It also means a student keeps the brick as it came: no reflashing, no SD card image, and the
brick's menus, sounds and other programs keep working.

### What follows from it

- There is one target runtime. Start-up only checks for the original firmware's interfaces.
- Modules may rely on how the LEGO firmware behaves. For example, ev3lib draws text in the
  brick's own fonts, which it reads from the firmware at run time instead of carrying a copy.
- There is no second backend and no platform detection at start-up.
- On stock firmware, the virtual machine only runs `.rbf` bytecode. EVƎ C's launcher is such a
  file: it starts the supervisor, which runs the native program in its own process group.

## 3. c4ev3, and how ev3lib answers it

c4ev3 (EV3-API) is the best-known C library for the original firmware. It works, but several
authors added to it over the years without a common design, and it shows.

### What we found

**Return types.** Within one module, functions return `void`, `bool`, `int`, `int8_t` or `char`
with no clear rule. `OnFwd(OUT_A)` returns `void`, so a program cannot tell whether the motor
started; `OutputStop()` returns `bool`; `ReadSensor()` returns an `int` where `-1` or `INT_MIN`
means an error. Generic error handling is impossible.

**Naming.** Three conventions side by side:

```c
OnFwd(OUT_A);                                       /* NXC legacy */
OutputStepPowerEx(OUT_A, 50, 0, 360, 0, true, 0);   /* CamelCase with an Ex suffix */
SensorsInit(); LcdInit(); SoundInit();              /* per-module init functions */
```

**Sensors.** All four ports are configured at once, and only once
(`SetAllSensorMode(TOUCH_PRESS, US_DIST_CM, COL_COLOR, NO_SEN)`); changing a port later is not
possible. `SetSensorMode()` is deprecated and does nothing. `ReadSensor()` returns a bare `int`
whose meaning depends on the mode: a colour comes back as `5`, and the program has to know
that 5 is red.

**Missing firmware features.** The firmware offers battery voltage, current and temperature,
motor current and stall detection, sensor auto-detection, raw RGB from the colour sensor, gyro
reset, daisy chains, Bluetooth, mailboxes, file operations and system information. c4ev3 exposes
none of these, or only as constants.

**Processes and threads.** c4ev3 was not designed for either:

- A destructor (`FreeEV3()`) stops all outputs and unmaps the sensors at process exit. A child
  created with `fork()` that exits normally runs it too, and stops every motor of the parent.
- The sensor memory is mapped shared; the child's exit unmaps it and closes its descriptors.
- There is not a single mutex or atomic operation; writes to the PWM device are unguarded.

### ev3lib's answer

| c4ev3 | ev3lib today |
|---|---|
| Mixed return types | Plain variant: a reading returns its value, a setting returns nothing, an error stops the program with `file:line`. Try variant: always an `Ev3Result`. |
| Three naming conventions | One: `ev3_<module>_<action>`, and `ev3_<module>_try_<action>`. |
| One init function per module | None. ev3lib starts before `main()` and stops the motors at every end of the program. |
| Sensors configured once, all at once | No configuration. A reading switches the sensor to the mode it needs; `ev3_sensor_get_type()` says what is plugged in. |
| Bare `int` readings | Typed readings: `ev3_sensor_read_color()` returns `Ev3Color`, `ev3_sensor_read_rgb()` returns `Ev3ColorRgb`. |
| No error context | `ev3_error_message()` says what went wrong; `ev3_error_string()` names the code. |
| `*Ex` functions with many positional flags | Named functions with readable parameters, e.g. `ev3_motor_run_degrees_ramp()`. |
| Deprecated aliases | None: one function, one name. |
| No gyro reset, no raw RGB | `ev3_sensor_gyro_reset()`, `ev3_sensor_read_rgb()`. |

Not answered yet:

- **Battery, motor current and stall detection, files, Bluetooth, mailboxes, daisy chains,
  system information.** They are planned; see the open ev3lib issues.
- **Thread safety.** ev3lib does not lock its device access yet. Until it does, drive the
  hardware from one thread.
- **`fork()`.** ev3lib registers its motor stop with `atexit()`. A child created with `fork()`
  inherits it, so a child that ends with `exit()` or returns from `main()` stops all motors. End
  a child with `_exit()`, or use threads.

Coming from c4ev3 is mostly a matter of names: `OnFwd` becomes `ev3_motor_start`, `Wait`
becomes `ev3_sleep_ms`, `ButtonIsDown(BTNEXIT)` becomes `ev3_button_is_pressed(EV3_BUTTON_BACK)`,
and `InitEV3()`, `FreeEV3()` and `SetAllSensorMode()` simply go away. EVƎ C uses no code from
c4ev3.

## 4. Compared with Pybricks

Pybricks is the most advanced alternative environment for the EV3, in MicroPython. It runs on
its own system image on an SD card (EV3 MicroPython is built on ev3dev), so it is outside what EVƎ C targets, but its API is a good reference.

### Concepts we take from Pybricks

These are planned for ev3lib, in C form:

| Area | Pybricks | ev3lib plan |
|---|---|---|
| Drive base | `DriveBase(left, right, wheel_diameter, axle_track)` | a drive-base module configured with the same four values |
| Driving | `straight(mm)`, `turn(deg)`, `arc(radius, angle)`, `drive(speed, turn_rate)` | the same four movements |
| Gyro help | `use_gyro(True)` | the drive base corrects with a gyro |
| Odometry | `distance()`, `angle()` | distance and angle of the drive base |
| Motor target | `run_target()`, `run_until_stalled()`, `track_target()` | run to a position, run until stalled, track a target |
| Motor tuning | `control.pid()`, `control.limits()`, `load()`, gears | PID, speed and acceleration limits, load estimate, gear ratio |

What ev3lib has today is closer to the EV3 blocks: regulated speed and power, runs by degrees,
rotations or seconds, ramps, and two motors driven together by the firmware
(`ev3_motor_sync_*`), which keeps a robot going straight.

### What the original firmware allows beyond Pybricks

Because ev3lib talks to the original firmware directly, it can reach things Pybricks does not
offer. Some exist now, the rest are planned:

| Area | Status |
|---|---|
| The brick's own fonts, pictures in the EV3 software's RGF format | now |
| Files returned from the brick to the computer after a run | now |
| Pictures and sounds of the EV3 software | partly (RGF picture files now, sound files planned) |
| Battery and power information, firmware and hardware versions | planned |
| Daisy chains of up to four bricks, mailboxes between bricks | planned |
| Bluetooth, Wi-Fi and USB management, file I/O on the brick | planned |
| Firmware dialogs on the display, graphs, a stack of saved screens | planned |
| NXT and HiTechnic sensors, sensor names, units and raw data | planned |
| Long presses and press counting on the buttons, LED patterns | partly (LED patterns now) |
| Auto-shutdown control | planned |
| Plain Linux underneath: syscalls, raw I/O, pthreads | now |

### Speech

Pybricks can speak at run time. ev3lib does not plan text-to-speech on a 300 MHz ARM9. The idea
is to synthesise speech on the computer with a good engine in the student's language, convert it
to the brick's sound format, upload it with the program, and play it as a sound file. It is not
built yet.

### Who each one is for

| | ev3lib | Pybricks |
|---|---|---|
| Language | C | MicroPython |
| System | the original LEGO firmware | its own image on an SD card |
| Hardware access | direct, through the firmware's device files | through the MicroPython runtime |
| User | from a child moving on from blocks to an advanced C programmer | Python beginners and up |
| Extending | anything Linux on the brick offers | what the Pybricks API offers |
