# Repository Guidelines

## Project Structure & Module Organization
`design.md` is the current source of truth for architecture, toolchain choices, and planned testing. It is kept locally and is not committed (see `.gitignore`). Treat it as authoritative when adding code or documentation. Runtime assets live under [`assets/images/`](assets/images) and [`assets/sounds/`](assets/sounds), grouped by category such as `assets/images/Firmware`, `assets/images/Eyes`, `assets/sounds/System`, and `assets/sounds/Communication`. Working notes belong in `.work/ideas/` (local only, not committed); keep contributor-facing material in the repository root and `docs/`.

Repository layout:

- `src/` — VS Code extension (TypeScript), `package.json` in the repository root.
- `ev3lib/include/ev3.h` + `ev3lib/include/ev3/` — public headers of Layer 1 (`ev3_core`); `ev3/toolkit/` is Layer 2, `ev3/robotics/` is Layer 3.
- `ev3lib/src/{core,toolkit,robotics}/` — implementation of the three layers.
- `ev3lib/tests/` — PC-hosted unit tests.
- `templates/` — starter projects created by the extension.
- `assets/` — LCD images and sounds shipped with the library.
- `docs/` — contributor and user documentation.

Follow that structure when scaffolding new code.

## Build, Test, and Development Commands
This snapshot does not yet include runnable build scripts. Before adding one-off tooling, align it with the commands already proposed in `design.md`:

- `arm-linux-gnueabi-gcc program.c -lev3 -lm -lpthread` builds EV3-targeted binaries.
- `cd ev3lib/tests && cmake .. && make && ctest` is the planned native test loop for PC-hosted `ev3lib` tests.
- `gcc -o test_pid test_pid.c ../src/toolkit/ev3_pid.c -lm` is the lightweight single-test pattern for pure C modules.

Prefer `rg --files` and `rg "<pattern>"` for repository search.

## Coding Style & Naming Conventions
Use 4-space indentation for C, TypeScript, JSON, and Markdown. Keep public C APIs in the `ev3_{module}_{action}` form described in `design.md`, for example `ev3_motor_start` or `ev3_sensor_read_touch`. Name headers and C sources consistently (`ev3_motor.h`, `ev3_motor.c`). Preserve existing asset naming when adding media; category folders use Title Case and filenames may use spaces or underscores to match LEGO naming.

## Testing Guidelines
Add unit tests beside the future `ev3lib/tests` tree and keep them runnable on a PC first. Favor small native tests for math, PID, filtering, state, and validation code, then add hardware integration coverage separately for EV3 uploads. New tests should describe the feature or module under test, for example `test_pid.c` or `motor_validation.test.ts`.

## Commit & Pull Request Guidelines
Local git history is not available in this workspace export, so no repository-specific commit convention can be inferred. Use short imperative subjects with a clear scope, such as `docs: clarify toolchain options` or `ev3lib: add motor port validation`. Pull requests should summarize the change, reference the relevant section of `design.md`, list validation performed, and include screenshots only for UI or asset changes.
