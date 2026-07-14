# Owned Brainfuck Interpreter Fixture

`interpreter.bf` is Hypothalamus' owned Brainfuck-in-Brainfuck test fixture.
It reads an interpreted Brainfuck program, then `!`, then input for that
interpreted program:

```text
<program>!<program input>
```

Examples:

```sh
cargo run -- -O3 examples/interpreter.bf -o /tmp/hypothalamus-bfi
printf ',+.!A' | /tmp/hypothalamus-bfi
printf '++[>++[>++<-]<-]>>.!' | /tmp/hypothalamus-bfi | od -An -t u1
```

This is intentionally bounded so it stays useful as a compiler fixture:

- maximum interpreted program length: 24 Brainfuck commands;
- interpreted data tape length: 8 cells;
- EOF behavior for interpreted `,` follows Hypothalamus' generated input
  behavior and leaves the current cell unchanged;
- out-of-bounds interpreted pointer movement is unsupported.

The `.bf` file is generated from an internal macro design and contains only
Brainfuck commands plus whitespace.
