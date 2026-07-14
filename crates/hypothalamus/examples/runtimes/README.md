# Freestanding Runtime Notes

Most freestanding Hypothalamus target presets emit linkable Brainfuck payloads.
Complete-image targets also provide a tiny built-in runtime.

The stable freestanding ABI is:

```c
extern void bf_main(void);

void bf_putchar(unsigned char byte);
int bf_getchar(void); /* return -1 for EOF / no input */
```

## x86 Bare Metal

Build a payload object:

```sh
hypothalamus --target x86_64-none examples/hello.bf -o hello_bf.o
hypothalamus --target i386-none examples/hello.bf -o hello_bf.o
```

Your runtime owns CPU setup, stack setup, linker script, and output hardware.
For a tiny serial-only smoke test, provide `bf_putchar` as a byte write to the
serial port and `bf_getchar` as `return -1;` until input exists.

## Nintendo DS ARM9

Build an ARM9 payload object:

```sh
hypothalamus --target nds-arm9 examples/hello.bf -o hello_arm9.o
```

The `nds-arm9` preset selects `armv5te-none-eabi` with ARM946E-S CPU flags and
emits a freestanding object by default. It does not build a complete `.nds`
image yet; provide your own ARM7/runtime layer, linker script, and packaging.

`examples/runtimes/nds-arm9/` contains a minimal startup, linker script, and
runtime that links a Hypothalamus payload into an ARM9 ELF. The example stores
output bytes in memory and returns EOF for input; it is link groundwork, not a
complete DS program.

For devkitPro setup, see:

- <https://devkitpro.org/wiki/Getting_Started/Nintendo_DS>
- <https://devkitpro.org/wiki/Getting_Started/devkitPPC>

## Game Boy Advance

Build a complete ROM:

```sh
hypothalamus --target gba examples/hello.bf -o hello.gba
```

The `gba` preset selects `thumbv4t-none-eabi` with ARM7TDMI/Thumb flags, links
a tiny startup/runtime layer, extracts loadable ROM segments from the linked
ELF, and writes a valid GBA header.

The built-in v0 runtime displays output in Mode 3 text and returns `-1` from
`bf_getchar`, so input instructions see EOF. Use object output for a custom
runtime:

```sh
hypothalamus --target gba --emit obj examples/hello.bf -o hello_gba.o
```

GBA ROM builds prefer LLVM tools: `clang` for compilation and `ld.lld` for
linking. Hypothalamus checks `PATH` and tools placed beside the configured
`clang`, which keeps future bundled binary releases simple.

If `ld.lld` is unavailable, Hypothalamus falls back to devkitARM GCC for the
startup/runtime link step. It checks `PATH`, then `/opt/devkitpro/devkitARM/bin`.
Use `--gba-gcc <path>` to override discovery. `--gba-objcopy` is still accepted
for compatibility, but normal ROM builds no longer require objcopy.
