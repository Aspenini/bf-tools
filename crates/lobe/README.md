# Lobe

Fast Brainfuck interpreter.

```bash
cargo install --git https://github.com/Aspenini/bf-tools lobe
lobe program.bf
lobe program.bf --bits 16            # wider cells
lobe program.bf --tape-size 100000   # for programs that outgrow 30,000 cells
```

`.` writes the cell as one raw byte, matching compiled Brainfuck.

Part of [bf-tools](https://github.com/Aspenini/bf-tools).
