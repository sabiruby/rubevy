//! Puts a directory of Ruby files into the binary, for a build that has no directory to read.
//!
//! A browser has no filesystem: a page is served a wasm module and whatever it fetches, and
//! `std::fs` answers nothing there. A game whose scripts are `.rb` files on a PC therefore needs
//! the same files *inside* the binary in the browser build, and the way to put them there is a
//! build script that writes a table of `include_str!`s. Both of rubevy's sample games wrote that
//! build script, and the two were the same file to the byte — 35 lines each.
//!
//! ```no_run
//! // the whole of a game's build.rs, inside its `fn main`
//! rubevy_build::Embed::new("ruby").write();
//! ```
//!
//! ```text
//! // src/platform.rs, in the browser half
//! include!(concat!(env!("OUT_DIR"), "/ruby_files.rs"));
//! // `pub static RUBY_FILES: &[(&str, &str)]`, one pair per file:
//! // ("ruby/lib/helper.rb", "module Helper\n…")
//! ```
//!
//! The table is what `rubevy::EmbeddedHost` takes, so the same files a script `require`s on a PC
//! are the ones it requires in a page — under the same paths, which is why they are written
//! relative to the crate rather than as the absolute paths the build machine had.
//!
//! **This crate is std and nothing else.** It is a build-dependency, and everything a
//! build-dependency names is compiled for the build machine before the crate that uses it starts;
//! putting this in rubevy itself would build Bevy a second time for every game that embeds a
//! script.
//!
//! It also never runs in a browser — it runs on the machine doing the building, for every target
//! — so the `std::fs` in here is not the thing rubevy's own `tests/no_wasm_unsupported.rs`
//! forbids in `src/`. Reading the directory is the whole point of it.

use std::path::{Path, PathBuf};

/// A directory to put into the binary, and what to call the table it becomes.
///
/// The defaults are the two games' build script, so that a game that had that file can delete it
/// and write `rubevy_build::Embed::new("ruby").write()` without changing the `include!` on the
/// other side: `.rb` files under the directory, a `pub static RUBY_FILES` in `ruby_files.rs`.
/// Everything about that is settable, because a game with two kinds of embedded file needs two
/// tables with two names.
#[derive(Debug, Clone)]
pub struct Embed {
    dir: PathBuf,
    extensions: Vec<String>,
    name: String,
    out_file: String,
    bytes: bool,
}

impl Embed {
    /// Files under `dir`, which is relative to the crate being built (`CARGO_MANIFEST_DIR`).
    pub fn new(dir: impl Into<PathBuf>) -> Embed {
        Embed {
            dir: dir.into(),
            // what the two games embedded, and what a `require` of source needs
            extensions: vec![String::from("rb")],
            name: String::from("RUBY_FILES"),
            out_file: String::from("ruby_files.rs"),
            bytes: false,
        }
    }

    /// Which files to take, by extension and without the dot (`["rb"]` by default; `["mrb"]`
    /// with [`Embed::as_bytes`] for a table of compiled files).
    pub fn extensions(mut self, extensions: &[&str]) -> Embed {
        self.extensions = extensions.iter().map(|e| e.trim_start_matches('.').to_string()).collect();
        self
    }

    /// What to call the `static` (`RUBY_FILES` by default).
    pub fn name(mut self, name: impl Into<String>) -> Embed {
        self.name = name.into();
        self
    }

    /// What to call the file written into `OUT_DIR` (`ruby_files.rs` by default), which is the
    /// name the `include!` on the other side spells.
    pub fn out_file(mut self, out_file: impl Into<String>) -> Embed {
        self.out_file = out_file.into();
        self
    }

    /// Embed the files as **bytes** (`&[(&str, &[u8])]`, `include_bytes!`) rather than as text.
    /// That is what a `.mrb` is: RITE bytecode, which is not a string and which needs no
    /// compiler at the other end.
    pub fn as_bytes(mut self) -> Embed {
        self.bytes = true;
        self
    }

