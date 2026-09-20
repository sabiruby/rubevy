//! A [`Host`](sabiruby::Host) whose files are in the binary, so that `require` works where there
//! is no filesystem.
//!
//! rubevy's own host reads `require` from the asset directory (`std::fs`), which a browser does
//! not have: a page is served one wasm module and whatever it fetches, and nothing there is a
//! path. Both sample games answered that by putting their `.rb` files into the binary at build
//! time and reading them out of a table — and then could not `require` at all, because the table
//! was theirs and the VM only ever asks its `Host`. This is that table behind the `Host`.
//!
//! The table itself is the build script's to write; `rubevy-build` (beside this crate, std only,
//! so that a build script does not build Bevy a second time) writes exactly the shape these
//! functions take.

use bevy::log::warn;
use sabiruby::{EvalOptions, Host};

/// What turns Ruby source into a RITE binary, for an [`EmbeddedHost`] that serves `.rb` files.
///
/// It is the shape of [`Host::compile`] itself, because that is who calls it: the VM asks its
/// host to compile, and this hands the ask on. A browser's is a call into the page
/// (`window.compile(source)`, a `#[wasm_bindgen]` binding), which can use the source and nothing
/// else; a native one is usually `sabiruby_compiler`. What a caller cannot honour — the options'
/// `line` and `scopes`, which only `eval` and `Binding#eval` ever set — it may ignore, and then
/// `require` still works and `eval` of a string that reads its caller's locals does not.
pub type CompileFn = Box<dyn FnMut(&[u8], &EvalOptions) -> Result<Vec<u8>, String> + Send + Sync>;

/// A [`Host`](sabiruby::Host) that reads `require` out of tables built into the binary.
///
/// Install it on the VM at `Startup`, in place of the one the plugin put there:
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use rubevy::{EmbeddedHost, ScriptWorld};
/// // what the build script wrote: `include!(concat!(env!("OUT_DIR"), "/ruby_files.rs"))`
/// # static RUBY_FILES: &[(&str, &str)] = &[];
/// fn embed_the_scripts(mut world: ResMut<ScriptWorld>) {
///     // the host and where a `require "helper"` is looked for, which are one act:
///     // `ScriptWorld::require_from`
///     world.require_from(EmbeddedHost::new(RUBY_FILES), &["ruby"]);
/// }
/// # fn build(app: &mut App) {
/// app.add_systems(Startup, embed_the_scripts);
/// # }
/// ```
///
/// **The paths in the table are the paths a script requires.** The VM joins a load path and the
/// name a script wrote (`"ruby"` + `"helper"` + `".rb"`), and what comes out is looked up in the
/// table by exact text — so the table's keys are relative paths with `/` in them, which is what
/// `rubevy-build` writes (`"ruby/lib/helper.rb"`). A leading `./` is taken off first, since a
/// script that writes `require "./helper.rb"` means the same file.
///
/// That is why it goes on through [`ScriptWorld::require_from`](crate::ScriptWorld::require_from)
/// and not through `Vm::set_host` alone: a host installed without a load path to match it is
/// asked for the paths the *plugin's* host reads (`assets/scripts/helper.rb`), finds nothing, and
/// the script gets a `LoadError` for a file the game can see is in the binary. Where that bites
/// is a browser, because a browser is the reason this type exists at all. The host says so once
/// if it happens: an ask it could never answer — a path whose first directory is not one the
/// table uses — is a `warn!` naming both, the once.
///
/// A `.mrb` is served from [`EmbeddedHost::with_binaries`] and needs no compiler at all: the VM
/// reads the RITE magic and runs the bytes. A `.rb` is source, so it needs one, and a build with
/// no `ruby-source` feature and no [`EmbeddedHost::compile_with`] answers a `require` of one with
/// the same message the filesystem host does.
pub struct EmbeddedHost {
    text: &'static [(&'static str, &'static str)],
    binary: &'static [(&'static str, &'static [u8])],
    compile: Option<CompileFn>,
    /// Whether the load-path hint has been said. It is said once per host and never again: the
    /// second `require` of a badly pointed load path is not news, and a host that is asked for
    /// something it does not have is an ordinary `LoadError` the rest of the time.
    said: bool,
}

