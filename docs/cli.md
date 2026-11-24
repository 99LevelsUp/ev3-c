# `ev3 c`: EVƎ C from a terminal

`ev3 c` does from a terminal, a script or an agent what EVƎ C's VS Code commands do: build a
project, upload it, run it and read its results. It goes through EVƎ Cockpit's daemon like the
extension does, so it can work on a brick while VS Code has the same brick open.

## Installing

EVƎ Cockpit's `ev3` command runs `ev3 c …` as the add-on program `ev3-c`, which it looks for on
PATH. `ev3-c` is a Rust program in `backend/`. Until it ships in packages (#13 U7), build it from
this repository with Rust (the toolchain `rust-toolchain.toml` names) and the system's `curl`,
which downloads the compiler:

```
npm run build:runtime                 # the brick runtime: ev3lib and the supervisor
cd backend
cargo build --release
```

Then put `backend/target/release` on PATH. `ev3-c` finds ev3lib's headers and the runtime in the
repository above it. `ev3 c --version` checks it. EVƎ Cockpit must be installed and have started
its daemon once; opening VS Code with it installed does that.

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
ev3 c get <brick path> [local path]  download a file from the brick (default: its name, here)
ev3 c toolchain                      check the compiler, downloading it when needed
ev3 c watch                          follow the program state of the brick until Ctrl+C
```

`watch` tells the brick's program state each time it changes, for a status bar: which brick,
whether a program runs, or why no brick can be followed now. It never connects or holds a
brick. It only follows the one the brick choice names while that brick is online, and it
chooses again whenever the bricks, EVƎ Cockpit's windows or the daemon change. It prints
text, or events with `--events`, but no single `--json` object.

The folder holds the project's `ev3c.json`; without one, the current folder is used.

## Files a program returns

A program marks files for return with ev3lib's `ev3_return_fopen()` or `ev3_return_file()`.
`ev3 c run` and `ev3 c runs show` download them into the project's `returned/run-<n>/` folder: a
file inside the brick's project folder at its path there, any other below `brick/` at its whole
brick path. With `--json`, `result.returned` lists the brick paths and `returnedFiles` says where
each one went, or why it could not be returned.

| Option | Meaning |
|---|---|
| `--brick <name\|id>` | The brick. Without it: `EV3C_BRICK`, else the brick EVƎ Cockpit's VS Code window shows, else the only connected brick. |
| `--toolchain <path>` | A compiler to use instead of the pinned one; also `EV3C_TOOLCHAIN`. |
| `--json` | Print one JSON object instead of text, errors included. |
| `--events` | Print one JSON event per line as the command proceeds (see below); wins over `--json`. |

`EV3_COCKPIT_CHANNEL` selects a development or test daemon instead of the user's own.
`EV3C_CLIENT_NAME` names the connection to the cockpit instead of
`ev3 c <command> (terminal)`. Other clients read that name in `brick_busy`; EVƎ C's VS Code
extension uses it to connect as `EVƎ C (VS Code)`. `EV3C_TOOLCHAIN_SETTING` names where
`EV3C_TOOLCHAIN` came from, so that a wrong path points the user to it; the extension sets it to
`ev3-c.toolchain.path`.

## Exit codes

| Code | When |
|---|---|
| 0 | Done; for `run`, the program exited with 0. |
| the program's code | `run`: the program exited with it (capped at 255). |
| 70 | `run`: also when ev3lib stopped the program because a plain call failed. The result then says so, and `result.libraryError` holds ev3lib's line, `file:line: function: what went wrong`. |
| 128 + n | `run`: the program was ended by signal n (a crash with signal 11 gives 139). |
| 130 | `run`: the program was stopped (Stop, Back, Ctrl+C), or the command was interrupted. |
| 1 | Something failed: the build, the cockpit, the brick, or no result was recorded. |
| 2 | The command line was wrong. |

A failure prints the cockpit's error code in brackets (`[brick_busy]`), or, when the daemon could
not start, why (`[missing_binary]`). With `--json` the object is
`{ "error": { "message", "code"?, "failure"? } }`.

## The program's output during `run`

`run` shows the program's stdout and stderr while the program runs: its stdout on standard
output and its stderr on standard error, as they come, with the program's own line ends. The
result that follows does not repeat them; it only says when an output was cut at the size limit
or could not be read. EVƎ C follows the run's `out` and `err` files on the brick through EVƎ
Cockpit, which allows two follows per brick for every client together. When none is free, a line
says so, and the output comes when the program ends.

With `--json`, `run` follows nothing and prints the output in its one object, so it leaves the
brick's follows to others. With `--events`, the output comes as `output` events (below), and the
`result` event still carries all of it.

## Ctrl+C during `run`

- While building or connecting, it ends the command; nothing is started.
- From the upload on, it stops the program. A stop during the upload cancels the start; a stop
  while the program runs ends it, and its result is printed as usual.
- A second Ctrl+C leaves without waiting for the result; `ev3 c runs` lists the run later.

In any other command, Ctrl+C ends the command with code 130.

## Events (`--events`)

With `--events`, a program that starts `ev3 c` follows the command as it proceeds. EVƎ C's VS
Code extension works this way: it runs `ev3-c` once per command, and runs `ev3-c watch` for the
status bar. Standard output carries one JSON object per line
and nothing else; each has `event` first. The progress events come as they happen, and each
command ends with exactly one terminal event, the last line, which carries `exitCode`.

| Event | When | Other fields |
|---|---|---|
| `building` | The build starts (`build`, `upload`, `run`). | `folder`: the project folder |
| `diagnostics` | The compiler said something, warnings or errors. | `text` |
| `built` | The build succeeded. | `project`, `artifacts` (as in `build --json`) |
| `connected` | The brick is held through EVƎ Cockpit. | `brickId` |
| `started` | `run`: the program was uploaded and started. | `start`: `outcome`, `runSeq`, `uploaded`, `skipped`; `launchId`, `startedAt` |
| `output` | `run`: a piece of the program's output, while it runs and as it ends. | `stream`: `out` or `err`; `text`: whole UTF-8 characters, with the program's own line ends |
| `outputAtEnd` | `run`: the output cannot be followed while the program runs; it comes in `output` events as the run ends. | `reason` |
| `stopping` | `run`: an interrupt asked the program to stop. | |
| `program` | `watch`: the program state of the followed brick. | `brickId`, `running`, `runSeq` |
| `noBrick` | `watch`: no brick can be followed now. | `error`: `message`, `code`? |
| `notice` | EVƎ Cockpit's client says something, such as replacing an older daemon. | `text` |

A run's start time is known only to the computer that started it. `startedAt` in `runs` and
`runs show` comes from `EV3C_LAUNCHES`: a caller that keeps the `launchId` and `startedAt` of
the `started` events hands them back as a JSON object of launch ID to milliseconds since 1970,
for example `{"k3x9…": 1795000000123}`.

The terminal event is the object `--json` would print, with `event` and `exitCode` added:

| Event | When |
|---|---|
| `result` | `run` or `runs show` has the run's result (`result`, `start`, `returnedFiles`). |
| `noResult` | `run`: no result was recorded in time (`noResult`). |
| `done` | Any other command finished (or `runs show` found no record: `result` is `null`). |
| `error` | The command failed (`error`: `message`, `code`?, `failure`?); `exitCode` 2 for a wrong command line, whose usage text still goes to standard error. |
| `interrupted` | An interrupt ended the command (`interrupted: true`). |

For example, `ev3 c run --events`:

```
{"event":"building","folder":"/home/me/robot"}
{"event":"built","project":"robot","artifacts":[…]}
{"event":"connected","brickId":"usb-0016530aaaaa"}
{"event":"started","start":{"outcome":"started","runSeq":7,"uploaded":[…],"skipped":[…]}}
{"event":"output","stream":"out","text":"distance: 42 cm\n"}
{"event":"result","exitCode":0,"project":"robot","brickId":"usb-0016530aaaaa","start":{…},"result":{…}}
```

Standard input stands for Ctrl+C, which a program that starts `ev3 c` cannot send on every
system: each line `interrupt` acts as one Ctrl+C, and so does the end of standard input, so a
caller that goes away stops the run it started. Keep standard input an open pipe for as long
as the command should go on: started with standard input closed or empty, the command is
interrupted at once. Other lines are ignored. Standard error stays quiet apart from a usage
text. `--help` and `--version` print text as always.

## Where things are kept

The downloaded compiler is cached per user: `%LOCALAPPDATA%\ev3-c\toolchain` on Windows,
`~/Library/Caches/ev3-c/toolchain` on macOS, `$XDG_CACHE_HOME/ev3-c/toolchain` (or
`~/.cache/ev3-c/toolchain`) on Linux. The VS Code extension runs `ev3-c` too, so both use the
same cache.

`ev3-c` finds EVƎ C's own files, ev3lib's headers (`ev3lib/include`) and the brick
runtime (`out/runtime`), in the nearest folder above the program that holds both;
`EV3C_PACKAGE_ROOT` names that folder instead.
