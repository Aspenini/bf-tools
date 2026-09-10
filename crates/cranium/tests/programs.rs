//! End-to-end tests: compile Cranium, run the Brainfuck, check the output.

use cranium::{compile_str, module};
use lobe::{CellSize, create_runtime_with_tape};

/// Compile `source`, run it on `input`, and return what it printed.
fn run_with(source: &str, input: &[u8]) -> String {
    let compiled = match compile_str(source) {
        Ok(output) => output,
        Err(err) => panic!("failed to compile:\n{source}\n\n{err}"),
    };
    let tape = compiled.cells_used.max(30_000);
    let mut runtime = create_runtime_with_tape(&compiled.code, CellSize::Bits8, tape)
        .expect("cranium emits valid brainfuck");
    let mut input = std::io::Cursor::new(input.to_vec());
    let mut output = Vec::new();
    runtime
        .run_with_io(&mut input, &mut output)
        .expect("program runs");
    String::from_utf8(output).expect("output is text")
}

fn run(source: &str) -> String {
    run_with(source, &[])
}

/// Compile once and run the result against several inputs.
///
/// Worth it for the larger examples, where compiling dominates.
fn compiled_runner(source: &str) -> impl Fn(&[u8]) -> String + use<> {
    let compiled = match compile_str(source) {
        Ok(output) => output,
        Err(err) => panic!("failed to compile:\n{err}"),
    };
    let tape = compiled.cells_used.max(30_000);
    move |bytes: &[u8]| {
        let mut runtime = create_runtime_with_tape(&compiled.code, CellSize::Bits8, tape)
            .expect("cranium emits valid brainfuck");
        let mut input = std::io::Cursor::new(bytes.to_vec());
        let mut output = Vec::new();
        runtime
            .run_with_io(&mut input, &mut output)
            .expect("program runs");
        String::from_utf8_lossy(&output).into_owned()
    }
}

