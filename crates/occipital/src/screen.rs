//! The framebuffer a program draws into, and the text console layered on it.
//!
//! The screen is 256x192 palette-indexed pixels. That size is not nostalgia:
//! it means every coordinate a program sends fits in one byte, and a byte is
//! the widest value Brainfuck adds without help.
//!
//! Nothing here knows about windows, so all of it can be tested by looking at
//! the pixels afterwards.

use crate::font;
use crate::protocol::Command;

/// Screen width in pixels.
pub const WIDTH: usize = 256;

/// Screen height in pixels.
pub const HEIGHT: usize = 192;

/// Width of one character cell: a glyph plus a column of spacing.
pub const CELL_WIDTH: usize = font::WIDTH + 1;

/// Height of one character cell: a glyph plus a row of spacing.
pub const CELL_HEIGHT: usize = font::HEIGHT + 1;

/// Character columns that fit across the screen.
pub const COLUMNS: usize = WIDTH / CELL_WIDTH;

/// Character rows that fit down the screen.
pub const ROWS: usize = HEIGHT / CELL_HEIGHT;

/// Palette index the screen starts cleared to.
pub const DEFAULT_BACKGROUND: u8 = 0;

/// Palette index text and drawing start in.
pub const DEFAULT_COLOR: u8 = 15;

/// How many columns a tab advances to.
const TAB_WIDTH: usize = 4;

/// A palette-indexed framebuffer with a text cursor.
#[derive(Debug, Clone)]
pub struct Screen {
    pixels: Vec<u8>,
    palette: [u32; 256],
    color: u8,
    column: usize,
    row: usize,
    dirty: bool,
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

impl Screen {
    /// Create a screen cleared to the background colour.
    pub fn new() -> Self {
        Self {
            pixels: vec![DEFAULT_BACKGROUND; WIDTH * HEIGHT],
            palette: default_palette(),
            color: DEFAULT_COLOR,
            column: 0,
            row: 0,
            dirty: true,
        }
    }

