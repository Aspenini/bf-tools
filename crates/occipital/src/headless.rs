//! Running a program with no window.
//!
//! [`Headless`] is the same protocol and the same framebuffer as
//! [`crate::Console`], with scripted input instead of a keyboard and nothing
//! shown on screen. It exists so a graphical program can be tested by looking
//! at the pixels it produced, on a machine with no display.

use crate::protocol::{Command, Decoder, InputMode, event};
use crate::screen::Screen;
use hypothalamus::bf::Op;
use hypothalamus::codegen::CodegenOptions;
use hypothalamus::driver::OptLevel;
use hypothalamus::isa::IsaOptions;
use hypothalamus::jit::{self, Host, JitError};
use std::collections::VecDeque;
use std::io;

/// A [`Host`] that draws into a framebuffer and answers from a script.
#[derive(Debug, Default)]
pub struct Headless {
    screen: Screen,
    decoder: Decoder,
    mode: InputMode,
    input: VecDeque<u8>,
    presents: usize,
    quit: bool,
}

impl Headless {
    /// Create a host whose input is immediately exhausted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a host that answers reads from `input`, in order.
    ///
    /// In [`InputMode::Events`] the script is the event stream itself, so a key
    /// press is [`event::KEY_DOWN`] followed by its code.
    pub fn with_input(input: &[u8]) -> Self {
        Self {
            input: input.iter().copied().collect(),
            ..Self::default()
        }
    }

    /// The framebuffer as the program left it.
    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    /// How many times the program asked for a frame to be shown.
    pub fn presents(&self) -> usize {
        self.presents
    }

    /// Whether the program closed the window itself.
    pub fn has_quit(&self) -> bool {
        self.quit
    }

    /// The input mode the program last selected.
    pub fn mode(&self) -> InputMode {
        self.mode
    }

    /// Whether the program stopped midway through a command.
    pub fn is_mid_command(&self) -> bool {
        self.decoder.is_mid_command()
    }

    /// How many scripted bytes are still unread.
    pub fn remaining_input(&self) -> usize {
        self.input.len()
    }

    /// Compile `ops` for this machine and run it against a fresh host.
    ///
    /// # Errors
    ///
    /// Returns whatever the JIT reports if the program cannot be compiled for
    /// this machine.
    pub fn run(ops: &[Op], input: &[u8]) -> Result<Self, JitError> {
        let mut host = Self::with_input(input);
        jit::run(
            ops,
            &CodegenOptions::for_jit(),
            IsaOptions {
                opt_level: OptLevel::Speed,
                position_independent: false,
            },
            &mut host,
        )?;
        Ok(host)
    }
}

impl Host for Headless {
    fn write(&mut self, byte: u8) -> io::Result<()> {
        if let Some(command) = self.decoder.push(byte) {
            match command {
                Command::Present => self.presents += 1,
                Command::Quit => self.quit = true,
                Command::Mode(mode) => self.mode = mode,
                other => self.screen.apply(other),
            }
        }
        Ok(())
    }

    fn read(&mut self) -> io::Result<Option<u8>> {
        if let Some(byte) = self.input.pop_front() {
            return Ok(Some(byte));
        }

        Ok(match self.mode {
            // A polling loop would spin forever on `NONE`, so an exhausted
            // script reports the window closing. That is what ends the loop,
            // and it is why a test does not need to count frames.
            InputMode::Events => Some(event::QUIT),
            InputMode::Text => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{DLE, command, key};
    use crate::screen::{DEFAULT_BACKGROUND, cube_color};
    use hypothalamus::bf;

    /// Compile a Brainfuck program that writes `bytes`, so the host can be
    /// exercised the way a real program drives it.
    fn emitting(bytes: &[u8]) -> Vec<Op> {
        // `+` n times then `.`, resetting the cell between bytes.
        let mut source = Vec::new();
        for byte in bytes {
            source.extend(std::iter::repeat_n(b'+', *byte as usize));
            source.push(b'.');
            source.push(b'[');
            source.push(b'-');
            source.push(b']');
        }
        bf::parse(&source).expect("generated program should parse")
    }

    #[test]
    fn draws_what_a_program_writes() {
        let host = Headless::run(
            &emitting(&[DLE, command::COLOR, 200, DLE, command::PLOT, 4, 5]),
            b"",
        )
        .expect("program should run");

        assert_eq!(host.screen().pixel(4, 5), Some(200));
        assert_eq!(host.screen().pixel(5, 5), Some(DEFAULT_BACKGROUND));
    }

    #[test]
    fn counts_presents_without_touching_the_framebuffer() {
        let host = Headless::run(
            &emitting(&[DLE, command::PRESENT, DLE, command::PRESENT]),
            b"",
        )
        .expect("program should run");

        assert_eq!(host.presents(), 2);
    }

    #[test]
    fn notices_a_program_closing_the_window() {
        let host =
            Headless::run(&emitting(&[DLE, command::QUIT]), b"").expect("program should run");

        assert!(host.has_quit());
    }

    #[test]
    fn follows_the_program_into_event_mode() {
        let host =
            Headless::run(&emitting(&[DLE, command::MODE, 1]), b"").expect("program should run");

        assert_eq!(host.mode(), InputMode::Events);
    }

    #[test]
    fn text_reads_come_from_the_script() {
        // `,+.` echoes one byte incremented.
        let ops = bf::parse(b",+.").expect("valid Brainfuck");
        let host = Headless::run(&ops, b"A").expect("program should run");

        assert_eq!(host.screen().cursor(), (1, 0));
        assert_eq!(host.remaining_input(), 0);
    }

    #[test]
    fn an_exhausted_script_reports_the_window_closing() {
        let mut host = Headless::with_input(&[event::KEY_DOWN, key::LEFT]);
        host.mode = InputMode::Events;

        assert_eq!(host.read().unwrap(), Some(event::KEY_DOWN));
        assert_eq!(host.read().unwrap(), Some(key::LEFT));
        // Nothing scripted is left, so a polling program is told to stop
        // rather than spinning.
        assert_eq!(host.read().unwrap(), Some(event::QUIT));
        assert_eq!(host.read().unwrap(), Some(event::QUIT));
    }

    #[test]
    fn an_exhausted_script_is_end_of_input_in_text_mode() {
        let mut host = Headless::new();

        assert_eq!(host.read().unwrap(), None);
    }

    #[test]
    fn reports_a_program_that_stopped_midway_through_a_command() {
        let host =
            Headless::run(&emitting(&[DLE, command::PLOT, 1]), b"").expect("program should run");

        assert!(host.is_mid_command(), "a truncated command went unnoticed");
    }

    #[test]
    fn the_colour_cube_reaches_the_framebuffer() {
        let red = cube_color(5, 0, 0);
        let host = Headless::run(
            &emitting(&[DLE, command::COLOR, red, DLE, command::CLEAR]),
            b"",
        )
        .expect("program should run");

        assert_eq!(host.screen().pixel(0, 0), Some(red));
    }
}
