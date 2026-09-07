# bf-tools

Brainfuck tooling monorepo: write real programs, then compile or interpret them.

| Crate | What |
| --- | --- |
| [`cranium`](crates/cranium) | A structured language that compiles **to** Brainfuck |
| [`hypothalamus`](crates/hypothalamus) | AOT compiler (LLVM) |
| [`lobe`](crates/lobe) | Interpreter |

```bash
cargo install cranium hypothalamus lobe
```

The three fit together end to end:

```bash
cranium life.cra -o life.bf     # Cranium  -> Brainfuck
hypothalamus life.bf -o life    # Brainfuck -> native binary
lobe life.bf                    # or just interpret it
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
indexing, structured control flow, and functions — see its
[README](crates/cranium#the-language) for the language and for what a tape
without addresses or a call stack costs.

```bash
cargo test --workspace
```

MIT — see [LICENSE](LICENSE).