    /// Apply one decoded command.
    ///
    /// [`Command::Present`] and [`Command::Quit`] are the window's business,
    /// not the framebuffer's, and are ignored here.
    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Text(byte) => self.write_text(byte),
            Command::Clear => self.clear(),
            Command::Color(color) => self.color = color,
            Command::Plot { x, y } => self.plot(x as usize, y as usize),
            Command::Rect {
                x,
                y,
                width,
                height,
            } => self.rect(x as usize, y as usize, width as usize, height as usize),
            Command::Line { x0, y0, x1, y1 } => self.line(x0, y0, x1, y1),
            Command::Palette {
                index,
                red,
                green,
                blue,
            } => self.set_palette(index, red, green, blue),
            Command::At { column, row } => {
                self.column = (column as usize).min(COLUMNS - 1);
                self.row = (row as usize).min(ROWS - 1);
            }
            Command::Present | Command::Quit | Command::Mode(_) => {}
        }
    }

    /// Whether anything has been drawn since [`Self::take_dirty`].
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Report whether anything was drawn, and reset the flag.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    /// The current drawing colour.
    pub fn color(&self) -> u8 {
        self.color
    }

    /// The text cursor, as a column and row in character cells.
    pub fn cursor(&self) -> (usize, usize) {
        (self.column, self.row)
    }

    /// The palette index at a pixel, or `None` off screen.
    pub fn pixel(&self, x: usize, y: usize) -> Option<u8> {
        (x < WIDTH && y < HEIGHT).then(|| self.pixels[y * WIDTH + x])
    }

    /// Resolve the screen into 32-bit `0x00RRGGBB` pixels for display.
    pub fn render(&self, into: &mut Vec<u32>) {
        into.clear();
        into.extend(
            self.pixels
                .iter()
                .map(|index| self.palette[*index as usize]),
        );
    }

    /// Fill the whole screen with the current colour and home the cursor.
    pub fn clear(&mut self) {
        self.pixels.fill(self.color);
        self.column = 0;
        self.row = 0;
        self.dirty = true;
    }

    fn set_palette(&mut self, index: u8, red: u8, green: u8, blue: u8) {
        self.palette[index as usize] = rgb(red, green, blue);
        // A palette change repaints every pixel using that entry.
        self.dirty = true;
    }

    fn plot(&mut self, x: usize, y: usize) {
        if x < WIDTH && y < HEIGHT {
            self.pixels[y * WIDTH + x] = self.color;
            self.dirty = true;
        }
    }

    fn rect(&mut self, x: usize, y: usize, width: usize, height: usize) {
        if width == 0 || height == 0 || x >= WIDTH || y >= HEIGHT {
            return;
        }

        // A rectangle that runs off an edge is clipped, not wrapped.
        let right = (x + width).min(WIDTH);
        let bottom = (y + height).min(HEIGHT);
        for row in y..bottom {
            self.pixels[row * WIDTH + x..row * WIDTH + right].fill(self.color);
        }
        self.dirty = true;
    }

    /// Bresenham between two points, both ends included.
    fn line(&mut self, x0: u8, y0: u8, x1: u8, y1: u8) {
        let (mut x, mut y) = (i32::from(x0), i32::from(y0));
        let (x1, y1) = (i32::from(x1), i32::from(y1));

        let step_x = if x1 >= x { 1 } else { -1 };
        let step_y = if y1 >= y { 1 } else { -1 };
        let run = (x1 - x).abs();
        let rise = -(y1 - y).abs();
        let mut error = run + rise;

        loop {
            self.plot(x as usize, y as usize);
            if x == x1 && y == y1 {
                return;
            }

            let doubled = error * 2;
            if doubled >= rise {
                error += rise;
                x += step_x;
            }
            if doubled <= run {
                error += run;
                y += step_y;
            }
        }
    }

    /// Draw one byte at the cursor, honouring the control characters a
    /// terminal would.
    fn write_text(&mut self, byte: u8) {
        match byte {
            b'\n' => self.newline(),
            b'\r' => self.column = 0,
            b'\t' => {
                let stop = (self.column / TAB_WIDTH + 1) * TAB_WIDTH;
                if stop >= COLUMNS {
                    self.newline();
                } else {
                    self.column = stop;
                }
            }
            b'\x08' => {
                if self.column > 0 {
                    self.column -= 1;
                    self.erase_cell(self.column, self.row);
                }
            }
            b'\x0C' => {
                // Form feed clears, which is what it does to a terminal.
                let color = self.color;
                self.color = DEFAULT_BACKGROUND;
                self.clear();
                self.color = color;
            }
            _ => {
                if self.column >= COLUMNS {
                    self.newline();
                }
                self.draw_glyph(byte);
                self.column += 1;
            }
        }
    }

    fn draw_glyph(&mut self, byte: u8) {
        let glyph = font::glyph(byte);
        let origin_x = self.column * CELL_WIDTH;
        let origin_y = self.row * CELL_HEIGHT;

        for (line, bits) in glyph.iter().enumerate() {
            for column in 0..font::WIDTH {
                // Bit 4 is the leftmost pixel.
                if bits & (1 << (font::WIDTH - 1 - column)) != 0 {
                    self.plot(origin_x + column, origin_y + line);
                }
            }
        }
    }

    fn erase_cell(&mut self, column: usize, row: usize) {
        let color = self.color;
        self.color = DEFAULT_BACKGROUND;
        self.rect(
            column * CELL_WIDTH,
            row * CELL_HEIGHT,
            CELL_WIDTH,
            CELL_HEIGHT,
        );
        self.color = color;
    }

    fn newline(&mut self) {
        self.column = 0;
        if self.row + 1 < ROWS {
            self.row += 1;
        } else {
            self.scroll();
        }
    }

    /// Move everything up one character row, clearing the row that opens up.
    fn scroll(&mut self) {
        let step = CELL_HEIGHT * WIDTH;
        self.pixels.copy_within(step.., 0);
        let tail = self.pixels.len() - step;
        self.pixels[tail..].fill(DEFAULT_BACKGROUND);
        self.dirty = true;
    }
}

