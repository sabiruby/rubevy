//! A program built out of a prelude and an author's file, and the compiler's line numbers put
//! back into that file.
//!
//! The cases are rubevy_games' garden's, moved here with the creatures' names taken out: they are
//! what a person meets — an error in the file being edited, an error in the prelude in front of
//! it, a message with no place in it at all — and the first of them is the real message the game
//! printed on 2026-09-18, for a line its author had written on line 118 of a 482-line-prelude
//! program.
//!
//! Two of the tests do not stop at the shape: they run the real compiler and read its real
//! message, so that the parsing here is checked against what `sabiruby_compiler` prints today
//! rather than against what this file remembers of it.

use rubevy::{in_the_authors_lines, Program};

/// What the prelude is called when the error turns out to be in it.
const PRELUDE: &str = "prelude.rb";

#[test]
fn a_compiler_line_is_reported_in_the_authors_own_file() {
    let real = "script.rb: script.rb:600:19: syntax error, unexpected '<'; expected an expression after the operator";
    assert_eq!(
        in_the_authors_lines(real, 482, PRELUDE),
        "script.rb: script.rb:118:19: syntax error, unexpected '<'; expected an expression after the operator"
    );
    // a line inside the prelude is not the author's file: it is said by name, with the line it
    // really is, rather than as a number nobody can find or a negative one
    assert_eq!(
        in_the_authors_lines("script.rb: script.rb:47:3: syntax error", 482, PRELUDE),
        "script.rb: prelude.rb:47:3: syntax error"
    );
    // the boundary: the prelude's last line is the prelude's, the first line after it is the
    // author's line 1
    assert_eq!(in_the_authors_lines("f:482:1: x", 482, PRELUDE), "prelude.rb:482:1: x");
    assert_eq!(in_the_authors_lines("f:483:1: x", 482, PRELUDE), "f:1:1: x");
    // another prelude for another kind of script is the same machinery under another name
    assert_eq!(
        in_the_authors_lines("rules.rb: rules.rb:20:1: syntax error", 120, "rules_prelude.rb"),
        "rules.rb: rules_prelude.rb:20:1: syntax error"
    );
    // several diagnostics, one to a line, each moved
    assert_eq!(
        in_the_authors_lines("a.rb:600:1: one\na.rb:610:2: two", 482, PRELUDE),
        "a.rb:118:1: one\na.rb:128:2: two"
    );
    // and a message with no place in it is handed back whole: `CompileError` renders
    // "compile error" when the compiler gave it no diagnostics, and a missing file's message is
    // the caller's own, which has a path with colons in it and no line
    assert_eq!(in_the_authors_lines("script.rb: compile error", 482, PRELUDE), "script.rb: compile error");
    let missing = "/home/x/game/ruby/script.rb: No such file or directory (os error 2)";
    assert_eq!(in_the_authors_lines(missing, 482, PRELUDE), missing);
}

/// The third place an error can be: **past the author's last line**, which is where the `tail`
/// is. The program ends with a call the prelude defines, and a `def` the author left open is
/// reported there — on the line after the file the author is looking at.
#[test]
fn an_error_past_the_authors_last_line_is_the_line_after_it() {
    let body = "def think\n  1\n"; // no `end`
    let program = Program::new("# prelude\ndef run; think; end\n", "script.rb", body, "run");
    // the author's two lines, then the blank left by the body's own trailing newline, then the
    // tail: the `run` the compiler would point at is the 4th line after the prelude
    let at_the_tail = program
        .source
        .lines()
        .position(|l| l == "run")
        .expect("the tail is in the program") as u32
        + 1;
    assert_eq!(at_the_tail, program.prelude_lines + 4);
    let message = format!("script.rb:{at_the_tail}:1: syntax error, unexpected end-of-input");
    assert_eq!(
        in_the_authors_lines(&message, program.prelude_lines, PRELUDE),
        "script.rb:4:1: syntax error, unexpected end-of-input",
        "past the end is reported past the end, not clamped and not made negative"
    );
}

