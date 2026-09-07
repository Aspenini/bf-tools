# Cranium

[![Crates.io](https://img.shields.io/crates/v/cranium.svg)](https://crates.io/crates/cranium)

A small structured language that compiles to Brainfuck, so you can actually
write programs for the runtimes in [bf-tools](https://github.com/Aspenini/bf-tools).

```bash
cargo install cranium
cranium hello.cra --run              # compile and run it now
cranium hello.cra -o hello.bf        # or emit Brainfuck to compile or interpret
```

```rust
fn main() {
    println("Hello, world!");
}
```

The output is ordinary Brainfuck, so it runs anywhere:

```bash
cranium life.cra -o life.bf
lobe life.bf                         # interpret
hypothalamus life.bf -o life         # or compile to a native binary
```

## A taste

```rust
const WIDTH = 12;

let grid: byte[144];

fn living_neighbours(x: byte, y: byte) -> byte {
    let count = 0;
    for dy in 0..3 {
        for dx in 0..3 {
            if dx == 1 && dy == 1 { continue; }
            let nx = (x + dx + WIDTH - 1) % WIDTH;
            let ny = (y + dy + WIDTH - 1) % WIDTH;
            count += grid[ny * WIDTH + nx];
        }
    }
    return count;
}
```

See [`examples/`](examples) for Conway's Game of Life, an expression
calculator with real operator precedence, a bubble sort, and FizzBuzz.

## The language

### Types

| Type | Cells | Range |
| --- | --- | --- |
| `byte` | 1 | `0..=255`, wrapping |
| `int` | 2 | `0..=65535`, wrapping |
| `bool` | 1 | `true` / `false` |
| `T[N]` | see below | fixed-length array of scalars |

All integers are unsigned. `bool` and `byte` widen to `int` on their own;
narrowing needs a cast:

```rust
let wide: int = 300;
let narrow = wide as byte;     // 44
let flag = wide as bool;       // true
```

### Declarations

```rust
const LIMIT = 40;              // compile-time constant
let counter: int = 0;          // global when written outside a function

fn add(a: byte, b: byte) -> byte {
    return a + b;
}

fn main() {
    let x = 5;                 // byte, inferred
    let total: int = 0;        // explicit
    let buffer: byte[64];      // zero-filled
    let name: byte[16] = "cranium";   // NUL-terminated
    let primes = [2, 3, 5, 7, 11];    // byte[5]
}
```

Literals up to 255 are `byte`; larger ones are `int`.

### Expressions

`+ - * / %`, `== != < <= > >=`, `&& ||` (short-circuiting), `! -`,
`& | ^ << >>`, and `as`. Precedence is the usual one. Division by zero
saturates the quotient rather than trapping.

### Statements

`if` / `else if` / `else`, `while`, `loop`, `for i in a..b` (upper bound
exclusive), `break`, `continue`, `return`, nested `{ }` blocks, and
`= += -= *= /= %=`.

### Builtins

| Call | Effect |
| --- | --- |
| `print(x, ...)` | numbers as decimal, string literals as text, `byte` arrays up to their first `0` |
| `println(x, ...)` | the same, then a newline |
| `putc(x)` | write one raw byte |
| `getc()` | read one byte; `0` at end of input |
| `puts(a)` | write a `byte` array up to its first `0` |
| `len(a)` | the array's length, as `int` |

`print` always writes a *number*, so `print('A')` prints `65`. Use `putc('A')`
to write the character.

## How it works, and what that costs

A Brainfuck tape has no addresses and no call stack, which shapes three things.

**Every call is inlined**, so recursion is a compile-time error rather than a
program that quietly destroys itself:

```
3:12: `fact` calls itself; Brainfuck has no call stack, so Cranium inlines
      every call and cannot compile recursion
```

Write the loop instead — [`examples/calc.cra`](examples/calc.cra) is a full
expression parser without a single recursive call. Inlining also means a
function called from ten places emits its body ten times, so large helpers
called from many sites grow the output.

**Arrays are walked, not addressed.** `a[3]` is free — the compiler knows the
cell. `a[i]` is not: the program carries `i` rightwards one element at a time,
reads or writes there, and follows a trail of breadcrumbs back. That costs
roughly `i²` steps and gives each element a few cells of bookkeeping rather
than one. Prefer small arrays and constant indices in hot loops; a 12×12 grid
is comfortable, a 200×200 one is not.

**Ordering comparisons are the expensive scalar operation.** `==` and `!=` are
cheap, but `<`, `<=`, `>`, and `>=` cancel their operands against each other,
so comparing two large `byte` values costs more than comparing small ones.
Multiplication and division on `int` use shift-and-add, so those stay bounded
by the bit width.

`break`, `continue`, and `return` are implemented with a single control cell
that loops fold into their condition. Blocks that use none of those keywords
pay nothing for it.

Arrays passed to functions are passed by reference, so a callee writes through
to the caller's array.

## Tape size

`--stats` reports how much tape a program needs:

```bash
$ cranium life.cra --stats -o life.bf
cranium: 733021 brainfuck commands, 1224 tape cells
```

Past the traditional 30,000 cells, tell the runtime:

```bash
lobe big.bf --tape-size 100000
hypothalamus big.bf --tape-size 100000 -o big
```

## CLI

```
cranium <input.cra> [OPTIONS]

-o, --output <PATH>   Where to write the Brainfuck (default: input with a .bf extension)
    --emit <KIND>     bf (default), tokens, or ast
-r, --run             Run the program instead of writing it out
    --stats           Report program size and tape usage
```

## Limitations

- Unsigned only; no signed integers or floats.
- No recursion, function pointers, or structs.
- Arrays hold scalars and are not nested; lengths are literals.
- Values wider than `int` need to be built by hand.

Part of [bf-tools](https://github.com/Aspenini/bf-tools). MIT licensed.
