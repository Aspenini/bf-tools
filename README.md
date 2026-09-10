# bf-tools

Brainfuck tooling monorepo: write real programs, then compile or interpret them.

| Crate | What |
| --- | --- |
| [`cranium-lang`](crates/cranium) | A structured language that compiles **to** Brainfuck |
| [`hypothalamus`](crates/hypothalamus) | AOT compiler and JIT (Cranelift) |
| [`lobe`](crates/lobe) | Interpreter |
| [`wernicke`](crates/wernicke) | Language server for Cranium |
| [`trepan`](crates/trepan) | Lifts Brainfuck back into Cranium |

Editor support: a [Zed extension](editors/zed) and the
[tree-sitter grammar](editors/tree-sitter-cranium) behind it.

```bash
cargo install --git https://github.com/Aspenini/bf-tools cranium-lang hypothalamus lobe wernicke trepan
```

The three fit together end to end:

```bash
cranium life.cra -o life.bf     # Cranium  -> Brainfuck
hypothalamus life.bf -o life    # Brainfuck -> native binary
lobe life.bf                    # or just interpret it
```

`trepan` goes the other way, turning Brainfuck back into Cranium for programs
that never compute an address - including
[`interpreter.bf`](crates/hypothalamus/examples/interpreter.bf), an 87 KB
Brainfuck interpreter that comes back as 104 named variables and no array.

Far enough that Cranium's examples include both a Brainfuck interpreter and
`lobotomy`, an optimizing ahead-of-time Brainfuck compiler. Compiled, each is
itself a Brainfuck program:

```bash
# a Brainfuck interpreter, running as Brainfuck
cranium crates/cranium/examples/bfi.cra -o bfi.bf
hypothalamus bfi.bf -o bfi
{ cat crates/lobe/bf/sierpinski.bf; echo '!'; } | ./bfi

# a Brainfuck compiler, running as Brainfuck
cranium crates/cranium/examples/lobotomy.cra -o lobotomy.bf
hypothalamus lobotomy.bf -o lobotomy
./lobotomy < crates/lobe/bf/sierpinski.bf > sierpinski.c
clang -O2 sierpinski.c -o sierpinski && ./sierpinski
```

```rust
// life.cra
fn main() {
    for n in 1..101 {
        if n % 15 == 0 { println("FizzBuzz"); }
        else if n % 3 == 0 { println("Fizz"); }
        else if n % 5 == 0 { println("Buzz"); }
        else { println(n); }
    }
}
```

Cranium has variables, `int` and `byte` arithmetic, arrays with runtime
indexing, structured control flow, and functions — enough that one of its
[examples](crates/cranium/examples/bfi.cra) is a Brainfuck interpreter, which
compiles to Brainfuck interpreting Brainfuck. See its
[README](crates/cranium#the-language) for the language and for what a tape
without addresses or a call stack costs.

```bash
cargo test --workspace
```

MIT — see [LICENSE](LICENSE).
