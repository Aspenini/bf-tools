//! The byte protocol between a Brainfuck program and the window.
//!
//! A Brainfuck program has one byte out and one byte in, so a graphical
//! interface has to be a stream of bytes and nothing else. Bytes a program
//! writes are text by default, drawn with the built-in font, which is what
//! makes an ordinary Cranium program work in the window unchanged. A command
//! is that stream escaped with [`DLE`], and every argument is one byte —
//! coordinates included — because arithmetic wider than a byte is expensive in
//! Brainfuck and the screen is sized to make it unnecessary.
//!
//! Going the other way, every read returns immediately in
//! [`InputMode::Events`] and blocks for a character in [`InputMode::Text`].

/// Introduces a command. `0x10` is ASCII "data link escape", which is what it
/// means here too.
///
/// A program that wants to draw a literal `0x10` writes [`DLE`] then
/// [`command::LITERAL`].
pub const DLE: u8 = 0x10;

/// Command bytes, each following a [`DLE`].
pub mod command {
    /// Draw a literal [`super::DLE`] as text. No arguments.
    pub const LITERAL: u8 = 0x00;

    /// Fill the screen with the current colour. No arguments.
    pub const CLEAR: u8 = 0x01;

    /// Set the current colour. One argument: a palette index.
    pub const COLOR: u8 = 0x02;

    /// Set one pixel. Two arguments: `x`, `y`.
    pub const PLOT: u8 = 0x03;

    /// Fill a rectangle. Four arguments: `x`, `y`, `width`, `height`.
    pub const RECT: u8 = 0x04;

    /// Draw a line. Four arguments: `x0`, `y0`, `x1`, `y1`.
    pub const LINE: u8 = 0x05;

    /// Show what has been drawn. No arguments.
    pub const PRESENT: u8 = 0x06;

    /// Set a palette entry. Four arguments: index, red, green, blue.
    pub const PALETTE: u8 = 0x07;

    /// Move the text cursor. Two arguments: column, row.
    pub const AT: u8 = 0x08;

    /// Close the window and end the program. No arguments.
    pub const QUIT: u8 = 0x09;

    /// Choose how reads behave. One argument: 0 for text, 1 for events.
    pub const MODE: u8 = 0x0A;
}

/// Event bytes, returned by a read in [`InputMode::Events`].
pub mod event {
    /// Nothing has happened. No further byte follows.
    pub const NONE: u8 = 0x00;

    /// A key went down. One byte follows: the key code.
    pub const KEY_DOWN: u8 = 0x01;

    /// A key came up. One byte follows: the key code.
    pub const KEY_UP: u8 = 0x02;

    /// The window was closed. No further byte follows.
    pub const QUIT: u8 = 0x03;
}

/// Key codes for keys that have no ASCII spelling.
///
/// Every other key reports its unshifted ASCII byte, so `a` is `0x61` whether
/// or not shift is held. No key reports `0`, which is why
/// [`event::NONE`] can be `0`.
pub mod key {
    /// Up arrow.
    pub const UP: u8 = 0x80;

    /// Down arrow.
    pub const DOWN: u8 = 0x81;

    /// Left arrow.
    pub const LEFT: u8 = 0x82;

    /// Right arrow.
    pub const RIGHT: u8 = 0x83;

    /// Escape.
    pub const ESCAPE: u8 = 0x84;

    /// Either shift key.
    pub const SHIFT: u8 = 0x85;

    /// Either control key.
    pub const CONTROL: u8 = 0x86;
}

/// How a program's reads behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    /// Reads block until a key produces a character, and return its ASCII
    /// byte. Enter reads as `\n`.
    ///
    /// This is the default because it is what a program written for a terminal
    /// already expects.
    #[default]
    Text,

    /// Reads return immediately with an [`event`] byte, so a program can poll
    /// in a loop without blocking.
    Events,
}

