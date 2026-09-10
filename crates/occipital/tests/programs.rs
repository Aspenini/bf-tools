//! The example programs, run headless and checked by their pixels.
//!
//! Each of these compiles a `.cra` file to Brainfuck, compiles that to machine
//! code, and runs it against a framebuffer with no window, so the whole path a
//! real program takes is exercised on a machine with no display.

use occipital::protocol::{event, key};
use occipital::screen::{self, Screen, cube_color};
use occipital::{Headless, Program};

/// Compile and run an example, returning the host it drew into.
fn run(example: &str, input: &[u8]) -> Headless {
    let path = format!("examples/{example}");
    let program = Program::load(&path).unwrap_or_else(|error| panic!("{path}: {error}"));

    Headless::run(program.ops(), input).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// Render the screen as text, for when an assertion fails and the pixels need
/// looking at.
fn as_ascii(screen: &Screen) -> String {
    const SHADES: &[u8] = b" .:-=+*#%@";
    let palette = screen::default_palette();

    let mut out = String::new();
    // Sample rather than scale: one character per 4x8 block.
    for row in (0..screen::HEIGHT).step_by(8) {
        for column in (0..screen::WIDTH).step_by(4) {
            let index = screen.pixel(column, row).unwrap_or(0);
            let color = palette[index as usize];
            let luminance =
                ((color >> 16) & 0xFF) * 30 + ((color >> 8) & 0xFF) * 59 + (color & 0xFF) * 11;
            let shade = (luminance / 100 * (SHADES.len() as u32 - 1) / 255) as usize;
            out.push(SHADES[shade.min(SHADES.len() - 1)] as char);
        }
        out.push('\n');
    }
    out
}

#[test]
fn bars_draws_the_whole_colour_cube() {
    let host = run("bars.cra", b"");
    let screen = host.screen();

    println!("{}", as_ascii(screen));

    // The bands run darkest red at the top to brightest at the bottom, and
    // each band steps green then blue across the screen.
    assert_eq!(
        screen.pixel(0, 0),
        Some(cube_color(0, 0, 0)),
        "top left should be the cube's black corner"
    );
    assert_eq!(
        screen.pixel(1, 160),
        Some(cube_color(5, 0, 0)),
        "the last band should start at full red"
    );

    // Every one of the 216 cells should be its own colour.
    let mut seen = std::collections::BTreeSet::new();
    for red in 0..6u8 {
        for green in 0..6u8 {
            for blue in 0..6u8 {
                // Sample near the top of each band, which keeps clear of the
                // label the program writes across row 22.
                let x = usize::from(green * 6 + blue) * 7 + 3;
                let y = usize::from(red) * 32 + 4;
                let found = screen.pixel(x, y).expect("sample is on screen");
                assert_eq!(
                    found,
                    cube_color(red, green, blue),
                    "cell ({red}, {green}, {blue}) at ({x}, {y})"
                );
                seen.insert(found);
            }
        }
    }
    assert_eq!(seen.len(), 216, "some colours were drawn twice");
}

#[test]
fn bars_labels_itself_in_text_over_the_bands() {
    let host = run("bars.cra", b"");

    // The label is written after the bands, in white, on row 22.
    let row = 22 * screen::CELL_HEIGHT;
    let has_white = (0..screen::WIDTH)
        .any(|x| (row..row + screen::CELL_HEIGHT).any(|y| host.screen().pixel(x, y) == Some(15)));

    assert!(has_white, "the label did not reach the framebuffer");
}

#[test]
fn bars_presents_its_finished_frame() {
    let host = run("bars.cra", b"");

    assert_eq!(host.presents(), 1);
    assert!(
        !host.is_mid_command(),
        "the program stopped midway through a command"
    );
}

#[test]
fn a_program_written_for_a_terminal_still_draws() {
    // fizzbuzz.cra was written before any of this existed and knows nothing
    // about the protocol. Plain bytes being text is what makes it work.
    let program = Program::load("../cranium/examples/fizzbuzz.cra")
        .expect("cranium's fizzbuzz should compile");
    let host = Headless::run(program.ops(), b"").expect("fizzbuzz should run");

    println!("{}", as_ascii(host.screen()));

    // It prints a hundred lines into a 24-row screen, so it must have
    // scrolled all the way down. Its last line ends in a newline, which is
    // why the bottom row itself is blank and the text sits just above it.
    assert_eq!(
        host.screen().cursor(),
        (0, screen::ROWS - 1),
        "the screen did not scroll to the bottom"
    );

    let last_line = (screen::ROWS - 2) * screen::CELL_HEIGHT;
    let has_text = (0..screen::WIDTH).any(|x| {
        (last_line..last_line + screen::CELL_HEIGHT).any(|y| host.screen().pixel(x, y) == Some(15))
    });
    assert!(has_text, "the last line printed left no pixels");

    assert_eq!(
        host.presents(),
        0,
        "it never asks for a frame, and need not"
    );
}

#[test]
fn bounce_moves_the_ball_when_told_to() {
    // Steer left, then let the script run out, which reports the window
    // closing and ends the loop.
    let host = run(
        "bounce.cra",
        &[event::KEY_DOWN, key::LEFT, event::NONE, event::NONE],
    );

    println!("{}", as_ascii(host.screen()));

    assert!(host.presents() > 1, "the ball never animated");
    assert!(
        host.mode() == occipital::InputMode::Events,
        "an interactive program should be polling"
    );
}

#[test]
fn bounce_stops_when_the_window_closes() {
    let host = run("bounce.cra", &[event::QUIT]);

    // Reaching here at all means the program returned rather than spinning.
    assert!(
        !host.is_mid_command(),
        "the program stopped midway through a command"
    );
}

#[test]
fn bounce_quits_on_the_q_key() {
    let host = run("bounce.cra", &[event::KEY_DOWN, b'q']);

    assert!(
        host.has_quit(),
        "pressing q should close the window from the program's side"
    );
}