/// Drop the SGR sequences so a screen can be compared as plain text.
fn without_colour(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('\u{1b}') {
        out.push_str(&rest[..start]);
        match rest[start..].find('m') {
            Some(end) => rest = &rest[start + end + 1..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Wrap `body` in a `main` so the tests stay readable.
fn main_of(body: &str) -> String {
    format!("fn main() {{\n{body}\n}}\n")
}

fn error_of(source: &str) -> String {
    compile_str(source)
        .err()
        .unwrap_or_else(|| panic!("expected a compile error for:\n{source}"))
        .to_string()
}

#[test]
fn prints_text_and_numbers() {
    assert_eq!(
        run(&main_of(
            r#"print("hi "); print(7); putc(' '); print(1234);"#
        )),
        "hi 7 1234"
    );
}

/// `print` always writes a number; `putc` is how a value becomes a character.
#[test]
fn print_writes_decimals_and_putc_writes_bytes() {
    assert_eq!(run(&main_of("print('A'); putc('A');")), "65A");
}

#[test]
fn byte_arithmetic_wraps_and_int_does_not() {
    assert_eq!(
        run(&main_of(
            "let a = 200; let b = 100; print(a + b); print(\" \"); print(a as int + b as int);"
        )),
        "44 300"
    );
}

#[test]
fn evaluates_every_arithmetic_operator() {
    assert_eq!(
        run(&main_of(
            r#"
            let a: int = 1000;
            let b: int = 7;
            print(a + b); print(" ");
            print(a - b); print(" ");
            print(a * b); print(" ");
            print(a / b); print(" ");
            print(a % b);
            "#
        )),
        "1007 993 7000 142 6"
    );
}

#[test]
fn compares_across_widths() {
    assert_eq!(
        run(&main_of(
            r#"
            let small = 200;
            let big: int = 300;
            print(small < big); print(small > big);
            print(small == 200); print(big != 300);
            print(small <= 200); print(big >= 301);
            "#
        )),
        "101010"
    );
}

#[test]
fn applies_bitwise_and_shift_operators() {
    assert_eq!(
        run(&main_of(
            r#"
            print(0b1100 & 0b1010); print(" ");
            print(0b1100 | 0b1010); print(" ");
            print(0b1100 ^ 0b1010); print(" ");
            print(5 << 2); print(" ");
            print(200 >> 3);
            "#
        )),
        "8 14 6 20 25"
    );
}

#[test]
fn shifts_by_a_runtime_amount() {
    assert_eq!(
        run(&main_of(
            "let n = 3; print(1 << n); print(\" \"); print(96 >> n);"
        )),
        "8 12"
    );
}

#[test]
fn runs_structured_control_flow() {
    assert_eq!(
        run(&main_of(
            r#"
            let total = 0;
            for i in 1..6 { total += i; }
            print(total);
            print(" ");
            let n = 0;
            while n < 3 { print(n); n += 1; }
            print(" ");
            let k = 0;
            loop {
                k += 1;
                if k == 2 { continue; }
                if k > 4 { break; }
                print(k);
            }
            "#
        )),
        "15 012 134"
    );
}

#[test]
fn breaks_out_of_nested_loops_only_once() {
    assert_eq!(
        run(&main_of(
            r#"
            for outer in 0..3 {
                for inner in 0..3 {
                    if inner == 1 { break; }
                    print(outer); print(inner); print(" ");
                }
            }
            print("done");
            "#
        )),
        "00 10 20 done"
    );
}

#[test]
fn if_else_chains_pick_one_branch() {
    assert_eq!(
        run(&main_of(
            r#"
            for n in 1..16 {
                if n % 15 == 0 { print("FB"); }
                else if n % 3 == 0 { print("F"); }
                else if n % 5 == 0 { print("B"); }
                else { print(n); }
                print(" ");
            }
            "#
        )),
        "1 2 F 4 B F 7 8 F B 11 F 13 14 FB "
    );
}

#[test]
fn calls_functions_and_returns_early() {
    let source = r#"
fn classify(n: byte) -> byte {
    if n == 0 { return 'z'; }
    if n < 10 { return 's'; }
    return 'b';
}

fn main() {
    print(classify(0) as int);
    putc(classify(0)); putc(classify(5)); putc(classify(200));
}
"#;
    assert_eq!(run(source), "122zsb");
}

#[test]
fn returns_from_inside_nested_loops() {
    let source = r#"
fn first_pair(limit: byte) -> int {
    for a in 1..10 {
        for b in 1..10 {
            if a * b == limit {
                return (a * 10 + b) as int;
            }
        }
    }
    return 0;
}

fn main() {
    print(first_pair(12)); print(" "); print(first_pair(97));
}
"#;
    assert_eq!(run(source), "26 0");
}

#[test]
fn short_circuits_both_directions() {
    let source = r#"
fn shout(tag: byte) -> bool {
    putc(tag);
    return true;
}

fn main() {
    if false && shout('a') { }
    if true || shout('b') { }
    if true && shout('c') { }
    if false || shout('d') { }
    print("|");
}
"#;
    assert_eq!(run(source), "cd|");
}

#[test]
fn indexes_arrays_by_constant_and_by_variable() {
    assert_eq!(
        run(&main_of(
            r#"
            let table = [3, 1, 4, 1, 5, 9, 2, 6];
            print(table[0]); print(table[7]);
            print(" ");
            for i in 0..8 { print(table[i]); }
            print(" ");
            let k = 5;
            print(table[k]); print(table[k - 2]);
            "#
        )),
        "36 31415926 91"
    );
}

#[test]
fn writes_array_elements_at_runtime_indices() {
    assert_eq!(
        run(&main_of(
            r#"
            let squares: byte[16];
            for i in 0..16 { squares[i] = i * i; }
            for i in 0..16 { print(squares[i]); print(" "); }
            "#
        )),
        "0 1 4 9 16 25 36 49 64 81 100 121 144 169 196 225 "
    );
}

#[test]
fn handles_arrays_larger_than_one_index_cell() {
    assert_eq!(
        run(&main_of(
            r#"
            let big: byte[400];
            big[300] = 42;
            let where: int = 300;
            big[where - 1] = 7;
            print(big[where]); print(" "); print(big[299]); print(" "); print(big[0]);
            "#
        )),
        "42 7 0"
    );
}

#[test]
fn stores_wide_values_in_arrays() {
    assert_eq!(
        run(&main_of(
            r#"
            let totals: int[8];
            for i in 0..8 { totals[i] = i as int * 1000; }
            let k = 6;
            print(totals[k]); print(" "); print(totals[7]);
            "#
        )),
        "6000 7000"
    );
}

#[test]
fn treats_strings_as_byte_arrays() {
    assert_eq!(
        run(&main_of(
            r#"
            let greeting: byte[16] = "hello";
            puts(greeting);
            print("|");
            print(greeting);
            print("|");
            print(len(greeting));
            print("|");
            greeting[0] = 'H';
            puts(greeting);
            "#
        )),
        "hello|hello|16|Hello"
    );
}

#[test]
fn passes_arrays_to_functions_by_reference() {
    let source = r#"
fn fill(target: byte[6], value: byte) {
    for i in 0..6 { target[i] = value; }
}

fn total(source: byte[6]) -> int {
    let sum: int = 0;
    for i in 0..6 { sum += source[i] as int; }
    return sum;
}

fn main() {
    let cells: byte[6];
    fill(cells, 50);
    print(total(cells));
    print(" ");
    print(cells[3]);
}
"#;
    assert_eq!(run(source), "300 50");
}

#[test]
fn uses_globals_and_constants() {
    let source = r#"
const STEP = 5;
let counter: int = 100;

fn bump() {
    counter += STEP as int;
}

fn main() {
    bump(); bump(); bump();
    print(counter);
}
"#;
    assert_eq!(run(source), "115");
}

#[test]
fn reads_standard_input() {
    let source = r#"
fn main() {
    let count = 0;
    loop {
        let c = getc();
        if c == 0 { break; }
        if c >= 'a' && c <= 'z' { c = c - 32; }
        putc(c);
        count += 1;
    }
    print("|");
    print(count);
}
"#;
    assert_eq!(run_with(source, b"abc XYZ"), "ABC XYZ|7");
}

#[test]
fn reads_zero_at_end_of_input() {
    assert_eq!(run(&main_of("print(getc());")), "0");
}

#[test]
fn casts_between_scalar_types() {
    assert_eq!(
        run(&main_of(
            r#"
            let wide: int = 300;
            print(wide as byte); print(" ");
            print(200 as int * 2); print(" ");
            print(0 as bool); print(7 as bool);
            "#
        )),
        "44 400 01"
    );
}

#[test]
fn negates_and_inverts() {
    // `-1` is a signed value, not a byte that wrapped to 255.
    assert_eq!(
        run(&main_of("print(-1); print(\" \"); print(!0); print(!5);")),
        "-1 10"
    );
}

#[test]
fn signed_arithmetic_goes_below_zero() {
    assert_eq!(
        run(&main_of(
            r#"
            let a: sint = 4;
            let b: sint = 10;
            print(a - b); print(" ");
            print(a + b); print(" ");
            print(a * -3); print(" ");
            let small: sbyte = -5;
            print(small); print(" ");
            print(-small);
            "#
        )),
        "-6 14 -12 -5 5"
    );
}

/// Division truncates toward zero and the remainder follows the dividend, so
/// `q * d + r` comes back to the dividend in every sign combination.
#[test]
fn signed_division_truncates_toward_zero() {
    assert_eq!(
        run(&main_of(
            r#"
            print(-7 / 2); print(" "); print(-7 % 2); print(" ");
            print(7 / -2); print(" "); print(7 % -2); print(" ");
            print(-7 / -2); print(" "); print(-7 % -2); print(" ");
            print(7 / 2); print(" "); print(7 % 2);
            "#
        )),
        "-3 -1 -3 1 3 -1 3 1"
    );

    assert_eq!(
        run(&main_of(
            "let a: sint = -1000; let b: sint = 7; print(a / b); print(\" \"); print(a % b);"
        )),
        "-142 -6"
    );
}

#[test]
fn signed_values_compare_by_sign_first() {
    assert_eq!(
        run(&main_of(
            r#"
            print(-5 < 3); print(3 < -5); print(-5 < -3); print(-3 < -5);
            print(-5 == -5); print(-5 <= -5); print(0 > -1); print(-1 >= 0);
            "#
        )),
        "10101110"
    );
}

#[test]
fn signed_values_reach_their_extremes() {
    assert_eq!(
        run(&main_of(
            r#"
            let lo: sint = -32768;
            let hi: sint = 32767;
            print(lo); print(" "); print(hi); print(" ");
            let blo: sbyte = -128;
            let bhi: sbyte = 127;
            print(blo); print(" "); print(bhi); print(" "); print(0 - 0);
            "#
        )),
        "-32768 32767 -128 127 0"
    );
}

#[test]
fn widening_a_signed_value_keeps_its_sign() {
    assert_eq!(
        run(&main_of(
            r#"
            let small: sbyte = -5;
            let wide: sint = small;
            print(wide); print(" ");
            // A byte and an sbyte have no common byte-wide type, so they meet
            // at sint rather than silently wrapping.
            let count: byte = 200;
            print(count + small);
            "#
        )),
        "-5 195"
    );
}

#[test]
fn shifting_a_signed_value_right_keeps_its_sign() {
    assert_eq!(
        run(&main_of(
            r#"
            print(-8 >> 1); print(" ");
            print(-8 >> 2); print(" ");
            print(-1 >> 4); print(" ");
            let n = 2;
            print(-64 >> n); print(" ");
            print(-3 << 2);
            "#
        )),
        "-4 -2 -1 -16 -12"
    );
}

#[test]
fn signed_values_live_in_arrays_and_loops() {
    assert_eq!(
        run(&main_of(
            r#"
            let deltas: sint[6];
            for i in 0..6 {
                deltas[i] = i as sint - 3;
            }
            for i in 0..6 { print(deltas[i]); print(" "); }
            let total: sint = 0;
            for i in 0..6 { total += deltas[i]; }
            print("sum "); print(total);
            "#
        )),
        "-3 -2 -1 0 1 2 sum -3"
    );
}

#[test]
fn casts_convert_between_signed_and_unsigned() {
    assert_eq!(
        run(&main_of(
            r#"
            let negative: sbyte = -1;
            print(negative as byte); print(" ");
            print(negative as sint); print(" ");
            let big: byte = 200;
            print(big as sbyte); print(" ");
            print(-5 as bool);
            "#
        )),
        "255 -1 -56 1"
    );
}

#[test]
fn mixing_int_and_sint_needs_a_cast() {
    let message = error_of(&main_of("let u: int = 5; let s: sint = -5; print(u + s);"));
    assert!(message.contains("no common type"), "{message}");
    assert!(message.contains("as"), "{message}");

    let message = error_of(&main_of("let b: sbyte = 200;"));
    assert!(message.contains("200 does not fit in `sbyte`"), "{message}");

    let message = error_of(&main_of("let s: sbyte = -1; let b: byte = s;"));
    assert!(message.contains("as byte"), "{message}");
}

#[test]
fn scopes_shadow_and_restore() {
    assert_eq!(
        run(&main_of(
            r#"
            let x = 1;
            {
                let x = 2;
                print(x);
            }
            print(x);
            "#
        )),
        "21"
    );
}

/// Arrays are laid out in declaration order, which is what lets a programmer
/// put the array a hot loop leans on nearest to the working set.
#[test]
fn lays_arrays_out_in_declaration_order() {
    let compiled = compile_str(
        "let first: byte[8];
let second: byte[8];
fn main() { first[0] = 1; second[0] = 2; }
",
    )
    .expect("valid program");
    let names: Vec<&str> = compiled
        .arrays
        .iter()
        .map(|array| array.name.as_str())
        .collect();
    assert_eq!(names, ["first", "second"]);
    assert!(compiled.arrays[0].base < compiled.arrays[1].base);
    // Scalars and temporaries sit below every array, never between two of them.
    assert!(compiled.arrays[0].base > 0);
}

#[test]
fn emits_only_brainfuck_commands() {
    let compiled = compile_str(&main_of("print(42);")).expect("valid program");
    assert!(
        compiled
            .code
            .bytes()
            .all(|byte| b"+-<>.,[]".contains(&byte))
    );
    let depth = compiled.code.bytes().fold(0_i64, |depth, byte| match byte {
        b'[' => depth + 1,
        b']' => depth - 1,
        _ => depth,
    });
    assert_eq!(depth, 0, "brackets must balance");
}

#[test]
fn reports_recursion_instead_of_looping_forever() {
    let message = error_of(
        r#"
fn fact(n: byte) -> byte {
    if n == 0 { return 1; }
    return n * fact(n - 1);
}
fn main() { print(fact(5)); }
"#,
    );
    assert!(message.contains("calls itself"), "{message}");
    assert!(message.contains("recursion"), "{message}");
}

#[test]
fn reports_indirect_recursion() {
    let message = error_of(
        r#"
fn ping(n: byte) -> byte { return pong(n); }
fn pong(n: byte) -> byte { return ping(n); }
fn main() { print(ping(1)); }
"#,
    );
    assert!(message.contains("calls itself"), "{message}");
}

#[test]
fn reports_useful_type_errors() {
    let message = error_of(&main_of("let wide: int = 300; let narrow: byte = wide;"));
    assert!(message.contains("as byte"), "{message}");

    let message = error_of(&main_of("let flag: bool = 3;"));
    assert!(message.contains("!= 0"), "{message}");
}

#[test]
fn reports_missing_names_and_bad_indices() {
    assert!(error_of(&main_of("print(nope);")).contains("not defined"));
    assert!(error_of(&main_of("let a: byte[4]; a[9] = 1;")).contains("outside"));
    assert!(error_of(&main_of("break;")).contains("outside a loop"));
    assert!(error_of("fn other() {}").contains("no `main`"));
}

#[test]
fn reports_where_the_problem_is() {
    let message = error_of("fn main() {\n    let x = 1;\n    x = nope;\n}\n");
    assert!(message.starts_with("<source>:3:"), "{message}");
}

#[test]
fn emulates_a_terminal() {
    let screen = compiled_runner(include_str!("../examples/terminal.cra"));
    let row = |text: &str, index: usize| -> String {
        without_colour(text)
            .lines()
            .nth(index)
            .unwrap_or_default()
            .to_string()
    };

    // Printable text lands at the cursor, inside a drawn frame.
    let plain = screen(b"hello");
    assert_eq!(row(&plain, 0), "+--------------------------------+");
    assert_eq!(row(&plain, 1), "|hello                           |");
    assert!(plain.contains("cursor: row 1, col 6"), "{plain}");

    // CSI H places the cursor absolutely, counting from one.
    assert_eq!(
        row(&screen(b"\x1b[3;5Hhi"), 3),
        "|    hi                          |"
    );

    // Backspace moves without erasing; overwriting is what erases.
    assert_eq!(
        row(&screen(b"back\x08\x08\x08BACK"), 1),
        "|bBACK                           |"
    );

    // A tab advances to the next multiple of eight.
    assert_eq!(
        row(&screen(b"ab\tc"), 1),
        "|ab      c                       |"
    );

    // Erase to end of line, and erase the whole display.
    assert_eq!(
        row(&screen(b"ABCDEFGHIJ\x1b[1;5H\x1b[K"), 1),
        "|ABCD                            |"
    );
    assert_eq!(
        row(&screen(b"junk\x1b[2J\x1b[1;1Hclean"), 1),
        "|clean                           |"
    );

    // The cursor can be saved and restored around a jump.
    assert_eq!(
        row(&screen(b"A\x1b[s\x1b[5;20HB\x1b[uC"), 1),
        "|AC                              |"
    );

    // Writing past the last column wraps onto the next row.
    let wrapped = screen(&[b'x'; 35]);
    assert_eq!(row(&wrapped, 1), "|xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx|");
    assert_eq!(row(&wrapped, 2), "|xxx                             |");

    // A ninth line scrolls the first one off the top.
    let scrolled = screen(b"1\r\n2\r\n3\r\n4\r\n5\r\n6\r\n7\r\n8\r\n9");
    assert_eq!(row(&scrolled, 1), "|2                               |");
    assert_eq!(row(&scrolled, 8), "|9                               |");

    // SGR is tracked per cell and re-emitted only where the colour changes.
    let coloured = screen(b"ab\x1b[1;31mcd\x1b[0mef");
    assert!(
        coloured.contains("\u{1b}[0;37mab\u{1b}[1;31mcd\u{1b}[0;37mef"),
        "{coloured}"
    );
    assert_eq!(row(&coloured, 1), "|abcdef                          |");
}

#[test]
fn compiles_and_runs_the_examples() {
    let cases: &[(&str, &str, &str)] = &[
        (include_str!("../examples/hello.cra"), "", "Hello, world!\n"),
        (
            include_str!("../examples/calc.cra"),
            "2 + 3 * 4 - 10 / 5",
            "12\n",
        ),
    ];

    for (source, input, expected) in cases {
        assert_eq!(run_with(source, input.as_bytes()), *expected);
    }

    let fizzbuzz = run(include_str!("../examples/fizzbuzz.cra"));
    assert!(fizzbuzz.starts_with("1\n2\nFizz\n4\nBuzz\n"), "{fizzbuzz}");
    assert!(fizzbuzz.ends_with("Buzz\n"));
    assert_eq!(fizzbuzz.lines().count(), 100);

    // The adventure: walk the intended route and check it can be finished.
    let adventure = run_with(
        include_str!("../examples/adventure.cra"),
        b"take key
e
n
take lamp
e
take sword
w
s
e
take rope
n
n
open
",
    );
    assert!(adventure.contains("-- The Cell --"), "{adventure}");
    assert!(
        adventure.contains("You take the brass lamp."),
        "{adventure}"
    );
    assert!(
        adventure.contains("You are free, in 8 moves."),
        "{adventure}"
    );

    // Without the lamp the great hall is dark, and without the sword the
    // hound in the courtyard is impassable.
    let blocked = run_with(
        include_str!("../examples/adventure.cra"),
        b"e
e
n
w
n
take lamp
s
e
n
n
",
    );
    assert!(blocked.contains("-- Darkness --"), "{blocked}");
    assert!(blocked.contains("You blunder into a pillar"), "{blocked}");
    assert!(blocked.contains("The hound rises"), "{blocked}");
    assert!(!blocked.contains("-- The Gate --"), "{blocked}");

    let sorted = run(include_str!("../examples/sort.cra"));
    assert!(
        sorted.contains("after:  1 3 7 9 19 23 31 42 55 64 77 88"),
        "{sorted}"
    );

    // A Brainfuck compiler, written in Cranium, emitting C.
    let lobotomy = include_str!("../examples/lobotomy.cra");
    let compiled = run_with(lobotomy, b"+++[->++<]>.[-]");
    // Runs are folded and `[-]` becomes a single store.
    assert!(compiled.contains("*p += 3;"), "{compiled}");
    assert!(compiled.contains("*p += 2;"), "{compiled}");
    assert!(compiled.contains("*p = 0;"), "{compiled}");
    assert!(
        !compiled.contains(
            "*p += 1;
    *p += 1;"
        ),
        "{compiled}"
    );
    // The emitted C is a complete, balanced program.
    assert!(compiled.starts_with(
        "/* generated by lobotomy */
#include <stdio.h>
"
    ));
    assert!(compiled.trim_end().ends_with(
        "return 0;
}"
    ));
    assert_eq!(
        compiled.matches('{').count(),
        compiled.matches('}').count(),
        "braces must balance:
{compiled}"
    );
    // Unmatched brackets are reported where a C compiler will refuse to build.
    assert!(run_with(lobotomy, b"+]+").contains("#error lobotomy: unmatched ]"));
    assert!(run_with(lobotomy, b"+[+").contains("#error lobotomy: unmatched ["));
    // Bytes that are not one of the eight commands are ignored.
    assert!(run_with(lobotomy, b"ignore me +").contains("*p += 1;"));

    // A Brainfuck interpreter, written in Cranium, running Brainfuck.
    let bfi = include_str!("../examples/bfi.cra");
    assert_eq!(run_with(bfi, b"++++++++[>++++++++<-]>+.!"), "A");
    assert_eq!(
        run_with(
            bfi,
            b"++++++++[>++++[>++>+++>+++>+<<<<-]>+>+>->>+[<]<-]>>.>---.+++++++..+++.>>.<-.<.+++.------.--------.>>+.>++.!"
        ),
        "Hello World!
"
    );
    assert_eq!(run_with(bfi, b",[.,]!echo me"), "echo me");
    assert!(run_with(bfi, b"[[!").contains("unmatched"));

    let life = run(include_str!("../examples/life.cra"));
    assert!(life.starts_with("generation 0\n.#..........\n"), "{life}");
    // The blinker is horizontal on even generations and vertical on odd ones.
    assert!(life.contains("\n.......###..\n"), "{life}");
    assert!(life.contains("generation 7"), "{life}");
}

/// Compile a set of named sources, entry first, and run the result.
fn run_project(files: &[(&str, &str)], input: &[u8]) -> String {
    let mut loader = module::Memory::new(files.iter().copied());
    let compiled = match cranium::compile_with(files[0].0, &mut loader) {
        Ok(output) => output,
        Err(err) => panic!("failed to compile: {err}"),
    };
    let tape = compiled.cells_used.max(30_000);
    let mut runtime = create_runtime_with_tape(&compiled.code, CellSize::Bits8, tape)
        .expect("cranium emits valid brainfuck");
    let mut input = std::io::Cursor::new(input.to_vec());
    let mut output = Vec::new();
    runtime
        .run_with_io(&mut input, &mut output)
        .expect("program runs");
    String::from_utf8(output).expect("output is text")
}

fn project_error(files: &[(&str, &str)]) -> String {
    let mut loader = module::Memory::new(files.iter().copied());
    cranium::compile_with(files[0].0, &mut loader)
        .err()
        .unwrap_or_else(|| panic!("expected a compile error"))
        .to_string()
}

#[test]
fn imports_pull_in_another_file() {
    let out = run_project(
        &[
            (
                "main.cra",
                "import \"math.cra\";\nfn main() { print(square(7)); }\n",
            ),
            (
                "math.cra",
                "fn square(n: byte) -> int { return n as int * n as int; }\n",
            ),
        ],
        b"",
    );
    assert_eq!(out, "49");
}

#[test]
fn imports_bring_constants_globals_and_arrays() {
    let out = run_project(
        &[
            (
                "main.cra",
                "import \"data.cra\";\nfn main() {\n print(SIZE); print(\" \");\n print(counter); print(\" \");\n puts(greeting); bump();\n print(\" \"); print(counter);\n}\n",
            ),
            (
                "data.cra",
                "const SIZE = 4;\nlet counter: int = 10;\nlet greeting: byte[8] = \"hey\";\nfn bump() { counter += SIZE as int; }\n",
            ),
        ],
        b"",
    );
    assert_eq!(out, "4 10 hey 14");
}

/// A file's imports land ahead of its own items, so a constant defined in one
/// is already in scope for a constant in the other.
#[test]
fn imported_constants_are_in_scope_for_later_ones() {
    let out = run_project(
        &[
            (
                "main.cra",
                "import \"base.cra\";\nconst DOUBLE = BASE * 2;\nfn main() { print(DOUBLE); }\n",
            ),
            ("base.cra", "const BASE = 21;\n"),
        ],
        b"",
    );
    assert_eq!(out, "42");
}

/// Two files importing the same third one must not define it twice.
#[test]
fn a_shared_import_is_only_included_once() {
    let out = run_project(
        &[
            (
                "main.cra",
                "import \"a.cra\";\nimport \"b.cra\";\nfn main() { print(from_a()); print(from_b()); }\n",
            ),
            (
                "a.cra",
                "import \"shared.cra\";\nfn from_a() -> byte { return shared() + 1; }\n",
            ),
            (
                "b.cra",
                "import \"shared.cra\";\nfn from_b() -> byte { return shared() + 2; }\n",
            ),
            ("shared.cra", "fn shared() -> byte { return 10; }\n"),
        ],
        b"",
    );
    assert_eq!(out, "1112");
}

#[test]
fn errors_name_the_file_they_are_in() {
    let message = project_error(&[
        ("main.cra", "import \"lib.cra\";\nfn main() { helper(); }\n"),
        ("lib.cra", "fn helper() {\n    let x = nope;\n}\n"),
    ]);
    assert!(message.starts_with("lib.cra:2:"), "{message}");
    assert!(message.contains("not defined"), "{message}");
}

#[test]
fn reports_import_problems() {
    let missing = project_error(&[("main.cra", "import \"gone.cra\";\nfn main() { }\n")]);
    assert!(missing.contains("gone.cra"), "{missing}");
    assert!(missing.starts_with("main.cra:1:"), "{missing}");

    let cycle = project_error(&[
        ("main.cra", "import \"a.cra\";\nfn main() { }\n"),
        ("a.cra", "import \"main.cra\";\n"),
    ]);
    assert!(cycle.contains("import cycle"), "{cycle}");

    // One namespace across the program, so a clash is an error.
    let clash = project_error(&[
        (
            "main.cra",
            "import \"lib.cra\";\nfn helper() { }\nfn main() { }\n",
        ),
        ("lib.cra", "fn helper() { }\n"),
    ]);
    assert!(clash.contains("more than once"), "{clash}");
}

#[test]
fn a_detached_source_cannot_import() {
    let message = error_of("import \"lib.cra\";\nfn main() { }\n");
    assert!(message.contains("lib.cra"), "{message}");
}

/// The multi-file example, loaded from disk through the real import
/// resolution rather than a loader built for the test.
#[test]
fn compiles_the_multi_file_example() {
    let compiled = cranium::compile_file("examples/project/main.cra")
        .unwrap_or_else(|err| panic!("failed to compile the project: {err}"));

    let tape = compiled.cells_used.max(30_000);
    let mut runtime = create_runtime_with_tape(&compiled.code, CellSize::Bits8, tape)
        .expect("cranium emits valid brainfuck");
    let mut input = std::io::Cursor::new(Vec::new());
    let mut output = Vec::new();
    runtime
        .run_with_io(&mut input, &mut output)
        .expect("program runs");

    assert_eq!(
        String::from_utf8(output).expect("output is text"),
        "CRANIUM
count 3, mean 5 #####
"
    );
}

/// Graphics with no runtime: the terminal is the display, so the whole
/// "driver" is an escape sequence per pixel pair.
#[test]
fn truecolor_draws_a_gradient_out_of_half_blocks() {
    let frame = run(include_str!("../examples/truecolor.cra"));

    // 64x32 pixels is 16 rows of 64 cells, each cell two vertical pixels.
    let rows: Vec<&str> = frame.lines().collect();
    assert_eq!(rows.len(), 16, "wrong number of text rows");
    for row in &rows {
        assert_eq!(
            row.matches('\u{2580}').count(),
            64,
            "a row is not 64 cells wide"
        );
        // Each row hands the terminal its colours back before the newline.
        assert!(row.ends_with("\u{1b}[0m"), "a row did not reset the colour");
    }

    // The first cell is the top-left pixel and the one below it: the gradient
    // runs blue at x = 0 and green down the y axis.
    assert!(
        frame.starts_with("\u{1b}[38;2;0;0;255m\u{1b}[48;2;0;8;255m\u{2580}"),
        "{:?}",
        &frame[..frame.len().min(60)]
    );

    // Every pixel is a full 24-bit colour, and both layers of every cell are
    // set, or a cell would inherit whatever came before it.
    assert_eq!(frame.matches("\u{1b}[38;2;").count(), 16 * 64);
    assert_eq!(frame.matches("\u{1b}[48;2;").count(), 16 * 64);
}

/// The animation contract: `home` before each frame is what draws in place,
/// and a read that returns nothing is what lets the loop keep running.
///
/// Deliberately an 8x4 screen. The interpreter is slow enough that a full
/// 64x32 frame costs seconds, and nothing being checked here depends on size.
#[test]
fn gfx_animates_in_place() {
    let frames = run_with(
        r#"
import "std/gfx.cra";

fn shade(x: byte, y: byte) {
    set_rgb(x, y, 0);
}

fn main() {
    gfx_w = 8;
    gfx_h = 4;

    hide_cursor();
    clear();

    loop {
        let key = getc();
        if key == 'q' {
            break;
        }
        home();
        present();
    }

    show_cursor();
    reset_color();
}
"#,
        // Two bytes the program ignores, which is what a key nobody pressed
        // looks like, then quit.
        b"..q",
    );

    // 8x4 pixels is 8 cells across and 2 rows down, twice over.
    assert_eq!(frames.matches('\u{2580}').count(), 2 * 8 * 2);
    // One home from `clear`, then one before each frame.
    assert_eq!(frames.matches("\u{1b}[H").count(), 3);
    // The terminal is left as it was found.
    assert_eq!(frames.matches("\u{1b}[?25l").count(), 1);
    assert!(
        frames.ends_with("\u{1b}[?25h\u{1b}[0m"),
        "cursor and colour not restored"
    );
}

/// The animated example is big enough that running it here would cost more
/// than it is worth, so this only checks it still compiles - which is what
/// would break if the library changed under it.
#[test]
fn bounce_example_compiles() {
    let compiled =
        compile_str(include_str!("../examples/bounce.cra")).expect("bounce.cra should compile");

    assert!(compiled.code.len() > 100_000, "suspiciously small");
    assert!(
        compiled.cells_used < 200,
        "the demo should not want a big tape"
    );
}