/// Pack a colour the way the display buffer wants it.
fn rgb(red: u8, green: u8, blue: u8) -> u32 {
    u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue)
}

/// The palette a program gets without setting one up.
///
/// This is the usual 256-colour terminal layout: sixteen named colours, then a
/// 6x6x6 colour cube, then twenty-four greys. The cube is the useful part for
/// drawing, because an index is just arithmetic on the components —
/// `16 + 36 * red + 6 * green + blue` for components in `0..6` — which a
/// Brainfuck program can compute in bytes.
pub fn default_palette() -> [u32; 256] {
    let mut palette = [0; 256];

    // The sixteen named colours, in the order terminals put them.
    const BASE: [u32; 16] = [
        0x000000, 0xAA0000, 0x00AA00, 0xAA5500, 0x0000AA, 0xAA00AA, 0x00AAAA, 0xAAAAAA, 0x555555,
        0xFF5555, 0x55FF55, 0xFFFF55, 0x5555FF, 0xFF55FF, 0x55FFFF, 0xFFFFFF,
    ];
    palette[..16].copy_from_slice(&BASE);

    // The colour cube. 0 and then 95..255 in steps of 40 is the standard ramp.
    const RAMP: [u8; 6] = [0, 95, 135, 175, 215, 255];
    for red in 0..6 {
        for green in 0..6 {
            for blue in 0..6 {
                palette[16 + 36 * red + 6 * green + blue] = rgb(RAMP[red], RAMP[green], RAMP[blue]);
            }
        }
    }

    // The greys, avoiding both ends because the cube already has them.
    for step in 0..24 {
        let level = 8 + 10 * step as u8;
        palette[232 + step] = rgb(level, level, level);
    }

    palette
}

