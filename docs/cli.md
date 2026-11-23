# `ev3 c`: EVƎ C from a terminal

`ev3 c` does from a terminal, a script or an agent what EVƎ C's VS Code commands do: build a
project, upload it, run it and read its results. It goes through EVƎ Cockpit's daemon like the
extension does, so it can work on a brick while VS Code has the same brick open.

## Installing

EVƎ Cockpit's `ev3` command runs `ev3 c …` as the add-on program `ev3-c`, which it looks for on
PATH (on Windows, the `.cmd` shim npm installs). From this repository:

```
npm install
npm run build:runtime
npm run bundle:cli
npm install --global .
```

`ev3-c` then runs from npm's global folder; `ev3 c --version` checks it. EVƎ Cockpit must be
installed and have started its daemon once (opening VS Code with it does).

## Commands

```
ev3 c build [folder]                 build the project
ev3 c upload [folder]                build, then upload what changed
ev3 c run [folder]                   build, upload and run, then print the result
ev3 c stop                           stop the program running on the brick
ev3 c emergency-stop                 stop every program and motor on the brick
ev3 c runs [folder]                  list the project's runs on the brick
ev3 c runs show <n> [folder]         print run n's result
ev3 c runs delete [--keep <n>] [folder]
                                     delete the run folders, all or all but the newest n
ev3 c toolchain                      check the compiler, downloading it when needed
```

The folder holds the project's `ev3c.json`; without one, the current folder is used.

| Option | Meaning |
|---|---|
| `--brick <name\|id>` | The brick. Without it: `EV3C_BRICK`, else the brick EVƎ Cockpit's VS Code window shows, else the only connected brick. |
| `--toolchain <path>` | A compiler to use instead of the pinned one; also `EV3C_TOOLCHAIN`. |
| `--json` | Print one JSON object instead of text, errors included. |

`EV3_COCKPIT_CHANNEL` selects a development or test daemon instead of the user's own.

## Exit codes

| Code | When |
|---|---|
| 0 | Done; for `run`, the program exited with 0. |
| the program's code | `run`: the program exited with it (capped at 255). |
| 128 + n | `run`: the program was ended by signal n (a crash with signal 11 gives 139). |
| 130 | `run`: the program was stopped (Stop, Back, Ctrl+C), or the command was interrupted. |
| 1 | Something failed: the build, the cockpit, the brick, or no result was recorded. |
| 2 | The command line was wrong. |

A failure prints the cockpit's error code in brackets (`[brick_busy]`), or, when the daemon could
not start, why (`[missing_binary]`). With `--json` the object is
`{ "error": { "message", "code"?, "failure"? } }`.

## Ctrl+C during `run`

- While building or connecting, it ends the command; nothing is started.
- From the upload on, it stops the program. A stop during the upload cancels the start; a stop
  while the program runs ends it, and its result is printed as usual.
- A second Ctrl+C leaves without waiting for the result; `ev3 c runs` lists the run later.

## Where things are kept

The downloaded compiler is cached per user: `%LOCALAPPDATA%\ev3-c\toolchain` on Windows,
`~/Library/Caches/ev3-c/toolchain` on macOS, `$XDG_CACHE_HOME/ev3-c/toolchain` (or
`~/.cache/ev3-c/toolchain`) on Linux. It is separate from the one the VS Code extension keeps.
