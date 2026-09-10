//! The window a Brainfuck program draws into.
//!
//! [`Console`] implements [`hypothalamus::jit::Host`], so the compiled program
//! calls straight into it: every `.` is a byte for the decoder, every `,` is a
//! keystroke or an event. The window is driven from those callbacks rather than
//! from a loop of its own, which keeps everything on one thread and means the
//! program itself decides the frame rate.
//!
//! A program that computes for a long time without any I/O will not pump the
//! window while it does, so the window stops repainting until the next `.` or
//! `,`. Anything drawing or polling — which is any interactive program — pumps
//! it constantly.

use crate::protocol::{Command, Decoder, InputMode, event, key};
use crate::screen::{self, Screen};
use hypothalamus::jit::Host;
use minifb::{Key, KeyRepeat, Scale, ScaleMode, Window, WindowOptions};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Longest a frame is held before an automatic repaint.
///
/// A program that never sends `PRESENT` — anything written for a terminal —
/// still animates, because drawing marks the screen dirty and the next byte
/// after this long repaints it.
const FRAME: Duration = Duration::from_millis(16);

/// How long a blocking read waits between polls of the window.
const POLL: Duration = Duration::from_millis(2);

/// How the window is set up.
#[derive(Debug, Clone)]
pub struct Options {
    /// Window title.
    pub title: String,

    /// Whole-number magnification of the 256x192 screen.
    pub scale: u8,

    /// Keep the window open after the program ends, until it is closed.
    pub wait_on_exit: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            title: "occipital".to_string(),
            scale: 3,
            wait_on_exit: true,
        }
    }
}

/// A window, a framebuffer, and a keyboard, behind one byte in and one byte out.
pub struct Console {
    screen: Screen,
    decoder: Decoder,
    window: Window,
    buffer: Vec<u32>,
    mode: InputMode,
    /// Characters the window has reported, waiting to be read in text mode.
    typed: Rc<RefCell<VecDeque<u8>>>,
    /// Event bytes waiting to be read in event mode.
    events: VecDeque<u8>,
    last_present: Instant,
    /// Set once the window has gone, whether the program noticed or not.
    closed: bool,
    wait_on_exit: bool,
}

impl Console {
    /// Open a window.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform could not create a window, which on a
    /// machine with no display is the usual outcome.
    pub fn open(options: &Options) -> Result<Self, minifb::Error> {
        let mut window = Window::new(
            &options.title,
            screen::WIDTH,
            screen::HEIGHT,
            WindowOptions {
                scale: scale_of(options.scale),
                scale_mode: ScaleMode::Stretch,
                resize: true,
                ..WindowOptions::default()
            },
        )?;

        // The program sets its own pace; minifb must not sleep on its behalf.
        window.set_target_fps(0);

        let typed = Rc::new(RefCell::new(VecDeque::new()));
        window.set_input_callback(Box::new(Typed {
            queue: Rc::clone(&typed),
        }));

        Ok(Self {
            screen: Screen::new(),
            decoder: Decoder::new(),
            window,
            buffer: Vec::with_capacity(screen::WIDTH * screen::HEIGHT),
            mode: InputMode::default(),
            typed,
            events: VecDeque::new(),
            last_present: Instant::now(),
            closed: false,
            wait_on_exit: options.wait_on_exit,
        })
    }

    /// The framebuffer, for tests and for callers that want the final image.
    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    /// Whether the window has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Show the last frame and, if asked, hold the window open until it is
    /// closed.
    ///
    /// A program's final frame is usually the interesting one, so it would be
    /// unhelpful for the window to vanish the moment the program returns.
    pub fn finish(&mut self) {
        if self.closed {
            return;
        }

        self.present();
        if !self.wait_on_exit {
            return;
        }

        self.window
            .set_title(&format!("{} - finished", self.window_title_prefix()));
        while self.window.is_open() && !self.window.is_key_down(Key::Escape) {
            self.present();
            std::thread::sleep(POLL);
        }
        self.closed = true;
    }

    /// Report an unfinished command, for a caller that wants to warn about it.
    pub fn is_mid_command(&self) -> bool {
        self.decoder.is_mid_command()
    }

    fn window_title_prefix(&self) -> String {
        // minifb has no getter for the title, so this is the one place the
        // default is repeated.
        "occipital".to_string()
    }

    /// Push the framebuffer to the window and pump its event queue.
    fn present(&mut self) {
        if self.closed {
            return;
        }
        if !self.window.is_open() {
            self.closed = true;
            return;
        }

        self.screen.render(&mut self.buffer);
        if self
            .window
            .update_with_buffer(&self.buffer, screen::WIDTH, screen::HEIGHT)
            .is_err()
        {
            self.closed = true;
            return;
        }

        self.screen.take_dirty();
        self.last_present = Instant::now();
    }

