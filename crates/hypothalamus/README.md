# Hypothalamus

Optimizing Brainfuck AOT compiler and JIT with a Cranelift backend.

```bash
cargo install --git https://github.com/Aspenini/bf-tools hypothalamus
hypothalamus hello.bf -o hello        # writes hello.exe on Windows
hypothalamus hello.bf --run           # or compile and run it in one step
```

The code generator is compiled in, so `--run` and `--emit obj` need no external
tools at all. Only `--emit exe` does, to link the object against the C runtime:
Hypothalamus looks for `cc`, `clang`, then `gcc`, and `--linker <path>` points
it somewhere else. `hypothalamus tools doctor` reports what it found.

`.` and `,` move raw bytes, so compiled programs produce the same output as
`lobe` on every platform — on Windows the generated program puts its standard
streams into binary mode rather than letting the C runtime rewrite newlines.

## Targets

`--emit obj` cross-compiles to any architecture Cranelift supports, with no
toolchain for that target installed:

```bash
hypothalamus --target x86_64-none hello.bf -o hello_bf.o
hypothalamus --target aarch64-unknown-none-elf --freestanding hello.bf -o hello_bf.o
```

`hypothalamus --list-targets` prints the presets. Any Cranelift target triple
works too, and a bare `-none` triple needs an explicit object format, so write
`aarch64-unknown-none-elf` rather than `aarch64-unknown-none`.

Cranelift's backends cover **x86-64, aarch64, riscv64, and s390x**. It has no
32-bit x86 or ARM backend, so those targets are not available.

`--freestanding` emits a callable payload instead of a hosted `main`; see
[`examples/runtimes/`](examples/runtimes) for the ABI.

## Optimization

`--opt-level` takes the familiar `0`–`3`, `s`, and `z` and folds them onto
Cranelift's three levels: `0` is `none`, `1`–`3` are `speed`, and `s`/`z` are
`speed_and_size`. Cranelift compiles fast enough that the default is fine even
for very large programs — these are the two biggest programs in this repo,
compiled and run on one machine, output discarded:

| program | `.bf` size | compile | native run | interpreted |
| --- | --- | --- | --- | --- |
| `life.cra` | 303 KB | 0.40 s | 0.055 s | 2.3 s |
| `bfi.cra` running `sierpinski.bf` | 344 KB | 0.23 s | 0.49 s | 208 s |

`--opt-level 0` roughly halves the compile time and costs about 10x at run
time, which is a worse trade than it used to be — there is little reason to
reach for it now.

Part of [bf-tools](https://github.com/Aspenini/bf-tools).
