# Brick spike: run-contract assumptions on a real EV3

Date: 18.11.2026. Brick: EV3 with firmware V1.10E, connected over USB to EVƎ Cockpit 0.2.394 in full access mode. Probes: `tests/hardware/spike/` (static Zig 0.14.1 + musl binaries, a probe launcher assembled with LEGO's `lmsasm`, and a VS Code extension that drives them through the cockpit API).

## Brick facts

| Question | Finding |
|---|---|
| Kernel | `Linux 2.6.33-rc4 ... gcc 4.3.3 (Sourcery G++ Lite 2009q1-203)`, `armv5tejl` |
| C library | glibc 2.8 (`/lib/libc-2.8.so`, `/lib/ld-2.8.so`) |
| Static Zig + musl binary | Runs; prints, reads `uname`, and its exit code (3) reaches the shell |
| Uploaded file permissions | Files written through cockpit's `writeFile` arrive as `-rwxrwxrwx`; no `chmod` is needed |
| Project folder | `/home/root/lms2012/prjs` is a symlink to `/mnt/ramdisk/prjs`, a 6 MiB tmpfs with about 4.6 MiB free on this brick |
| `/tmp` area | `/var/volatile`, a 2 MiB tmpfs |
| Clock | `Thu Jan 1 00:03:28 UTC 1970` with 249 s uptime: the clock starts at the epoch on boot, so there is no battery-backed clock to rely on |

The project folder lives in RAM. What survives a power-off without a clean shutdown was not tested.

## Launcher and supervisor behavior

| Question | Finding |
|---|---|
| VM responsiveness while the launcher polls every 100 ms | Slot state plus one small file read took 7–20 ms per second, for 10 s |
| Slot token held by the VM | The VM held `<project>/.tok` (opened by the launcher before `opSYSTEM`) at supervisor start-up in every run |
| Slot end detection | The supervisor saw the VM drop the token's inode within one 100 ms poll of cockpit's `stopProgram` |
| Process tree of a background `opSYSTEM` child | Reparented to init (`ppid=1`) but still in the VM's process group and session (`pgid=sid=1481`); `setsid()` succeeds and gives it its own group |
| Inherited descriptors | The child inherits every VM descriptor: the motor, sensor, UI, USB and Bluetooth devices, `/dev/fb0`, internal pipes, and the slot token |
| Motor stop on Stop | The motor stopped and stayed stopped after `stopProgram` |
| Back takeover | With `.bak` present the launcher blocked Back: a short press left the program running, a long press stopped it and the motor |
| Supervisor killed with SIGKILL | The launcher saw `/proc/<pid>` disappear and ended the slot within 1.5 s; the orphaned motor loop then restarted the motor, as the plan's residual risk expects |
| Program ending on its own | Removing `.run` ended the slot; cockpit reported not running |
| Short dot-names with absolute paths | `.tok`, `.run` and `.bak` built from `FILENAME GET_FOLDERNAME` worked for create, remove, and exist |

## Not as expected

- **Screen:** while the slot ran, the brick screen was blank, and the probe's `write()` calls to `/dev/fb0` did not appear. The firmware does not draw over a running program, but a native program cannot draw by writing to the framebuffer device either. Drawing on the LCD needs its own investigation; v1 sends program output to the run folder, so the run contract does not depend on it.
- **Toolchain:** Zig 0.15 and later (checked with 0.17.1) cannot link anything that uses `malloc` for ARMv5, because their libc allocator needs atomic helpers the CPU lacks. Zig 0.14.1, which keeps musl's own `malloc`, works.

## Consequences for the plan

- The Zig version is pinned to 0.14.1.
- The launcher does not change file permissions.
- Retention defaults are sized for several projects sharing the 4.6 MiB of free RAM.
- Everything else in the launcher, supervisor, and lock design holds as planned.