impl EmbeddedHost {
    /// A host serving Ruby **source** from `text`: pairs of a path and the file's text, as
    /// `include_str!` gives them.
    pub fn new(text: &'static [(&'static str, &'static str)]) -> EmbeddedHost {
        EmbeddedHost { text, binary: &[], compile: None, said: false }
    }

    /// Also serve compiled files: pairs of a path and a `.mrb`'s bytes, as `include_bytes!` gives
    /// them. They are looked in before the source table, since a file that is already bytecode
    /// asks nothing of a compiler.
    pub fn with_binaries(
        mut self,
        binary: &'static [(&'static str, &'static [u8])],
    ) -> EmbeddedHost {
        self.binary = binary;
        self
    }

    /// What compiles a `.rb` this host served (and what `eval` compiles through). Without it the
    /// crate's own answer is used: the reference compiler in a build with the `ruby-source`
    /// feature, and an error saying so in a build without it.
    ///
    /// This is how a browser gets a compiler at all — there is no C build in a page, and the one
    /// the playground publishes is a function the page defines.
    pub fn compile_with(
        mut self,
        compile: impl FnMut(&[u8], &EvalOptions) -> Result<Vec<u8>, String> + Send + Sync + 'static,
    ) -> EmbeddedHost {
        self.compile = Some(Box::new(compile));
        self
    }

    /// How many files this host can serve, for a panel or a test.
    pub fn len(&self) -> usize {
        self.text.len() + self.binary.len()
    }

    /// Whether it can serve none at all.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The bytes of `path`, from the compiled table first and the source table second.
    fn find(&self, path: &str) -> Option<Vec<u8>> {
        let want = strip_here(path);
        if let Some((_, bytes)) = self.binary.iter().find(|(p, _)| *p == want) {
            return Some(bytes.to_vec());
        }
        self.text.iter().find(|(p, _)| *p == want).map(|(_, src)| src.as_bytes().to_vec())
    }

    /// Says, once, when the VM asks for something this host could not have answered whatever it
    /// held — a path whose first directory is not one of the table's.
    ///
    /// **Why that and not "the file is missing".** A miss is the ordinary way `require` works: it
    /// tries each load path in turn and each extension in turn, so a `require "helper"` that
    /// succeeds under `"ruby"` has already missed `ruby/helper.mrb`, and one under a second load
    /// path has missed the first. A host cannot tell a miss on the way to a hit from the last
    /// miss of all — `require` does not tell it which candidate is the last — so warning on a
    /// miss would cry wolf on every working game. The first *directory*, though, is not about
    /// which candidate this is: a load path entry that reaches outside the table can never hit
    /// under any name, which is exactly the state that being handed the plugin's load path
    /// leaves the VM in.
    ///
    /// The other end of the door is [`ScriptWorld::require_from`](crate::ScriptWorld::require_from),
    /// which is why it is what the message names. rubevy cannot see a `Vm::set_host` — the VM
    /// keeps no getter for its host or its load path — so the host itself is the only place
    /// standing where this is visible.
    fn note_a_miss(&mut self, path: &str) {
        if self.said {
            return;
        }
        let want = strip_here(path);
        let asked = directory_of(want);
        let mut roots: Vec<&str> =
            self.binary.iter().map(|(p, _)| directory_of(p)).chain(self.text.iter().map(|(p, _)| directory_of(p))).collect();
        roots.sort_unstable();
        roots.dedup();
        if roots.contains(&asked) {
            return;
        }
        self.said = true;
        let shown = |d: &str| if d.is_empty() { String::from("(no directory)") } else { format!("{d}/") };
        let held: Vec<String> = roots.iter().map(|d| shown(d)).collect();
        warn!(
            "rubevy: an EmbeddedHost was asked for {want:?}, and it has nothing under {} — its \
             {} files are under {}. If a `require` is failing, the VM's load path is still \
             pointing where this host cannot reach: `ScriptWorld::require_from(host, &[{}])` \
             sets the host and the load path together. (said once)",
            shown(asked),
            self.len(),
            held.join(", "),
            roots.iter().map(|d| format!("{d:?}")).collect::<Vec<_>>().join(", ")
        );
    }
}