/// The number is counted off the text that really went in front, so a prelude that ends with a
/// newline (one read from a file) and one that does not (one written in the source) both answer
/// the line the author's first line became.
#[test]
fn the_authors_first_line_is_where_the_program_says_it_is() {
    for prelude in ["def run; end", "def run; end\n"] {
        let program = Program::new(prelude, "script.rb", "answer 1\nanswer 2", "run");
        let line = program
            .source
            .lines()
            .position(|l| l == "answer 1")
            .expect("the body is in the program") as u32
            + 1;
        assert_eq!(
            line,
            program.prelude_lines + 1,
            "prelude {prelude:?} says {} lines, body at {line}",
            program.prelude_lines
        );
        // and what the compiler would report for that line comes back as the author's line 1
        let message = format!("script.rb:{line}:1: whatever");
        assert_eq!(in_the_authors_lines(&message, program.prelude_lines, PRELUDE), "script.rb:1:1: whatever");
    }
}

/// The separator is in the program, so that a listing of what was really compiled says where the
/// author's file begins.
#[test]
fn the_program_is_the_two_files_and_the_call_that_starts_them() {
    let program = Program::new("# prelude", "brain.rb", "walk", "run");
    assert_eq!(program.source, "# prelude\n# ---- brain.rb ----\nwalk\nrun\n");
    assert_eq!(program.prelude_lines, 2);
}

/// **The program carries the name**, so that the separator comment and the name the compiler is
/// given cannot drift apart. All three places in rubevy_games wrote it twice —
/// `Program::new(&prelude, name, body, tail)` and then `platform::compile(&program.source,
/// name)` — and two spellings of one name are a name that can be changed in one of them.
#[test]
fn the_program_knows_what_to_call_the_file() {
    let program = Program::new("# prelude", "brain.rb", "walk", "run");
    assert_eq!(program.name, "brain.rb");
    assert!(
        program.source.contains(&format!("# ---- {} ----", program.name)),
        "the same name the separator got: {}",
        program.source
    );
    // and it is the one to hand a compiler, which is what the separator says the file is
    let opts = sabiruby_compiler::Options {
        filename: program.name.clone(),
        ..Default::default()
    };
    let error = sabiruby_compiler::compile(b"1 < < 2", &opts).expect_err("does not compile");
    assert!(error.to_string().starts_with("brain.rb:"), "{error}");
}

/// **What the compiler really prints**, rather than what this file remembers of it: a program
/// with a prelude in front is compiled for real, and the message that comes back is moved into
/// the author's own line.
#[test]
fn the_reference_compilers_own_message_moves_into_the_authors_file() {
    let prelude = "def run\n  think\nend\n";
    // line 2 of the author's file: an operator with nothing after it
    let body = "def think\n  1 < < 2\nend";
    let program = Program::new(prelude, "script.rb", body, "run");
    let opts = sabiruby_compiler::Options { filename: "script.rb".into(), ..Default::default() };
    let error = sabiruby_compiler::compile(program.source.as_bytes(), &opts)
        .expect_err("`1 < < 2` does not compile");
    let message = error.to_string();
    assert!(
        message.starts_with(&format!("script.rb:{}:", program.prelude_lines + 2)),
        "the compiler names the line of the whole program: {message}"
    );
    let moved = in_the_authors_lines(&message, program.prelude_lines, PRELUDE);
    assert!(moved.starts_with("script.rb:2:"), "put back into the author's file: {moved}");
}

/// **And what the browser's bridge prints.** The playground's wasm module compiles with
/// `sabiruby_compiler` too and hands JS `CompileError`'s `Display` — the same text — with one
/// difference: the bridge is given a source and nothing else, so every program a page compiles is
/// called `playground.rb` (`sabiruby-playground/wasm/src/lib.rs`, `FILENAME`; rubevy_games'
/// `docs/web.md`). This is that message, made the way the module makes it. It does not run a
/// browser; what it checks is that a file name nobody chose is still only a name, and the line
/// number in front of the colon is found and moved all the same.
#[test]
fn the_browser_bridges_message_moves_too() {
    let program = Program::new("def run\n  think\nend\n", "script.rb", "def think\n  1 < < 2\nend", "run");
    let opts = sabiruby_compiler::Options { filename: "playground.rb".into(), ..Default::default() };
    let error = sabiruby_compiler::compile(program.source.as_bytes(), &opts)
        .expect_err("`1 < < 2` does not compile");
    // the page throws the message, and the caller puts the file it asked for in front of it
    let thrown = format!("script.rb: {error}");
    let moved = in_the_authors_lines(&thrown, program.prelude_lines, PRELUDE);
    assert!(
        moved.starts_with("script.rb: playground.rb:2:"),
        "the author's line, under the only name a page can give it: {moved}"
    );
}
