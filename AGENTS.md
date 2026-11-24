# Repository Guidelines

## Project Structure & Module Organization
`design.md` is the current source of truth for architecture, toolchain choices, and planned testing. It is kept locally and is not committed (see `.gitignore`). Treat it as authoritative when adding code or documentation. Runtime assets live under [`assets/images/`](assets/images) and [`assets/sounds/`](assets/sounds), grouped by category such as `assets/images/Firmware`, `assets/images/Eyes`, `assets/sounds/System`, and `assets/sounds/Communication`. Working notes belong in `.work/ideas/` (local only, not committed); keep contributor-facing material in the repository root and `docs/`.

Repository layout:

- `backend/` — the Rust Cargo workspace:
  - `crates/core` holds the core: projects, the toolchain, the launcher and brick operations behind the `BrickPort` trait.
  - `crates/ev3-c` is the `ev3-c` program. It provides the CLI and the MCP server, and is the link to EVƎ Cockpit's daemon through the cockpit's `ev3-client` crate.
  - `crates/xtask` holds the repository's build tasks (the brick runtime and the native C tests).
- `src/` — the VS Code extension (TypeScript), with `package.json` in the repository root. It is UI only. Every command runs `ev3-c` with `--events` (`src/backend/ev3c.ts`), and the status bar follows `ev3-c watch`.
- The repository's languages are Rust (backend and tooling), TypeScript (the extension) and C (code that runs on the brick). There is no Python and there are no new JavaScript scripts.
- `supervisor/` — the native supervisor that runs a student program on the brick (C, built for the brick and for host tests).
- `ev3lib/include/ev3.h` + `ev3lib/include/ev3/` — public headers of Layer 1 (`ev3_core`); `ev3/toolkit/` is Layer 2, `ev3/robotics/` is Layer 3.
- `ev3lib/src/{core,toolkit,robotics}/` — implementation of the three layers.
- `ev3lib/tests/` — PC-hosted unit tests.
- `templates/` — starter projects created by the extension.
- `assets/` — LCD images and sounds shipped with the library.
- `tests/hardware/` — programs and procedures that verify behavior on a real brick.
- `docs/` — contributor and user documentation; `docs/plans/` holds implementation plans and `docs/spikes/` hardware spike findings.

Follow that structure when scaffolding new code.

## Build, Test, and Development Commands
- `npm install` once, then `npm run test:ci` runs compile, lint, and the extension's unit tests (`npm run compile`, `npm run lint`, `npm run test:unit` individually).
- In `backend/`, run `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test`. `cargo build` builds the `ev3-c` program, which the extension finds in a working copy.
- `npm run build:runtime` builds the brick runtime into `out/runtime/`, and `npm run test:native` runs the C tests on Linux or macOS. Both run the Rust xtask.
- `npm run bundle` produces `out/extension.js` for packaging.
- Brick binaries are static ARMv5TE soft-float executables built with Zig and musl, not with `arm-linux-gnueabi-gcc`: the stock firmware ships glibc 2.8 on kernel 2.6.33. The current plan in `docs/plans/` overrides the toolchain section of `design.md`.

Prefer `rg --files` and `rg "<pattern>"` for repository search.

## Coding Style & Naming Conventions
Use 4-space indentation for C, TypeScript, JSON, and Markdown. Keep public C APIs in the `ev3_{module}_{action}` form described in `design.md`, for example `ev3_motor_start` or `ev3_sensor_read_touch`. Name headers and C sources consistently (`ev3_motor.h`, `ev3_motor.c`). Preserve existing asset naming when adding media; category folders use Title Case and filenames may use spaces or underscores to match LEGO naming.

## Testing Guidelines
Add unit tests beside the future `ev3lib/tests` tree and keep them runnable on a PC first. Favor small native tests for math, PID, filtering, state, and validation code, then add hardware integration coverage separately for EV3 uploads. New tests should describe the feature or module under test, for example `test_pid.c` or `motor_validation.test.ts`.

## Commit & Pull Request Guidelines
Local git history is not available in this workspace export, so no repository-specific commit convention can be inferred. Use short imperative subjects with a clear scope, such as `docs: clarify toolchain options` or `ev3lib: add motor port validation`. Pull requests should summarize the change, reference the relevant section of `design.md`, list validation performed, and include screenshots only for UI or asset changes.
