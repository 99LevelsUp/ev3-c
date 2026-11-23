# EVƎ C

Write, build and run C programs on LEGO MINDSTORMS EV3 bricks that keep their **original LEGO
firmware**: no ev3dev, no reflashing. EVƎ C compiles your program into a static ARM binary,
puts it on the brick with a small launcher that shows up in the brick's own file menu, runs it,
and brings back what it printed, how it ended, and any files it wrote.

EVƎ C builds on [EVƎ Cockpit](https://github.com/99LevelsUp/ev3-cockpit), which connects to the
bricks over USB, Bluetooth or Wi-Fi. EVƎ C itself never talks to a brick directly.

> **Status:** in development, not released yet. Hardware testing is under way.

## What you get

- **A VS Code extension** with the commands *EV3 C: Build, Upload, Run, Stop, Emergency Stop,
  Show Runs, Delete Runs, Download File from Brick* and *Check Toolchain*, and a status bar item
  that shows whether a program runs.
- **`ev3 c`**, the same from a terminal or a script, as a subcommand of EVƎ Cockpit's `ev3`
  command ([docs/cli.md](docs/cli.md)).
- **An MCP server**, `ev3 c mcp`, so an AI agent can build and run projects next to EVƎ Cockpit's
  own MCP tools ([docs/mcp.md](docs/mcp.md)).
- **ev3lib**, a small C library for the brick: motors (power, regulated speed, turns by degrees
  or time, rotation readings), buttons, LEDs, tones, and files returned to your computer
  (`ev3lib/include/ev3.h`).
- **The compiler is downloaded for you:** a pinned [Zig](https://ziglang.org/) builds static
  ARMv5TE binaries, with nothing to install.

## A project

A project is a folder with `ev3c.json` and your C sources:

```json
{ "name": "hello" }
```

```c
#include <stdio.h>
#include "ev3.h"

int main(void)
{
    printf("Hello from the EV3!\n");
    ev3_motor_set_power(EV3_PORT_A, 50);   /* motor on port A at half power */
    ev3_core_sleep_ms(1000);
    ev3_motor_stop(EV3_PORT_A, 1);
    return 0;
}
```

**EV3 C: Run** (or `ev3 c run` in the folder) builds the project, uploads what changed and starts
it. When it ends, you see its output, how it ended (exit code, crash signal, or stopped), and
the files it returned.

## How a program runs

On the brick, a launcher in the brick's user slot starts a small supervisor, which runs your
program in its own process group and keeps each run's output and result in a run folder.

- **Stop, Back or Emergency Stop:** the supervisor ends the program and stops the motors, also
  when the program crashes.
- **A previous run:** its results stay on the brick, so you can read them later, also for a run
  started from the brick's menu.

## Requirements

- An EV3 brick with the stock LEGO firmware.
- EVƎ Cockpit, which provides the connection to the bricks.
- VS Code 1.95 or later for the extension; Node.js 20 or later for `ev3 c` from source.

## Building from source

```
npm install
npm run build:runtime   # ev3lib and the supervisor for the brick
npm run compile
npm run test:ci         # compile, lint, unit tests
npm run test:native     # C tests, on Linux or macOS
```

The backend is moving to Rust (`backend/`, a Cargo workspace); see
[issue #13](https://github.com/99LevelsUp/ev3-c/issues/13).
