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
///     world.vm.set_host(Box::new(EmbeddedHost::new(RUBY_FILES)));
///     // a `require "helper"` is looked for under these, as it is on a filesystem
///     world.vm.set_load_path(&["ruby"]);
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
/// A `.mrb` is served from [`EmbeddedHost::with_binaries`] and needs no compiler at all: the VM
/// reads the RITE magic and runs the bytes. A `.rb` is source, so it needs one, and a build with
/// no `ruby-source` feature and no [`EmbeddedHost::compile_with`] answers a `require` of one with
/// the same message the filesystem host does.
pub struct EmbeddedHost {
    text: &'static [(&'static str, &'static str)],
    binary: &'static [(&'static str, &'static [u8])],
    compile: Option<CompileFn>,
}

impl EmbeddedHost {
    /// A host serving Ruby **source** from `text`: pairs of a path and the file's text, as
    /// `include_str!` gives them.
    pub fn new(text: &'static [(&'static str, &'static str)]) -> EmbeddedHost {
        EmbeddedHost { text, binary: &[], compile: None }
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
        let want = path.strip_prefix("./").unwrap_or(path);
        if let Some((_, bytes)) = self.binary.iter().find(|(p, _)| *p == want) {
            return Some(bytes.to_vec());
        }
        self.text.iter().find(|(p, _)| *p == want).map(|(_, src)| src.as_bytes().to_vec())
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
        self.find(path)
    }

    fn file_exists(&mut self, path: &str) -> bool {
        self.find(path).is_some()
    }
}
