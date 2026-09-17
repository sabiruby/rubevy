//! `src/` may not name a std API that is not there in a browser.
//!
//! rubevy runs in a browser (`wasm32-unknown-unknown`) as well as natively, and on
//! 2026-09-18 the published garden was a black screen because of one line of this crate.
//! The tick's answer loop, added when component reads became synchronous, measured the
//! frame with `std::time::Instant::now()`. That target has no clock in std, so the call is
//! `panic!("time not implemented on this platform")` — a panic in the middle of the first
//! `Update`, which takes the whole page with it before a single frame is drawn. The story,
//! with the console output and the fix (the loop now keeps time on `clock_ns`, Bevy's
//! `Instant`), is in `docs/worklog/2026-09-18-wasm-instant.md`.
//!
//! **A wasm build does not catch this.** `cargo build --target wasm32-unknown-unknown`
//! finished happily on the broken code: std compiles for that target with its `unsupported`
//! backend, so every one of these APIs *type-checks* and only fails — by panicking, by
//! always returning an error, or by doing nothing — when it is reached at run time. Adding
//! the wasm build to CI would not have said a word. Nor do the tests: they are native, and
//! natively the line was correct. So this test reads the source instead, which is the one
//! place the mistake is visible without a browser.
//!
//! What is forbidden, and what is not:
//!
//! * `std::time::Instant` and `std::time::SystemTime` — no clock. `std::time::Duration` is
//!   fine: it is arithmetic, not a clock.
//! * `Instant::now()` written bare, because the reader cannot tell which `Instant` it is.
//!   `bevy::platform::time::Instant` (`web-time` in a browser) is the one to use, and a file
//!   that says `use bevy::platform::time::Instant;` may then call `Instant::now()` bare —
//!   the `use` line is what tells the two apart. Any other import of a name `Instant` needs
//!   an exception comment saying where it comes from.
//! * `std::thread` — a browser's main thread is the only one; bevy's task pools are what a
//!   frame has instead.
//! * `std::fs`, `std::net`, `std::process` — no filesystem, no sockets, no processes.
//!
//! The only way past it is a comment **on the line** beginning `// wasm:` and saying why,
//! for example `// wasm: native-only, behind cfg`. `#[cfg(not(target_arch = "wasm32"))]`
//! is *not* enough on its own: this test does not read cfgs — it reads lines — and a cfg
//! elsewhere in the file says nothing about what the browser build does when it reaches
//! that code by another road. Write the reason down.
//!
//! A line that is only a comment is skipped, since a comment calls nothing. That is how the
//! answer loop's own rustdoc may go on explaining what `std::time::Instant` would have done.

use std::path::{Path, PathBuf};

/// A thing `src/` may not name, and why a browser cannot have it.
struct Forbidden {
    /// The text to look for in a line, after `use` groups have been flattened.
    path: &'static str,
    why: &'static str,
}

const FORBIDDEN: &[Forbidden] = &[
    Forbidden {
        path: "std::time::Instant",
        why: "std has no clock on wasm32-unknown-unknown: `Instant::now()` panics there. \
              Use `bevy::platform::time::Instant` (or rubevy's `clock_ns`).",
    },
    Forbidden {
        path: "std::time::SystemTime",
        why: "std has no clock on wasm32-unknown-unknown: `SystemTime::now()` panics there.",
    },
    Forbidden {
        path: "std::thread",
        why: "A browser gives this crate one thread. Bevy's task pools are what a frame has \
              instead; `thread::sleep` has no browser meaning at all.",
    },
    Forbidden {
        path: "std::fs",
        why: "A browser has no filesystem. An asset comes through bevy's `AssetServer`.",
    },
    Forbidden {
        path: "std::net",
        why: "A browser has no sockets. Fetch and WebSocket are what a page can open.",
    },
    Forbidden {
        path: "std::process",
        why: "A browser has no processes to start or to exit from.",
    },
];

/// The import that makes a bare `Instant::now()` the browser's one.
const BEVY_INSTANT: &str = "bevy::platform::time::Instant";

/// The comment that lets a line through, with the reason after it.
const EXCEPTION: &str = "// wasm:";

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` under `dir`, deepest last, sorted so that the report is the same every run.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&d)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", d.display()))
            .map(|e| e.expect("cannot read a directory entry").path())
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                todo.push(p);
            } else if p.extension().is_some_and(|e| e == "rs") {
                found.push(p);
            }
        }
    }
    found.sort();
    found
}

/// The commas of `body` that are not inside a `{…}`.
fn top_level_commas(body: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, c) in body.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&body[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&body[start..]);
    parts
}

/// `use a::{b, c::{d, e}};` written out as `a::b`, `a::c::d`, `a::c::e`, so that a grouped
/// import is compared against the same text a plain one is. Anything that is not a `use`
/// line comes back as itself.
fn flatten_use(line: &str) -> Vec<String> {
    let mut out = vec![line.to_string()];
    let trimmed = line.trim_start();
    let rest = match trimmed.strip_prefix("pub use ").or_else(|| trimmed.strip_prefix("use ")) {
        Some(r) => r,
        None => return out,
    };
    fn walk(prefix: &str, body: &str, out: &mut Vec<String>) {
        for part in top_level_commas(body) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            match (part.find('{'), part.rfind('}')) {
                (Some(o), Some(c)) if o < c => {
                    let head = format!("{prefix}{}", part[..o].trim());
                    walk(&head, &part[o + 1..c], out);
                }
                _ => out.push(format!("{prefix}{part}")),
            }
        }
    }
    let rest = rest.trim_end().trim_end_matches(';');
    match (rest.find('{'), rest.rfind('}')) {
        (Some(o), Some(c)) if o < c => walk(rest[..o].trim(), &rest[o + 1..c], &mut out),
        _ => out.push(rest.trim().to_string()),
    }
    out
}

/// True when the line is nothing but a comment: it calls nothing, so it may say anything.
fn is_comment_only(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") || t.starts_with("/*") || t.starts_with('*')
}

/// True when the line carries `// wasm:` with a reason after it.
fn has_exception(line: &str) -> bool {
    match line.split_once(EXCEPTION) {
        Some((_, reason)) => !reason.trim().is_empty(),
        None => false,
    }
}

