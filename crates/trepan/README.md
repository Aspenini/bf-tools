# Trepan

Lifts Brainfuck back into [Cranium](../cranium). The other direction from
`cranium`, for the programs where it is possible.

```bash
cargo install --git https://github.com/Aspenini/bf-tools trepan

trepan hello.bf                  # writes hello.cra
trepan hello.bf -o -             # or to standard output
trepan hello.bf --stats          # how many cells became variables
```

Brainfuck has one anonymous tape and a pointer into it; Cranium has named
variables. The gap closes exactly when the pointer's position is known at every
point in the program, because then every `+`, `.` and `,` names one specific
cell — and that cell can simply be a variable.

The classic hello world goes in:

```brainfuck
++++++++[>+++++++>++++++++++>+++>+<<<<-]>++.>+.+++++++..+++.>>.<-.<.+++.------.--------.>>+.
```

and this comes out:

```rust
let c0 = 0;
let c1 = 0;
// ...

fn main() {
    c0 += 10;
    c1 += c0 * 7;
    c2 += c0 * 10;
    c3 += c0 * 3;
    c4 += c0;
    c0 = 0;
    c1 += 2;
    putc(c1);
    c2 += 1;
    putc(c2);
    // ...
}
```

The setup loop was a multiply-transfer, and it reads as one.

## What it will not lift

A loop that leaves the pointer somewhere other than where it found it — a
`[>]`-style scan, or any other unbalanced loop — makes the position after it
depend on the data. Those programs genuinely need the tape, and trepan declines
them rather than emitting something useless:

```
$ trepan sierpinski.bf
trepan: sierpinski.bf: a loop here leaves the pointer 2 cells from where it
started, so the position after it ran depends on how many times it ran and the
cells after it cannot become variables
```

It would be easy to emit `tape[p]` for every operation instead. It would also
be pointless: Cranium walks to a runtime index one cell at a time, so the
result would compile back into a far larger program than it started as. A
declined program is more useful than a useless one.

In practice the line falls where you would expect — **a program lifts when it
does not compute addresses**:

| program | cells | verdict |
| --- | --- | --- |
| `hello.bf` | 5 | lifts |
| `interpreter.bf` — a Brainfuck interpreter, 87 KB | 104 | lifts |
| FizzBuzz, compiled from Cranium | 49 | lifts |
| `sierpinski.bf` | — | scans the tape |
| `golden.bf` | — | scans the tape |

Cranium's own output lifts exactly when the source avoided arrays, since array
indexing is what emits the unbalanced loops.

## How it works

There is no second Brainfuck front end. `trepan` parses and optimizes with
[`hypothalamus`](../hypothalamus), whose IR is already offset-relative rather
than pointer-relative, already turns `[-]` into a store, and already recognizes
multiply-transfer loops as a single operation. That is most of a decompiler,
written for a compiler.

What is left is a check that the pointer never gets away, a pass to find which
cells are actually touched, and an emitter.

## Worth knowing

- **It is a decompiler, not an inverse.** Cranium inlines every call, so the
  functions are gone for good. Brainfuck has only `while`, so `if` and `for`
  are gone too, and so are types and names. Cells come back as `c0`, `c17` —
  their tape offsets, because Brainfuck kept nothing better.
- **Cells that are only passed over are not declared.** Only what a program
  reads or writes becomes a variable.
- **End of input differs.** `,` becomes `getc()`, which reads 0 at end of
  input. The Brainfuck runtimes disagree here — [`lobe`](../lobe) stores 0,
  [`hypothalamus`](../hypothalamus) leaves the cell alone — so a program that
  reads past the end of its input may not agree with the original. Everything
  else carries over exactly, because Brainfuck's wrapping byte cell is
  precisely Cranium's `byte`.

A trepan is the drill that opens a skull, which is how you get a look at what
is inside the cranium. Part of [bf-tools](https://github.com/Aspenini/bf-tools).
MIT.
