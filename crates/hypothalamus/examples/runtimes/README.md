# Freestanding Runtime Notes

`--freestanding` emits a linkable Brainfuck payload instead of a hosted `main`,
for a runtime you provide.

The stable freestanding ABI is:

```c
extern void bf_main(void);

void bf_putchar(unsigned char byte);
int bf_getchar(void); /* return -1 for EOF / no input */
```

Rename any of the three with `--entry`, `--putchar-symbol`, and
`--getchar-symbol`.

The generated payload calls nothing else. The tape is a zero-initialized block
in the object's own BSS, and the code contains no libcalls, so a runtime does
not have to supply `memset` or any other compiler helper.

Objects are position-dependent, on the assumption that a freestanding runtime
places the payload itself rather than relocating it through a global offset
table.

## Building a payload

```bash
hypothalamus --target x86_64-none examples/hello.bf -o hello_bf.o
```

The `x86_64-none` preset is `x86_64-unknown-none-elf` with the freestanding ABI
and object output by default. Any other Cranelift target works the same way,
with no toolchain for that target installed:

```bash
hypothalamus --target aarch64-unknown-none-elf --freestanding examples/hello.bf -o hello_bf.o
hypothalamus --target riscv64-unknown-none-elf --freestanding examples/hello.bf -o hello_bf.o
```

A bare `-none` triple does not say which object format to write, so name one:
`aarch64-unknown-none-elf`, not `aarch64-unknown-none`.

Cranelift has backends for x86-64, aarch64, riscv64, and s390x. It has no
32-bit x86 or ARM backend, so 32-bit embedded targets are out of reach.

## Writing the runtime

Your runtime owns CPU setup, stack setup, the linker script, and the output
hardware. For a serial-only smoke test, make `bf_putchar` a byte write to the
serial port and `bf_getchar` `return -1;` until input exists. Then link your
startup and runtime objects with the payload, and call `bf_main` once
everything it needs is up.
