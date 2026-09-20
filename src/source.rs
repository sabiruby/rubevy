//! What a script is before it is bytecode: a prelude put in front of the author's own file, and
//! the line numbers a compiler reports put back into that file.
//!
//! **rubevy does not compile anything here.** It has no compiler of its own (`ruby-source` brings
//! one along for `require`, and that is all), and in a browser the compiler is a function the page
//! defines. So both halves of this module work on **text**: one builds the program a compiler is
//! handed, the other reads the message it gives back. Neither knows what a compiler is.
//!
//! The two belong together because the second undoes what the first did. A prelude in front of a
//! file moves every line of that file down, so the compiler names a line the author cannot find:
//! one of the sample games reported line 600 for what its author had written on line 118, and the
//! other — which had the first half of this and not the second — still reports line numbers 295
//! lines too far down.

/// A prelude and an author's file, put together as one program to compile, with the number of
/// lines that went in front of the author's first line.
///
/// The program is
///
/// ```text
/// {prelude}
/// # ---- {name} ----
/// {body}
/// {tail}
/// ```
///
/// One program rather than a `require`, because the author's file is meant to read as if the
/// prelude's methods were the language: it calls them at the top level and defines what the
/// prelude will call back. The separator comment is there so that somebody reading the source
/// that was really compiled — a listing, a dump — can see where the author's file starts.
///
/// `tail` is what starts the program once both halves are in it (a call the prelude defines,
/// `"run"`); leave it empty and the program is just the two files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// The whole source, for whatever compiler the host has.
    pub source: String,
    /// What to call this file: the name that went into the separator comment, and the name to
    /// hand the compiler as the file being compiled.
    ///
    /// It is here because the two are the same name and a caller that writes it twice can write
    /// it twice differently — the separator saying `brain.rb` while the compiler's messages say
    /// something else. All three callers in rubevy_games had it in hand and passed it on
    /// (`platform::compile(&program.source, name)`); now the program carries it:
    ///
    /// ```no_run
    /// # use rubevy::Program;
    /// # fn compile(_source: &str, _name: &str) -> Result<Vec<u8>, String> { Ok(Vec::new()) }
    /// # fn build(prelude: &str, players_text: &str) -> Result<Vec<u8>, String> {
    /// let program = Program::new(prelude, "brain.rb", players_text, "run");
    /// let bytes = compile(&program.source, &program.name)?;
    /// # Ok(bytes) }
    /// ```
    ///
    /// **A compiler is free to ignore it**, and one of them does: the playground's browser bridge
    /// is handed a source and nothing else, and names every program `playground.rb`. That is why
    /// this is a name the program *has* rather than a name it is compiled under —
    /// [`in_the_authors_lines`] looks for the line number in a message, not for this.
    pub name: String,
    /// How many lines stand in front of the author's own first line: the author's line 1 is line
    /// `prelude_lines + 1` of [`Program::source`], and a compiler's line *n* is the author's
    /// `n - prelude_lines`.
    ///
    /// It is counted off the text that was actually put in front, rather than added up from the
    /// pieces, so it stays right whether or not the prelude ended with a newline of its own.
    ///
    /// This is the number [`in_the_authors_lines`] takes, and the same number a panel that shows
    /// a script's frames takes (rubevy_games' `VmInspector::fill`): both subtract it from a line
    /// the VM or the compiler reports.
    pub prelude_lines: u32,
}

impl Program {
    /// Puts the four pieces together. `name` goes in the separator comment and is kept as
    /// [`Program::name`], which is the name to hand the compiler; what a compiler does with a
    /// file name is still the compiler's own business, and in a browser it is not the caller's to
    /// choose at all (the playground's bridge names every program `playground.rb`).
    pub fn new(prelude: &str, name: &str, body: &str, tail: &str) -> Program {
        let head = format!("{prelude}\n# ---- {name} ----\n");
        // the lines that end before the body begins. `lines()` counts a final line without a
        // newline and does not count one after a trailing newline, which is exactly the
        // difference between a prelude read from a file and one written in the source.
        let prelude_lines = head.lines().count() as u32;
        Program {
            source: format!("{head}{body}\n{tail}\n"),
            name: name.to_string(),
            prelude_lines,
        }
    }
}

/// **Takes the prelude off the line numbers a compiler reported.**
///
/// `message` is what the compiler said, `prelude_lines` is [`Program::prelude_lines`], and
/// `prelude_name` is what to call the prelude when the error turns out to be in it.
///
/// Both compilers rubevy's callers use print `FILE:LINE:COL: message`, one diagnostic to a line:
/// natively that is `sabiruby_compiler::Diagnostic`'s `Display` ("as `mrbc` prints it"), and in a
/// browser it is the same `Display` inside the playground's wasm module, with the file always
/// named `playground.rb` because the bridge is handed a source and nothing else. So the one thing
/// to find in a line is the `:LINE:COL:` in it, and the first one on a line is the only one that
/// can be it — anything in front of it is the file's name, which may itself hold colons.
///
/// Three things can come back:
///
/// * a line **past the prelude** — the author's file: the number moves up by `prelude_lines`.
///   That includes a line past the author's last one, which is the `tail`
///   ([`Program::new`]): it is reported as the line after the file's end, which is where the
///   program's last statement really is.
/// * a line **inside the prelude** — not the author's file at all: it is said so by name, with
///   the line it really is, rather than as a number the author cannot find (and rather than a
///   negative one).
/// * a line with **no place in it** — `CompileError` renders "compile error" when the compiler
///   gave it no diagnostics, and a message about a missing file is the caller's own. It is handed
///   back whole: a message with no place is still the whole of what was said.
pub fn in_the_authors_lines(message: &str, prelude_lines: u32, prelude_name: &str) -> String {
    message
        .lines()
        .map(|line| one_diagnostic(line, prelude_lines, prelude_name))
        .collect::<Vec<_>>()
        .join("\n")
}

/// One line of a compiler's message, with its line number moved.
fn one_diagnostic(line: &str, prelude_lines: u32, prelude_name: &str) -> String {
    let bytes = line.as_bytes();
    // a run of digits from `at`, and where it ends
    let digits = |at: usize| -> (Option<u32>, usize) {
        let end = at + bytes[at..].iter().take_while(|b| b.is_ascii_digit()).count();
        (line[at..end].parse().ok(), end)
    };
    for colon in 0..line.len() {
        if bytes[colon] != b':' {
            continue;
        }
        let (Some(number), after_line) = digits(colon + 1) else { continue };
        if bytes.get(after_line) != Some(&b':') {
            continue;
        }
        let (Some(_), after_col) = digits(after_line + 1) else { continue };
        if bytes.get(after_col) != Some(&b':') {
            continue;
        }
        if number > prelude_lines {
            return format!("{}:{}{}", &line[..colon], number - prelude_lines, &line[after_line..]);
        }
        // the file's name is the word in front of that colon; the prelude's goes in its place
        let name_at = line[..colon].rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
        return format!("{}{prelude_name}:{number}{}", &line[..name_at], &line[after_line..]);
    }
    line.to_string()
}