    /// Writes the table into `OUT_DIR` and tells cargo when to run again. Answers the file it
    /// wrote, which is the path an `include!` names.
    ///
    /// It **panics** on anything it cannot do, which is what a build script should do: a table
    /// that is quietly empty is a browser build with no scripts in it, and that is found much
    /// later and much further away.
    pub fn write(&self) -> PathBuf {
        let root = PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR")
                .expect("rubevy-build: CARGO_MANIFEST_DIR is not set — this is a build script"),
        );
        let out = PathBuf::from(
            std::env::var("OUT_DIR").expect("rubevy-build: OUT_DIR is not set — this is a build script"),
        )
        .join(&self.out_file);
        let files = self.collect(&root);
        std::fs::write(&out, self.render(&files))
            .unwrap_or_else(|e| panic!("rubevy-build: cannot write {}: {e}", out.display()));
        // The directory first: cargo walks a directory it is given, which is what notices a file
        // that was *added*. Each file as well, because a directory whose mtime does not move —
        // an editor that writes in place — would otherwise say nothing changed.
        println!("cargo:rerun-if-changed={}", root.join(&self.dir).display());
        for (_, abs) in &files {
            println!("cargo:rerun-if-changed={}", abs.display());
        }
        out
    }

    /// The files to embed: the relative path each one will be known by, and the absolute path the
    /// `include_*!` reads it from. Sorted by the relative path, so that the table is the same on
    /// every machine and a rebuild changes nothing by accident.
    fn collect(&self, root: &Path) -> Vec<(String, PathBuf)> {
        let mut files = Vec::new();
        let mut todo = vec![root.join(&self.dir)];
        while let Some(dir) = todo.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    todo.push(path);
                } else if path
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| self.extensions.iter().any(|want| want == e))
                {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        // the path a script requires has `/` in it whatever the build machine uses
                        .replace('\\', "/");
                    files.push((rel, path));
                }
            }
        }
        files.sort();
        files
    }

    /// The Rust source of the table. Separate from the writing so that it can be read in a test.
    fn render(&self, files: &[(String, PathBuf)]) -> String {
        let (ty, include) =
            if self.bytes { ("&[u8]", "include_bytes!") } else { ("&str", "include_str!") };
        let name = &self.name;
        let mut out = format!(
            "// Written by rubevy-build from {}. Do not edit: it is rebuilt from the directory.\n\
             pub static {name}: &[(&str, {ty})] = &[\n",
            self.dir.display()
        );
        for (rel, abs) in files {
            out.push_str(&format!("    ({rel:?}, {include}({:?})),\n", abs.to_string_lossy()));
        }
        out.push_str("];\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own, under the system's temporary one. There is no
    /// `tempfile` here on purpose: this crate has no dependencies, and a build helper that
    /// brings a dependency tree along is half of what it is here to avoid.
    fn scratch(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rubevy-build-{}-{what}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a directory to work in");
        dir
    }

    fn put(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn it_takes_the_files_it_was_asked_for_deepest_and_all() {
        let root = scratch("collect");
        put(&root, "ruby/a.rb", "a");
        put(&root, "ruby/lib/b.rb", "b");
        put(&root, "ruby/lib/deep/c.rb", "c");
        put(&root, "ruby/notes.txt", "not ruby");
        put(&root, "elsewhere/d.rb", "not under the directory");

        let files = Embed::new("ruby").collect(&root);
        let names: Vec<&str> = files.iter().map(|(rel, _)| rel.as_str()).collect();
        assert_eq!(names, ["ruby/a.rb", "ruby/lib/b.rb", "ruby/lib/deep/c.rb"]);

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_missing_directory_is_an_empty_table_and_not_a_panic() {
        let root = scratch("missing");
        assert!(Embed::new("ruby").collect(&root).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_table_is_what_the_other_side_includes() {
        let files = vec![(String::from("ruby/a.rb"), PathBuf::from("/tmp/p/ruby/a.rb"))];
        let text = Embed::new("ruby").render(&files);
        assert!(text.contains("pub static RUBY_FILES: &[(&str, &str)] = &[\n"));
        assert!(text.contains("    (\"ruby/a.rb\", include_str!(\"/tmp/p/ruby/a.rb\")),\n"));
        assert!(text.ends_with("];\n"));

        // a table of compiled files is bytes, and needs no compiler at the other end
        let text = Embed::new("ruby").extensions(&["mrb"]).as_bytes().name("MRB_FILES").render(&files);
        assert!(text.contains("pub static MRB_FILES: &[(&str, &[u8])] = &[\n"));
        assert!(text.contains("include_bytes!(\"/tmp/p/ruby/a.rb\")"));
    }

    #[test]
    fn extensions_may_be_written_with_or_without_the_dot() {
        let root = scratch("exts");
        put(&root, "ruby/a.mrb", "");
        put(&root, "ruby/a.rb", "");
        let only_mrb = Embed::new("ruby").extensions(&[".mrb"]).collect(&root);
        assert_eq!(only_mrb.len(), 1);
        assert!(only_mrb[0].0.ends_with(".mrb"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
