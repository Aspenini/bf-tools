# Hypothalamus

Optimizing Brainfuck AOT compiler with an LLVM IR backend.

```bash
cargo install --git https://github.com/Aspenini/bf-tools hypothalamus
hypothalamus hello.bf -o hello        # writes hello.exe on Windows
hypothalamus big.bf -o big --opt-level 1   # -O2 gets slow on large programs
```

Requires `clang` for native emission; pass `--cc <path>` if it is not on your
`PATH`.

`.` and `,` move raw bytes, so compiled programs produce the same output as
`lobe` on every platform — on Windows the generated program puts its standard
streams into binary mode rather than letting the C runtime rewrite newlines. Part of [bf-tools](https://github.com/Aspenini/bf-tools).
