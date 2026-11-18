# Hardware acceptance: program run contract

This procedure proves the run contract on a real brick through EVƎ C's own commands, in EVƎ Cockpit's **safe** access mode. Record each result in the table at the end.

## Setup

1. One EV3 brick with the stock firmware, connected over USB and set as the active brick in EVƎ Cockpit. No cockpit tests may run on it at the same time.
2. A motor on port **A**.
3. `"ev3-cockpit.accessMode": "safe"` in the user settings (the default).
4. In this repository: `npm install`, `npm run build:runtime`, `npm run bundle`.
5. Start VS Code with the extension from source: `code --extensionDevelopmentPath=<repo> <repo>/tests/hardware/programs/acceptance.code-workspace`. EVƎ Cockpit must be installed.
6. Each command asks which project to use when the workspace holds several; pick the one the step names.

## Steps

| # | Project | Do | Expect |
|---|---|---|---|
| 1 | `hello` | **EV3 C: Check Toolchain**, then **EV3 C: Run** | The output channel shows the compiler path; the motor turns for 1 s; the result shows exit 0, "program end", the two printed lines, and a PC start time. |
| 2 | `ae-crash` | **EV3 C: Run** | The motor starts and stops when the program crashes after 2 s; the status bar returns to idle; the result shows signal 11 and the line printed before the crash. |
| 3 | `ae-loop` | **EV3 C: Run**, wait 3 s, **EV3 C: Stop** | The motor stops and stays stopped; the result says the slot ended; no shell or full mode was needed. |
| 4 | `ae-loop` | **EV3 C: Run**, then **EV3 C: Emergency Stop** | As step 3. |
| 5 | `ae-loop` | **EV3 C: Run**, then press **Back** briefly on the brick | The program stops as in step 3 (it did not take Back over). |
| 6 | `ae-back` | **EV3 C: Run**, press Back briefly twice, then hold Back for 2 s | Each short press toggles the motor and the program keeps running; holding Back stops the program and the motor; the result lists the "Back pressed" lines. |
| 7 | `ae-print` | **EV3 C: Run**, wait 3 s, **EV3 C: Stop** | The captured output is about 8 KiB and the result says stdout was truncated. |
| 8 | `ae-quick` | **EV3 C: Run** six times | Each result shows exit 7. **EV3 C: Show Runs** lists exactly five runs: the oldest was removed before the sixth appeared. |
| 9 | `ae-loop` | **EV3 C: Run**; while it runs, **EV3 C: Delete Runs** → all | Every run folder except the running one is deleted. Then **Stop**. |
| 10 | `ae-quick` | Disconnect the USB cable, start `ae-quick` from the brick's file menu, reconnect, then **EV3 C: Show Runs** | The brick-started run appears with its run number and result and without a PC start time. |
| 11 | `ae-loop` | **EV3 C: Run**, then immediately **EV3 C: Run** again | The second run starts only after the first supervisor finished, and its motor runs. Then **Stop**. |
| 12 | `ae-loop` | **EV3 C: Run**, then **EV3 C: Upload** while it runs | Upload is refused with a message asking to stop the program first. Then **Stop**. |
| 13 | `ae-loop` | **EV3 C: Run**; unplug and replug USB while it runs; then **Stop**, then **Run** `ae-quick` | The status bar follows the state again after reconnecting, and the next run gets its result. |

## Results

Brick firmware: ____  EVƎ Cockpit version: ____  EVƎ C commit: ____  Date: ____

| # | Pass / fail | Notes |
|---|---|---|
| 1 | | |
| 2 | | |
| 3 | | |
| 4 | | |
| 5 | | |
| 6 | | |
| 7 | | |
| 8 | | |
| 9 | | |
| 10 | | |
| 11 | | |
| 12 | | |
| 13 | | |
