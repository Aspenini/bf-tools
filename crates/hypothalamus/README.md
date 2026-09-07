# Hypothalamus

[![Crates.io](https://img.shields.io/crates/v/hypothalamus.svg)](https://crates.io/crates/hypothalamus)

Optimizing Brainfuck AOT compiler with an LLVM IR backend.

```bash
cargo install hypothalamus
hypothalamus hello.bf -o hello        # writes hello.exe on Windows
hypothalamus big.bf -o big --opt-level 1   # -O2 gets slow on large programs
```

Requires `clang` for native emission; pass `--cc <path>` if it is not on your
`PATH`. Part of [bf-tools](https://github.com/Aspenini/bf-tools).
