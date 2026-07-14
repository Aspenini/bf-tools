# Hypothalamus

[![Crates.io](https://img.shields.io/crates/v/hypothalamus.svg)](https://crates.io/crates/hypothalamus)

Optimizing Brainfuck AOT compiler with an LLVM IR backend.

```bash
cargo install hypothalamus
hypothalamus hello.bf -o hello
```

Requires `clang` for native emission. Part of [bf-tools](https://github.com/Aspenini/bf-tools).