/// `./helper.rb` and `helper.rb` are the same file: a name that says "here" does not go through
/// the load path at all, so it reaches a host as the script wrote it.
fn strip_here(path: &str) -> &str {
    path.strip_prefix("./").unwrap_or(path)
}

/// The first directory of a path, and `""` for a file with no directory at all — which is what a
/// table built from the crate root, or a load path entry of `""`, gives.
fn directory_of(path: &str) -> &str {
    match path.split_once('/') {
        Some((head, _)) => head,
        None => "",
    }
}

impl std::fmt::Debug for EmbeddedHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedHost")
            .field("source_files", &self.text.len())
            .field("compiled_files", &self.binary.len())
            .field("compiler", &if self.compile.is_some() { "given" } else { "the crate's own" })
            .finish()
    }
}

impl Host for EmbeddedHost {
    fn compile(&mut self, src: &[u8], opts: &EvalOptions) -> Result<Vec<u8>, String> {
        match &mut self.compile {
            Some(f) => f(src, opts),
            None => crate::reference_compile(src, opts),
        }
    }

    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        let found = self.find(path);
        if found.is_none() {
            self.note_a_miss(path);
        }
        found
    }

    fn file_exists(&mut self, path: &str) -> bool {
        let found = self.find(path).is_some();
        if !found {
            self.note_a_miss(path);
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static FILES: &[(&str, &str)] = &[("ruby/helper.rb", ""), ("ruby/lib/deep.rb", "")];

    /// The misses `require` makes on its way to a hit say nothing. It tries `.mrb` before `.rb`
    /// and each load path in turn, so a working game misses several times per `require`; a host
    /// that spoke up on a miss would speak up in every working game.
    #[test]
    fn the_misses_of_an_ordinary_require_say_nothing() {
        let mut host = EmbeddedHost::new(FILES);
        assert!(!host.file_exists("ruby/helper.mrb"), "the .mrb tried before the .rb");
        assert!(host.file_exists("ruby/helper.rb"));
        assert!(!host.file_exists("ruby/deep.rb"), "the first load path, before ruby/lib");
        assert!(host.file_exists("ruby/lib/deep.rb"));
        assert!(!host.file_exists("ruby/nowhere.rb"), "a file that is really not there");
        assert!(!host.said, "none of that is worth a word");
    }

    /// The ask this host could not have answered whatever it held: the plugin's load path, left
    /// in place. It is said, and said once.
    #[test]
    fn an_ask_from_outside_the_table_is_said_once() {
        let mut host = EmbeddedHost::new(FILES);
        assert!(!host.file_exists("assets/scripts/helper.rb"));
        assert!(host.said, "a path under `assets/` can never reach a table under `ruby/`");
        // and a second ask of the same kind is not a second word: `note_a_miss` returns on the
        // flag before it looks at the path at all
        assert!(!host.file_exists("assets/helper.rb"));
        assert!(host.said);
    }

    /// A table with no directories at all (a build helper pointed at the crate root) is reached
    /// by a load path of `""`, and that is not an ask from outside.
    #[test]
    fn a_table_at_the_top_is_reached_by_a_bare_name() {
        static FLAT: &[(&str, &str)] = &[("helper.rb", "")];
        let mut host = EmbeddedHost::new(FLAT);
        assert!(!host.file_exists("helper.mrb"));
        assert!(host.file_exists("helper.rb"));
        assert!(!host.said);
        assert!(!host.file_exists("ruby/helper.rb"));
        assert!(host.said, "and a load path of `ruby` cannot reach it");
    }

    #[test]
    fn a_path_is_split_at_its_first_slash() {
        assert_eq!(directory_of("ruby/lib/helper.rb"), "ruby");
        assert_eq!(directory_of("helper.rb"), "");
        assert_eq!(strip_here("./ruby/helper.rb"), "ruby/helper.rb");
        assert_eq!(strip_here("ruby/helper.rb"), "ruby/helper.rb");
    }
}
