//! End-to-end tests: compile Cranium, run the Brainfuck, check the output.

use cranium::compile_str;
use lobe::{create_runtime_with_tape, CellSize};

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
    assert_eq!(
        run(&main_of("print(-1); print(\" \"); print(!0); print(!5);")),
        "255 10"
    );
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

#[test]
fn emits_only_brainfuck_commands() {
    let compiled = compile_str(&main_of("print(42);")).expect("valid program");
    assert!(compiled
        .code
        .bytes()
        .all(|byte| b"+-<>.,[]".contains(&byte)));
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
    assert!(message.starts_with("3:"), "{message}");
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

    let sorted = run(include_str!("../examples/sort.cra"));
    assert!(
        sorted.contains("after:  1 3 7 9 19 23 31 42 55 64 77 88"),
        "{sorted}"
    );

    let life = run(include_str!("../examples/life.cra"));
    assert!(life.starts_with("generation 0\n.#..........\n"), "{life}");
    // The blinker is horizontal on even generations and vertical on odd ones.
    assert!(life.contains("\n.......###..\n"), "{life}");
    assert!(life.contains("generation 7"), "{life}");
}