/// Return the colour-cube index for components in `0..6`.
///
/// This is the formula the palette documentation describes, for callers that
/// would rather not repeat it.
pub fn cube_color(red: u8, green: u8, blue: u8) -> u8 {
    16 + 36 * red.min(5) + 6 * green.min(5) + blue.min(5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{Decoder, InputMode};

    /// Run a byte stream through the decoder and onto a screen.
    fn screen_after(bytes: &[u8]) -> Screen {
        let mut screen = Screen::new();
        let mut decoder = Decoder::new();
        for byte in bytes {
            if let Some(command) = decoder.push(*byte) {
                screen.apply(command);
            }
        }
        screen
    }

    #[test]
    fn the_screen_is_addressable_by_single_bytes() {
        // The whole point of 256x192: every coordinate fits in a byte.
        const { assert!(WIDTH == 256) };
        const { assert!(HEIGHT <= 256) };
    }

    #[test]
    fn starts_cleared_to_the_background() {
        let screen = Screen::new();

        assert_eq!(screen.pixel(0, 0), Some(DEFAULT_BACKGROUND));
        assert_eq!(
            screen.pixel(WIDTH - 1, HEIGHT - 1),
            Some(DEFAULT_BACKGROUND)
        );
        assert_eq!(screen.pixel(WIDTH, 0), None);
        assert_eq!(screen.pixel(0, HEIGHT), None);
    }

    #[test]
    fn plots_a_pixel_in_the_current_colour() {
        let mut screen = Screen::new();
        screen.apply(Command::Color(42));
        screen.apply(Command::Plot { x: 10, y: 20 });

        assert_eq!(screen.pixel(10, 20), Some(42));
        assert_eq!(screen.pixel(11, 20), Some(DEFAULT_BACKGROUND));
    }

    #[test]
    fn ignores_pixels_off_the_bottom_of_the_screen() {
        let mut screen = Screen::new();
        // y = 200 is past the 192-row screen; nothing should wrap around.
        screen.apply(Command::Color(7));
        screen.apply(Command::Plot { x: 0, y: 200 });

        assert!(
            (0..WIDTH * HEIGHT).all(|index| screen.pixels[index] == DEFAULT_BACKGROUND),
            "an off-screen plot touched the framebuffer"
        );
    }

    #[test]
    fn fills_and_clips_rectangles() {
        let mut screen = Screen::new();
        screen.apply(Command::Color(3));
        screen.apply(Command::Rect {
            x: 250,
            y: 188,
            width: 20,
            height: 20,
        });

        assert_eq!(screen.pixel(255, 191), Some(3));
        assert_eq!(screen.pixel(249, 191), Some(DEFAULT_BACKGROUND));
    }

    #[test]
    fn an_empty_rectangle_draws_nothing() {
        let mut screen = Screen::new();
        screen.apply(Command::Color(3));
        screen.apply(Command::Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 5,
        });

        assert_eq!(screen.pixel(0, 0), Some(DEFAULT_BACKGROUND));
    }

    #[test]
    fn draws_lines_including_both_ends() {
        let mut screen = Screen::new();
        screen.apply(Command::Color(9));
        screen.apply(Command::Line {
            x0: 0,
            y0: 0,
            x1: 10,
            y1: 10,
        });

        assert_eq!(screen.pixel(0, 0), Some(9));
        assert_eq!(screen.pixel(5, 5), Some(9));
        assert_eq!(screen.pixel(10, 10), Some(9));
    }

    #[test]
    fn draws_lines_in_every_direction() {
        for (x0, y0, x1, y1) in [(0, 0, 20, 5), (20, 5, 0, 0), (5, 0, 5, 20), (0, 5, 20, 5)] {
            let mut screen = Screen::new();
            screen.apply(Command::Color(9));
            screen.apply(Command::Line { x0, y0, x1, y1 });

            assert_eq!(screen.pixel(x0 as usize, y0 as usize), Some(9));
            assert_eq!(screen.pixel(x1 as usize, y1 as usize), Some(9));
        }
    }

    #[test]
    fn clearing_uses_the_current_colour() {
        let mut screen = Screen::new();
        screen.apply(Command::Color(5));
        screen.apply(Command::Clear);

        assert_eq!(screen.pixel(0, 0), Some(5));
        assert_eq!(screen.pixel(WIDTH - 1, HEIGHT - 1), Some(5));
        assert_eq!(screen.cursor(), (0, 0));
    }

    #[test]
    fn plain_text_advances_the_cursor() {
        let screen = screen_after(b"Hi");

        assert_eq!(screen.cursor(), (2, 0));
        // Something was drawn in the first two cells.
        assert!((0..CELL_WIDTH * 2).any(|x| screen.pixel(x, 1) != Some(DEFAULT_BACKGROUND)));
    }

    #[test]
    fn a_newline_returns_to_the_left_and_moves_down() {
        let screen = screen_after(b"a\nb");

        assert_eq!(screen.cursor(), (1, 1));
    }

    #[test]
    fn text_wraps_at_the_right_edge() {
        let line = vec![b'x'; COLUMNS + 1];
        let screen = screen_after(&line);

        assert_eq!(screen.cursor(), (1, 1));
    }

    #[test]
    fn the_bottom_row_scrolls_instead_of_overflowing() {
        let mut stream = vec![b'\n'; ROWS - 1];
        stream.push(b'A');
        let before = screen_after(&stream);
        assert_eq!(before.cursor(), (1, ROWS - 1));

        // One more newline has nowhere to go, so the screen moves up.
        let mut stream = vec![b'\n'; ROWS];
        stream.push(b'B');
        let after = screen_after(&stream);
        assert_eq!(after.cursor(), (1, ROWS - 1));
    }

    #[test]
    fn scrolling_moves_pixels_up_by_one_cell() {
        let mut screen = Screen::new();
        screen.apply(Command::Color(6));
        // Mark a pixel one cell down, then scroll it to the top.
        screen.apply(Command::Plot {
            x: 3,
            y: CELL_HEIGHT as u8,
        });
        screen.scroll();

        assert_eq!(screen.pixel(3, 0), Some(6));
        assert_eq!(screen.pixel(3, CELL_HEIGHT), Some(DEFAULT_BACKGROUND));
    }

    #[test]
    fn a_tab_moves_to_the_next_stop() {
        assert_eq!(screen_after(b"a\tb").cursor(), (TAB_WIDTH + 1, 0));
    }

    #[test]
    fn a_backspace_erases_the_character_before_it() {
        let screen = screen_after(b"ab\x08");

        assert_eq!(screen.cursor(), (1, 0));
        // The erased cell is background again.
        for x in CELL_WIDTH..CELL_WIDTH * 2 {
            for y in 0..CELL_HEIGHT {
                assert_eq!(screen.pixel(x, y), Some(DEFAULT_BACKGROUND));
            }
        }
    }

    #[test]
    fn a_backspace_at_the_left_edge_does_nothing() {
        assert_eq!(screen_after(b"\x08").cursor(), (0, 0));
    }

    #[test]
    fn a_form_feed_clears_to_the_background_whatever_the_colour() {
        let mut screen = screen_after(b"hello");
        screen.apply(Command::Color(200));
        screen.apply(Command::Text(b'\x0C'));

        assert_eq!(screen.pixel(0, 0), Some(DEFAULT_BACKGROUND));
        assert_eq!(screen.cursor(), (0, 0));
        // The colour survives the clear.
        assert_eq!(screen.color(), 200);
    }

    #[test]
    fn the_cursor_can_be_placed_and_is_clamped_on_screen() {
        let screen = screen_after(&[crate::protocol::DLE, crate::protocol::command::AT, 250, 250]);

        assert_eq!(screen.cursor(), (COLUMNS - 1, ROWS - 1));
    }

    #[test]
    fn a_byte_with_no_glyph_still_draws_something() {
        let screen = screen_after(&[0x01]);

        assert_eq!(screen.cursor(), (1, 0));
        assert!(
            (0..CELL_WIDTH).any(|x| (0..CELL_HEIGHT).any(|y| screen.pixel(x, y) != Some(0))),
            "an unprintable byte drew nothing"
        );
    }

    #[test]
    fn drawing_marks_the_screen_dirty_once() {
        let mut screen = Screen::new();
        assert!(screen.take_dirty());
        assert!(!screen.is_dirty());

        screen.apply(Command::Plot { x: 1, y: 1 });
        assert!(screen.take_dirty());
        assert!(!screen.take_dirty());
    }

    #[test]
    fn present_and_quit_do_not_touch_the_framebuffer() {
        let mut screen = Screen::new();
        screen.take_dirty();
        screen.apply(Command::Present);
        screen.apply(Command::Quit);
        screen.apply(Command::Mode(InputMode::Events));

        assert!(!screen.is_dirty());
    }

    #[test]
    fn renders_through_the_palette() {
        let mut screen = Screen::new();
        screen.apply(Command::Palette {
            index: 1,
            red: 0x12,
            green: 0x34,
            blue: 0x56,
        });
        screen.apply(Command::Color(1));
        screen.apply(Command::Plot { x: 0, y: 0 });

        let mut buffer = Vec::new();
        screen.render(&mut buffer);

        assert_eq!(buffer.len(), WIDTH * HEIGHT);
        assert_eq!(buffer[0], 0x123456);
    }

    #[test]
    fn the_default_palette_follows_the_documented_layout() {
        let palette = default_palette();

        assert_eq!(palette[0], 0x000000, "index 0 is black");
        assert_eq!(palette[15], 0xFFFFFF, "index 15 is white");
        // The cube's corners.
        assert_eq!(palette[cube_color(0, 0, 0) as usize], 0x000000);
        assert_eq!(palette[cube_color(5, 5, 5) as usize], 0xFFFFFF);
        assert_eq!(palette[cube_color(5, 0, 0) as usize], 0xFF0000);
        assert_eq!(palette[cube_color(0, 5, 0) as usize], 0x00FF00);
        assert_eq!(palette[cube_color(0, 0, 5) as usize], 0x0000FF);
    }

    #[test]
    fn the_cube_formula_stays_inside_the_cube() {
        assert_eq!(cube_color(0, 0, 0), 16);
        assert_eq!(cube_color(5, 5, 5), 231);
        // Out-of-range components are clamped rather than running into the greys.
        assert_eq!(cube_color(9, 9, 9), 231);
    }
}