    /// Pump the window without repainting, then repaint if a frame is due.
    fn pump(&mut self) {
        if self.closed {
            return;
        }
        if !self.window.is_open() {
            self.closed = true;
            return;
        }

        if self.screen.is_dirty() && self.last_present.elapsed() >= FRAME {
            self.present();
        } else {
            self.window.update();
        }
    }

    /// Move key presses and releases into the event queue.
    fn collect_events(&mut self) {
        for pressed in self.window.get_keys_pressed(KeyRepeat::No) {
            if let Some(code) = key_code(pressed) {
                self.events.push_back(event::KEY_DOWN);
                self.events.push_back(code);
            }
        }
        for released in self.window.get_keys_released() {
            if let Some(code) = key_code(released) {
                self.events.push_back(event::KEY_UP);
                self.events.push_back(code);
            }
        }
    }

    /// Move the control keys that `add_char` does not report into the text
    /// queue.
    ///
    /// minifb reports typed text as Unicode, which deliberately leaves out
    /// control characters, but a program reading a line needs at least Enter
    /// and Backspace.
    fn collect_control_keys(&mut self) {
        for pressed in self.window.get_keys_pressed(KeyRepeat::Yes) {
            let byte = match pressed {
                Key::Enter | Key::NumPadEnter => b'\n',
                Key::Backspace => 0x08,
                Key::Tab => b'\t',
                Key::Escape => 0x1B,
                _ => continue,
            };
            self.typed.borrow_mut().push_back(byte);
        }
    }

    fn read_event(&mut self) -> Option<u8> {
        self.pump();
        if self.closed {
            // The program may not act on this, but a window that is gone
            // cannot come back, so keep saying so rather than reporting end of
            // input and leaving a polling loop spinning on a stale cell.
            return Some(event::QUIT);
        }

        // A key event is two bytes, so anything already queued comes first.
        if let Some(byte) = self.events.pop_front() {
            return Some(byte);
        }

        self.collect_events();
        Some(self.events.pop_front().unwrap_or(event::NONE))
    }

    fn read_text(&mut self) -> Option<u8> {
        loop {
            self.pump();
            if self.closed {
                return None;
            }

            self.collect_control_keys();
            if let Some(byte) = self.typed.borrow_mut().pop_front() {
                return Some(byte);
            }

            std::thread::sleep(POLL);
        }
    }
}

impl Host for Console {
    fn write(&mut self, byte: u8) -> io::Result<()> {
        if let Some(command) = self.decoder.push(byte) {
            match command {
                Command::Present => {
                    self.present();
                    return Ok(());
                }
                Command::Quit => {
                    self.present();
                    self.closed = true;
                    return Ok(());
                }
                Command::Mode(mode) => {
                    self.mode = mode;
                    // Text typed before the switch is not an event stream, and
                    // events queued before it are not text.
                    self.typed.borrow_mut().clear();
                    self.events.clear();
                    return Ok(());
                }
                other => self.screen.apply(other),
            }
        }

        // Repaint on a timer so a program that never presents still animates.
        if self.screen.is_dirty() && self.last_present.elapsed() >= FRAME {
            self.present();
        }
        Ok(())
    }

