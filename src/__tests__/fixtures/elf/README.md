# ELF check fixtures

Real executables built with Zig 0.14.1 (`zig cc`) from a one-line program, `m.c`:

```c
int main(void) { return 0; }
```

Common flags, written `FLAGS` below: `-static -Os -ffunction-sections -fdata-sections -Wl,--gc-sections -s`.

| File | Command | What the check must say |
|---|---|---|
| `static-armv5te.elf` | `zig cc -target arm-linux-musleabi -mcpu=arm926ej_s FLAGS -o static-armv5te.elf m.c` | accepted |
| `armv7.elf` | `zig cc -target arm-linux-musleabi -mcpu=cortex_a8-fpregs FLAGS -o armv7.elf m.c` | ARMv7, soft-float, static: rejected for architecture only |
| `hard-float.elf` | `zig cc -target arm-linux-musleabihf -mcpu=arm926ej_s+vfp2 FLAGS -o hard-float.elf m.c` | ARMv5TE with VFP arguments and instructions: rejected as hard-float only |
| `x86-64.elf` | `zig cc -target x86_64-linux-musl FLAGS -o x86-64.elf m.c` | rejected as not ARM |
| `dynamic.elf` | `zig cc -target arm-linux-gnueabi -mcpu=arm926ej_s -Os -s -Wl,--gc-sections -Wl,-z,max-page-size=4096 -o dynamic.elf m.c` | ARMv5TE soft-float linked against glibc: rejected for its interpreter and `libc.so.6` |

`-mcpu=cortex_a8-fpregs` drops the FPU so the ARMv7 fixture differs from the accepted one only in architecture. Zig does not link a dynamic musl executable for this target, so the dynamic fixture uses glibc.
