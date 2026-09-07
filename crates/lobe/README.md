# Lobe

Fast Brainfuck interpreter.

```bash
cargo install --git https://github.com/Aspenini/bf-tools lobe
lobe program.bf
lobe program.bf --bits 16            # wider cells
lobe program.bf --tape-size 100000   # for programs that outgrow 30,000 cells
```

`.` writes the cell as one raw byte, matching compiled Brainfuck.

Embedding it is cheap: `default-features = false` leaves the interpreter
without the argument parser the `lobe` command needs.

```toml
lobe = { version = "0.1", default-features = false }
```

Part of [bf-tools](https://github.com/Aspenini/bf-tools).