    fn read(&mut self) -> io::Result<Option<u8>> {
        // Anything drawn should be on screen before the program waits on a
        // reply to it.
        if self.screen.is_dirty() {
            self.present();
        }

        Ok(match self.mode {
            InputMode::Events => self.read_event(),
            InputMode::Text => self.read_text(),
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        self.present();
        Ok(())
    }
}

/// Collects the Unicode characters minifb reports as they are typed.
struct Typed {
    queue: Rc<RefCell<VecDeque<u8>>>,
}

impl minifb::InputCallback for Typed {
    fn add_char(&mut self, character: u32) {
        // The protocol is bytes, so anything outside Latin-1 has no spelling
        // a Brainfuck program could use and is dropped.
        if let Some(byte) = char::from_u32(character).and_then(|character| {
            u8::try_from(u32::from(character)).ok().filter(|_| {
                // Control characters arrive through `collect_control_keys`
                // instead, so taking them here as well would double them up.
                !character.is_control()
            })
        }) {
            self.queue.borrow_mut().push_back(byte);
        }
    }
}

/// Map a window scale onto minifb's fixed set.
///
/// Anything in between rounds down to the next supported step, so an odd
/// `--scale` gives a smaller window rather than an error.
fn scale_of(scale: u8) -> Scale {
    match scale {
        0 | 1 => Scale::X1,
        2 | 3 => Scale::X2,
        4..=7 => Scale::X4,
        8..=15 => Scale::X8,
        16..=31 => Scale::X16,
        _ => Scale::X32,
    }
}

/// The protocol byte for a key, or `None` for a key with no spelling.
///
/// Letters report their unshifted ASCII, so a game can read `w` without caring
/// about shift or caps lock.
fn key_code(pressed: Key) -> Option<u8> {
    Some(match pressed {
        Key::A => b'a',
        Key::B => b'b',
        Key::C => b'c',
        Key::D => b'd',
        Key::E => b'e',
        Key::F => b'f',
        Key::G => b'g',
        Key::H => b'h',
        Key::I => b'i',
        Key::J => b'j',
        Key::K => b'k',
        Key::L => b'l',
        Key::M => b'm',
        Key::N => b'n',
        Key::O => b'o',
        Key::P => b'p',
        Key::Q => b'q',
        Key::R => b'r',
        Key::S => b's',
        Key::T => b't',
        Key::U => b'u',
        Key::V => b'v',
        Key::W => b'w',
        Key::X => b'x',
        Key::Y => b'y',
        Key::Z => b'z',
        Key::Key0 | Key::NumPad0 => b'0',
        Key::Key1 | Key::NumPad1 => b'1',
        Key::Key2 | Key::NumPad2 => b'2',
        Key::Key3 | Key::NumPad3 => b'3',
        Key::Key4 | Key::NumPad4 => b'4',
        Key::Key5 | Key::NumPad5 => b'5',
        Key::Key6 | Key::NumPad6 => b'6',
        Key::Key7 | Key::NumPad7 => b'7',
        Key::Key8 | Key::NumPad8 => b'8',
        Key::Key9 | Key::NumPad9 => b'9',
        Key::Space => b' ',
        Key::Enter | Key::NumPadEnter => b'\n',
        Key::Tab => b'\t',
        Key::Backspace => 0x08,
        Key::Comma => b',',
        Key::Period => b'.',
        Key::Slash => b'/',
        Key::Semicolon => b';',
        Key::Apostrophe => b'\'',
        Key::LeftBracket => b'[',
        Key::RightBracket => b']',
        Key::Backslash => b'\\',
        Key::Minus => b'-',
        Key::Equal => b'=',
        Key::Backquote => b'`',
        Key::Up => key::UP,
        Key::Down => key::DOWN,
        Key::Left => key::LEFT,
        Key::Right => key::RIGHT,
        Key::Escape => key::ESCAPE,
        Key::LeftShift | Key::RightShift => key::SHIFT,
        Key::LeftCtrl | Key::RightCtrl => key::CONTROL,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::command;
    use minifb::InputCallback;

    #[test]
    fn scales_round_down_to_a_supported_step() {
        assert!(matches!(scale_of(0), Scale::X1));
        assert!(matches!(scale_of(1), Scale::X1));
        assert!(matches!(scale_of(4), Scale::X4));
        assert!(matches!(scale_of(5), Scale::X4));
        assert!(matches!(scale_of(255), Scale::X32));
    }

    #[test]
    fn letters_report_their_unshifted_ascii() {
        assert_eq!(key_code(Key::W), Some(b'w'));
        assert_eq!(key_code(Key::Z), Some(b'z'));
        assert_eq!(key_code(Key::Key7), Some(b'7'));
    }

    #[test]
    fn keys_without_ascii_get_codes_above_it() {
        for pressed in [Key::Up, Key::Down, Key::Left, Key::Right, Key::Escape] {
            let code = key_code(pressed).expect("arrow keys have codes");
            assert!(code >= 0x80, "{pressed:?} collided with ASCII");
        }
    }

    #[test]
    fn no_key_reports_the_no_event_byte() {
        // `0` has to stay free, because it is how "nothing happened" is spelled.
        let every_key = [
            Key::A,
            Key::Z,
            Key::Key0,
            Key::Space,
            Key::Enter,
            Key::Tab,
            Key::Backspace,
            Key::Up,
            Key::Escape,
            Key::LeftShift,
            Key::LeftCtrl,
        ];
        for pressed in every_key {
            assert_ne!(key_code(pressed), Some(event::NONE));
        }
    }

    #[test]
    fn unmapped_keys_are_dropped() {
        assert_eq!(key_code(Key::F1), None);
        assert_eq!(key_code(Key::Insert), None);
    }

    #[test]
    fn typed_text_drops_control_characters_and_wide_ones() {
        let queue = Rc::new(RefCell::new(VecDeque::new()));
        let mut typed = Typed {
            queue: Rc::clone(&queue),
        };

        typed.add_char(u32::from('A'));
        typed.add_char(u32::from('\n')); // control: comes from the key instead
        typed.add_char(0x2603); // snowman: no byte spelling
        typed.add_char(u32::from('é')); // Latin-1: fits a byte

        let queue = queue.borrow();
        assert_eq!(queue.iter().copied().collect::<Vec<_>>(), [b'A', 0xE9]);
    }

    #[test]
    fn the_frame_budget_is_about_sixty_a_second() {
        assert!(FRAME.as_millis() > 0 && FRAME.as_millis() <= 20);
    }

    #[test]
    fn text_is_the_starting_input_mode() {
        // A program written for a terminal must not have to opt in.
        assert_eq!(InputMode::default(), InputMode::Text);
        assert_eq!(command::MODE, 0x0A);
    }
}