/// A decoded command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Draw one byte as text at the cursor.
    Text(u8),

    /// Fill the screen with the current colour.
    Clear,

    /// Set the current colour.
    Color(u8),

    /// Set one pixel.
    Plot {
        /// Column.
        x: u8,
        /// Row.
        y: u8,
    },

    /// Fill a rectangle.
    Rect {
        /// Left edge.
        x: u8,
        /// Top edge.
        y: u8,
        /// Width in pixels.
        width: u8,
        /// Height in pixels.
        height: u8,
    },

    /// Draw a line between two points, inclusive.
    Line {
        /// First column.
        x0: u8,
        /// First row.
        y0: u8,
        /// Second column.
        x1: u8,
        /// Second row.
        y1: u8,
    },

    /// Show what has been drawn.
    Present,

    /// Set a palette entry.
    Palette {
        /// Entry to change.
        index: u8,
        /// Red component.
        red: u8,
        /// Green component.
        green: u8,
        /// Blue component.
        blue: u8,
    },

    /// Move the text cursor.
    At {
        /// Character column.
        column: u8,
        /// Character row.
        row: u8,
    },

    /// Close the window.
    Quit,

    /// Change how reads behave.
    Mode(InputMode),
}

/// Turns a byte stream into [`Command`]s.
///
/// Bytes arrive one at a time, because that is how a Brainfuck program produces
/// them. A command that needs arguments yields nothing until its last argument
/// arrives.
#[derive(Debug, Default)]
pub struct Decoder {
    state: State,
}

#[derive(Debug, Default, Clone, Copy)]
enum State {
    /// Reading text.
    #[default]
    Text,

    /// A [`DLE`] arrived; the next byte says which command.
    Escaped,

    /// Collecting a command's arguments.
    Arguments {
        command: u8,
        buffer: [u8; 4],
        filled: usize,
        needed: usize,
    },
}

impl Decoder {
    /// Create a decoder positioned at the start of a stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one byte, returning a command once one is complete.
    ///
    /// An unrecognized command byte is discarded, along with nothing else, so a
    /// program written against a later version of the protocol degrades rather
    /// than desynchronizing the whole stream.
    pub fn push(&mut self, byte: u8) -> Option<Command> {
        match self.state {
            State::Text => {
                if byte == DLE {
                    self.state = State::Escaped;
                    return None;
                }
                Some(Command::Text(byte))
            }
            State::Escaped => {
                self.state = State::Text;
                match arity(byte) {
                    None => None,
                    Some(0) => complete(byte, &[]),
                    Some(needed) => {
                        self.state = State::Arguments {
                            command: byte,
                            buffer: [0; 4],
                            filled: 0,
                            needed,
                        };
                        None
                    }
                }
            }
            State::Arguments {
                command,
                mut buffer,
                filled,
                needed,
            } => {
                buffer[filled] = byte;
                let filled = filled + 1;
                if filled < needed {
                    self.state = State::Arguments {
                        command,
                        buffer,
                        filled,
                        needed,
                    };
                    return None;
                }

                self.state = State::Text;
                complete(command, &buffer[..needed])
            }
        }
    }

    /// Whether the decoder is midway through a command.
    ///
    /// A stream that ends here was cut off; the window reports it rather than
    /// pretending the last command arrived.
    pub fn is_mid_command(&self) -> bool {
        !matches!(self.state, State::Text)
    }
}

/// How many argument bytes a command takes, or `None` if it is unrecognized.
fn arity(command: u8) -> Option<usize> {
    Some(match command {
        command::LITERAL | command::CLEAR | command::PRESENT | command::QUIT => 0,
        command::COLOR | command::MODE => 1,
        command::PLOT | command::AT => 2,
        command::RECT | command::LINE | command::PALETTE => 4,
        _ => return None,
    })
}

