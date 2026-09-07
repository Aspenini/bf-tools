# Wernicke

A language server for [Cranium](../cranium), the structured language that
compiles to Brainfuck.

```bash
cargo install --git https://github.com/Aspenini/bf-tools wernicke
```

It speaks the language server protocol on standard input and output, so an
editor starts it rather than you. Point your editor's Cranium client at the
`wernicke` binary; `--stdio` is accepted and is the default.

## What it does

| | |
| --- | --- |
| Diagnostics | errors as you type, underlining the word rather than a point |
| Hover | signatures, the meaning of a type, what a builtin does |
| Go to definition | from a use to the `fn`, `const`, or `let` |
| Document symbols | the outline of a file |
| Completion | keywords, types, builtins, and everything the program defines |

Diagnostics follow `import`, and use the buffer you are editing rather than
what is on disk — so breaking a file that another one imports reports the
problem against the file it is in, before you save.

## How it works

There is no second model of the language. Diagnostics come from *compiling the
project for real*, which takes a few milliseconds, and everything else is read
off the same syntax tree the compiler uses. That means the server cannot drift
from the compiler: if it type checks here, it type checks in `cranium`.

The protocol is JSON-RPC over a pipe, so there are no dependencies beyond the
compiler itself. [`json`](src/json.rs) and [`rpc`](src/rpc.rs) are a few
hundred lines between them.

## Worth knowing

- **One error at a time.** Cranium stops at the first problem, so that is what
  gets reported. Fix it and the next one appears.
- **Only reachable code is checked.** Because Cranium inlines every call, a
  function nothing calls is never compiled, so a mistake inside it is not
  reported until something calls it.
- **Whole-document sync.** Every keystroke sends the whole file, which is
  simple and fast enough for source this size.

Named for Wernicke's area, the part of the brain that handles understanding
language. Part of [bf-tools](https://github.com/Aspenini/bf-tools). MIT.
