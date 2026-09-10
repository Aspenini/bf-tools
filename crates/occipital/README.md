# Occipital

A window for Brainfuck programs.

```bash
cargo install --git https://github.com/Aspenini/bf-tools occipital
occipital examples/bars.cra          # compiles the Cranium and runs it
occipital examples/bounce.cra        # arrows steer, q quits
occipital hello.bf                   # or a .bf file directly
```

Cranium's README explains what it could not do:

> It cannot do the other half of a terminal emulator — allocating a pseudo
> terminal, spawning a shell, drawing a window — because Brainfuck's whole
> interface is one byte in and one byte out, with no syscall to reach for.

Occipital does not widen that interface. It reads it differently. A program's
output is a command stream and its input is a keyboard, so a Cranium program
draws by printing, and the compiled program is still a `.bf` file that `lobe`
can interpret. Nothing was added to the language or the compiler.

Programs are compiled for the host by [`hypothalamus`](../hypothalamus)' JIT
and run in process, with the window standing in for the two I/O hooks. There
is no interpreter in the loop and no external toolchain.

## Text comes for free

Plain bytes are text, drawn with a built-in 5x7 font, with the control
characters a terminal would honour and scrolling at the bottom. So a program
written years before any of this works unchanged:

```bash
occipital ../cranium/examples/fizzbuzz.cra
```

The screen is 42x24 characters. Reads block for a keystroke by default, which
is what a program written for a terminal expects — except that a keystroke
arrives as it is typed, with no Enter needed and no line buffering.

## Drawing

256x192 pixels, 256 palette entries. Those numbers are chosen for the
language, not for nostalgia: every coordinate fits in one byte, and a byte is
the widest value Brainfuck adds without help.

[`examples/gfx.cra`](examples/gfx.cra) wraps the protocol, so a program says
what it means:

```rust
import "gfx.cra";

fn main() {
    color(rgb(0, 0, 1));
    clear();
    color(rgb(5, 3, 0));
    rect(40, 40, 10, 10);
    present();
}
```

Each of those is a handful of `putc` calls. A command is the byte `0x10`
followed by a command byte and its arguments:

| Command | Byte | Arguments |
| --- | --- | --- |
| Literal `0x10` | `0x00` | — |
| Clear | `0x01` | — |
| Colour | `0x02` | palette index |
| Plot | `0x03` | x, y |
| Rectangle | `0x04` | x, y, width, height |
| Line | `0x05` | x0, y0, x1, y1 |
| Present | `0x06` | — |
| Palette | `0x07` | index, red, green, blue |
| Cursor | `0x08` | column, row |
| Quit | `0x09` | — |
| Input mode | `0x0A` | 0 text, 1 events |

An unrecognized command byte is skipped on its own, so a program written
against a later version degrades instead of desynchronizing.

The default palette is the usual 256-colour terminal layout: sixteen named
colours, a 6x6x6 cube from index 16, then twenty-four greys. The cube is the
useful part, because a colour is arithmetic rather than a lookup —
`16 + 36 * red + 6 * green + blue` for components in `0..5`, which is what
`rgb` does.

Anything drawn shows up on its own within about a frame, so `present` is only
needed to pace an animation exactly.

## Input events

A game loop cannot block, so a program asks for events instead:

```rust
events_mode();

loop {
    let e = poll();
    if e == EV_QUIT {
        break;
    } else if e == EV_KEY_DOWN {
        let k = poll_key();
        if k == 'q' { quit(); break; }
        if k == KEY_LEFT { /* ... */ }
    }
}
```

After `events_mode()` a read returns immediately: `0` when nothing has
happened, or an event byte. A key event is followed by a second byte holding
the key code, which the program must also read. Letters and digits report
their unshifted ASCII, so `w` is `w` whatever shift is doing; arrows, escape,
shift and control get codes from `0x80` up. No key reports `0`, which is what
lets "nothing happened" be `0`.

## Testing a graphical program

`Headless` runs a program against the same framebuffer with scripted input and
no window, so a program's pixels can be checked on a machine with no display —
which is how this crate's own examples are tested.

```rust
use occipital::{Headless, Program};

let program = Program::load("bars.cra")?;
let host = Headless::run(program.ops(), b"")?;

assert_eq!(host.screen().pixel(0, 0), Some(16));
# Ok::<(), Box<dyn std::error::Error>>(())
```

Part of [bf-tools](https://github.com/Aspenini/bf-tools).
