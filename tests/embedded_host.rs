//! `require` out of the binary, which is the only `require` a browser can have.
//!
//! A page has no filesystem, so the host the plugin installs — which reads the asset directory
//! with `std::fs` — answers nothing there. [`EmbeddedHost`] is the same road with a table in
//! place of the directory: the files are put into the binary by the build script
//! (`rubevy-build`), and the VM asks for them by the same paths it would have asked the
//! filesystem for.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, EmbeddedHost, MrbAsset, RubevyPlugin, Script, ScriptWorld};

/// Ruby source in the binary, as `include_str!` would give it: the paths are relative to the
/// crate, with `/` in them, which is what the build helper writes.
static SOURCE_FILES: &[(&str, &str)] = &[
    ("ruby/helper.rb", "def helper_answer\n  42\nend\n"),
    ("ruby/lib/deep.rb", "require \"helper\"\ndef deep_answer\n  helper_answer + 1\nend\n"),
];

/// A compiled file in the binary, as `include_bytes!` gives it. This one is the crate's own
/// `assets/scripts/helper.mrb` (`tools/compile_scripts.sh` built it with the reference `mrbc`),
/// so it is real bytecode and not something this test made.
static COMPILED_FILES: &[(&str, &[u8])] =
    &[("ruby/helper.mrb", include_bytes!("../assets/scripts/helper.mrb"))];

/// What the scripts said, in order.
#[derive(Resource, Default)]
struct Said(Vec<String>);

fn answer_all(mut world: ResMut<ScriptWorld>, mut said: ResMut<Said>) {
    for r in world.take_requests() {
        if r.kind == "said" {
            said.0.push(r.text(0).unwrap_or("").to_string());
        }
        world.answer(&r, Answer::Nil);
    }
}

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// The compiler a browser would hand the host: it is given a source and the options, and — like
/// the playground's bridge, which is handed a source and nothing else — it uses the source.
fn a_compiler() -> impl FnMut(&[u8], &sabiruby::EvalOptions) -> Result<Vec<u8>, String> + Send + Sync
{
    |src: &[u8], opts: &sabiruby::EvalOptions| {
        let o = sabiruby_compiler::Options {
            filename: opts.filename.into(),
            debug_info: opts.debug_info,
            ..Default::default()
        };
        sabiruby_compiler::compile(src, &o).map_err(|e| e.to_string())
    }
}

/// Runs one script against one host and answers what it said.
fn run(host: EmbeddedHost, script: &str) -> Vec<String> {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Said>()
    .add_systems(Update, answer_all);
    {
        let mut world = app.world_mut().resource_mut::<ScriptWorld>();
        world.vm.set_host(Box::new(host));
        // where a `require` looks, exactly as it would on a filesystem — the table's keys are
        // those paths
        world.vm.set_load_path(&["ruby", "ruby/lib"]);
    }
    let handle = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(script));
    app.world_mut().spawn(Script::new(handle));
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
    std::mem::take(&mut app.world_mut().resource_mut::<Said>().0)
}

#[test]
fn a_script_requires_a_file_out_of_the_binary() {
    let host = EmbeddedHost::new(SOURCE_FILES).compile_with(a_compiler());
    assert_eq!(host.len(), 2);
    let said = run(host, r#"require "helper"; Rubevy.ask("said", helper_answer.to_s).pop"#);
    assert_eq!(said, vec!["42"], "the required file's method is there");
}

/// A required file may require another, and the second is looked for the same way.
#[test]
fn a_required_file_may_require_another() {
    let host = EmbeddedHost::new(SOURCE_FILES).compile_with(a_compiler());
    let said = run(host, r#"require "deep"; Rubevy.ask("said", deep_answer.to_s).pop"#);
    assert_eq!(said, vec!["43"]);
}

/// **Bytecode needs no compiler**: the VM reads the RITE magic and runs the bytes, so a build
/// with no `ruby-source` feature and a host with no compiler of its own can still `require` a
/// `.mrb`. `require "helper"` is tried as `.mrb` before `.rb` (SabiRuby's `require.rb`), which
/// is why this finds the compiled table and not the source one.
#[test]
fn a_compiled_file_needs_no_compiler() {
    let host = EmbeddedHost::new(SOURCE_FILES).with_binaries(COMPILED_FILES);
    let said = run(host, r#"require "helper"; Rubevy.ask("said", Helper.greet("x")).pop"#);
    assert_eq!(said, vec!["hello, x"], "the .mrb ran without anything compiling it");
}

/// A name that says where it lives (`./`) means the same file. SabiRuby looks such a name up
/// with no load path in front of it, so what reaches the host is the path as written.
#[test]
fn a_path_that_says_here_is_the_same_file() {
    let host = EmbeddedHost::new(SOURCE_FILES).compile_with(a_compiler());
    let said = run(host, r#"require "./ruby/helper.rb"; Rubevy.ask("said", helper_answer.to_s).pop"#);
    assert_eq!(said, vec!["42"]);
}

/// A file that is not in the table is a `LoadError`, as a file that is not on a disk is.
#[test]
fn a_file_that_is_not_in_the_binary_is_a_load_error() {
    let host = EmbeddedHost::new(SOURCE_FILES).compile_with(a_compiler());
    let said = run(
        host,
        r#"begin
             require "nowhere"
           rescue LoadError => e
             Rubevy.ask("said", e.message).pop
           end"#,
    );
    assert_eq!(said.len(), 1, "the require raised");
    assert!(said[0].contains("nowhere"), "and said which file: {}", said[0]);
}

/// Without a compiler — no `compile_with`, and a build without the `ruby-source` feature — a
/// `.rb` out of the table is the same refusal the filesystem host gives, so that the two hosts
/// do not disagree about what a build can do.
#[cfg(not(feature = "ruby-source"))]
#[test]
fn source_without_a_compiler_says_what_is_missing() {
    let host = EmbeddedHost::new(SOURCE_FILES);
    let said = run(
        host,
        r#"begin
             require "helper"
           rescue SyntaxError => e
             Rubevy.ask("said", e.message).pop
           end"#,
    );
    assert_eq!(said.len(), 1, "the require raised");
    assert!(said[0].contains("ruby-source"), "and named the feature: {}", said[0]);
}