fn complete(command: u8, arguments: &[u8]) -> Option<Command> {
    Some(match (command, arguments) {
        (command::LITERAL, []) => Command::Text(DLE),
        (command::CLEAR, []) => Command::Clear,
        (command::PRESENT, []) => Command::Present,
        (command::QUIT, []) => Command::Quit,
        (command::COLOR, [color]) => Command::Color(*color),
        (command::MODE, [mode]) => Command::Mode(if *mode == 0 {
            InputMode::Text
        } else {
            InputMode::Events
        }),
        (command::PLOT, [x, y]) => Command::Plot { x: *x, y: *y },
        (command::AT, [column, row]) => Command::At {
            column: *column,
            row: *row,
        },
        (command::RECT, [x, y, width, height]) => Command::Rect {
            x: *x,
            y: *y,
            width: *width,
            height: *height,
        },
        (command::LINE, [x0, y0, x1, y1]) => Command::Line {
            x0: *x0,
            y0: *y0,
            x1: *x1,
            y1: *y1,
        },
        (command::PALETTE, [index, red, green, blue]) => Command::Palette {
            index: *index,
            red: *red,
            green: *green,
            blue: *blue,
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed a whole stream and collect everything it decodes to.
    fn decode(bytes: &[u8]) -> Vec<Command> {
        let mut decoder = Decoder::new();
        bytes
            .iter()
            .filter_map(|byte| decoder.push(*byte))
            .collect()
    }

    #[test]
    fn plain_bytes_are_text() {
        assert_eq!(decode(b"hi"), [Command::Text(b'h'), Command::Text(b'i')]);
    }

    #[test]
    fn decodes_commands_without_arguments() {
        assert_eq!(
            decode(&[
                DLE,
                command::CLEAR,
                DLE,
                command::PRESENT,
                DLE,
                command::QUIT
            ]),
            [Command::Clear, Command::Present, Command::Quit]
        );
    }

    #[test]
    fn decodes_commands_with_arguments() {
        assert_eq!(
            decode(&[DLE, command::PLOT, 10, 20]),
            [Command::Plot { x: 10, y: 20 }]
        );
        assert_eq!(
            decode(&[DLE, command::RECT, 1, 2, 3, 4]),
            [Command::Rect {
                x: 1,
                y: 2,
                width: 3,
                height: 4
            }]
        );
        assert_eq!(
            decode(&[DLE, command::PALETTE, 7, 255, 128, 0]),
            [Command::Palette {
                index: 7,
                red: 255,
                green: 128,
                blue: 0
            }]
        );
    }

    #[test]
    fn arguments_may_arrive_one_byte_at_a_time() {
        let mut decoder = Decoder::new();

        // Nothing is reported until the last argument lands.
        for byte in [DLE, command::LINE, 1, 2, 3] {
            assert_eq!(decoder.push(byte), None);
            assert!(decoder.is_mid_command());
        }

        assert_eq!(
            decoder.push(4),
            Some(Command::Line {
                x0: 1,
                y0: 2,
                x1: 3,
                y1: 4
            })
        );
        assert!(!decoder.is_mid_command());
    }

    #[test]
    fn an_escaped_introducer_is_text() {
        assert_eq!(decode(&[DLE, command::LITERAL]), [Command::Text(DLE)]);
    }

    #[test]
    fn argument_bytes_are_never_mistaken_for_commands() {
        // An argument that happens to equal DLE is still just an argument.
        assert_eq!(
            decode(&[DLE, command::PLOT, DLE, DLE]),
            [Command::Plot { x: DLE, y: DLE }]
        );
    }

    #[test]
    fn an_unknown_command_is_skipped_without_losing_the_stream() {
        assert_eq!(
            decode(&[DLE, 0x7F, b'o', b'k']),
            [Command::Text(b'o'), Command::Text(b'k')]
        );
    }

    #[test]
    fn decodes_both_input_modes() {
        assert_eq!(
            decode(&[DLE, command::MODE, 1, DLE, command::MODE, 0]),
            [
                Command::Mode(InputMode::Events),
                Command::Mode(InputMode::Text)
            ]
        );
    }

    #[test]
    fn text_is_the_default_mode() {
        assert_eq!(InputMode::default(), InputMode::Text);
    }

    #[test]
    fn no_key_code_collides_with_the_no_event_byte() {
        for code in [
            key::UP,
            key::DOWN,
            key::LEFT,
            key::RIGHT,
            key::ESCAPE,
            key::SHIFT,
            key::CONTROL,
        ] {
            assert_ne!(code, event::NONE);
        }
    }
}