/// Why this line may not be in `src/`, or nothing.
fn verdict(line: &str, bevy_instant_imported: bool) -> Option<String> {
    let mut reasons: Vec<&str> = Vec::new();
    let expanded = flatten_use(line);
    for f in FORBIDDEN {
        if expanded.iter().any(|e| e.contains(f.path)) {
            reasons.push(f.why);
        }
    }
    // A bare `Instant::now()`: which `Instant`? Fully qualified with bevy's path it is the
    // browser's; so it is in a file that imported that one by name. Otherwise say so.
    if !bevy_instant_imported {
        let mut from = 0usize;
        while let Some(i) = line[from..].find("Instant::now") {
            let at = from + i;
            if !line[..at].trim_end().ends_with("bevy::platform::time::") {
                reasons.push(
                    "A bare `Instant::now()` does not say which `Instant` it is. Import \
                     `bevy::platform::time::Instant` (the browser has that one) or write the \
                     path out.",
                );
                break;
            }
            from = at + "Instant::now".len();
        }
    }
    if reasons.is_empty() {
        return None;
    }
    reasons.dedup();
    Some(reasons.join(" "))
}

/// Does this file import bevy's `Instant` by name?
fn imports_bevy_instant(text: &str) -> bool {
    text.lines().filter(|l| !is_comment_only(l)).any(|l| {
        flatten_use(l).iter().any(|e| {
            let e = e.trim();
            e == BEVY_INSTANT || e.starts_with(&format!("{BEVY_INSTANT} as "))
        })
    })
}

#[test]
fn src_names_nothing_a_browser_lacks() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut hits: Vec<String> = Vec::new();
    for file in rust_files(&src_dir()) {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let bevy_instant = imports_bevy_instant(&text);
        let shown = file.strip_prefix(root).unwrap_or(&file).display().to_string();
        for (i, line) in text.lines().enumerate() {
            if is_comment_only(line) || has_exception(line) {
                continue;
            }
            if let Some(why) = verdict(line, bevy_instant) {
                hits.push(format!("{shown}:{}: {}\n    → {why}", i + 1, line.trim()));
            }
        }
    }
    assert!(
        hits.is_empty(),
        "{} line(s) of src/ name a std API a browser does not have. Natively they work and \
         the wasm build compiles them, so nothing else here will tell you; in a browser they \
         panic or quietly do nothing (see this file's header and \
         docs/worklog/2026-09-18-wasm-instant.md). Fix them, or put `{EXCEPTION} <reason>` on \
         the line.\n\n{}\n",
        hits.len(),
        hits.join("\n")
    );
}

/// The guard itself, on lines it will never meet in `src/`: a test that cannot fail says
/// nothing, and this one is only ever exercised by the day somebody writes the bad line.
#[test]
fn the_guard_catches_what_it_is_for() {
    let caught = |line: &str| verdict(line, false).is_some();

    // The line that made the garden a black screen, and its relatives.
    assert!(caught("        let started = std::time::Instant::now();"));
    assert!(caught("let now = Instant::now();"));
    assert!(caught("use std::time::Instant;"));
    assert!(caught("use std::time::{Duration, Instant};"));
    assert!(caught("use std::{fmt, time::{Duration, SystemTime}};"));
    assert!(caught("std::thread::sleep(d);"));
    assert!(caught("use std::{fs, io::Write};"));
    assert!(caught("let s = std::net::TcpStream::connect(a);"));
    assert!(caught("std::process::exit(1);"));

    // What the browser does have, or what is not a clock at all.
    assert!(!caught("ORIGIN.get_or_init(bevy::platform::time::Instant::now).elapsed()"));
    assert!(!caught("    pub frame_time: Option<std::time::Duration>,"));
    assert!(!caught("use std::sync::{Arc, Mutex, OnceLock};"));
    assert!(!caught("std::mem::take(&mut v)"));

    // A file that imported bevy's `Instant` may call it bare; one that did not, may not.
    assert!(verdict("let now = Instant::now();", true).is_none());
    assert!(imports_bevy_instant("use bevy::platform::time::Instant;\n"));
    assert!(imports_bevy_instant("use bevy::platform::time::{Duration, Instant};\n"));
    assert!(!imports_bevy_instant("// use bevy::platform::time::Instant;\n"));
    assert!(!imports_bevy_instant("use std::time::Instant;\n"));

    // The two ways past the guard, and the one that is not a way past it.
    assert!(is_comment_only("    //! It is not `std::time::Instant`: that one panics."));
    assert!(has_exception("        std::fs::read(path).ok() // wasm: native-only"));
    assert!(!has_exception("        std::fs::read(path).ok() // wasm:"));
    assert!(!has_exception("        std::fs::read(path).ok() // native only"));
}
