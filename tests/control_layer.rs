//! `rubevy::layers::CONTROL`: `on(:event) { |payload| … }` in a script, one task per handler.
//!
//! What it checks: a handler hears what the game publishes, with the payload filling its block's
//! arguments; a handler may wait (`Rubevy.ask(...).pop`) inside the block — which is
//! `instance_exec` on the script's stage, a place a task could not wait before SabiRuby 0.7 —
//! and `self` there is one object per script; a handler that raises goes on hearing; and the
//! handler tasks end when the script is stopped. And where a handler that raises is logged, the
//! line is the author's: the game's prelude taken off the script's own file, a `require`d file's
//! line as it stands — the rule `ScriptEnded::at` follows.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{stop_script, Answer, MrbAsset, Program, Request, RubevyPlugin, Script, ScriptWorld};

#[derive(Resource, Default)]
struct Seen(Vec<Request>);

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// Every question is answered: `double` with twice its number, everything else with nil.
fn answer(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>) {
    for r in world.take_requests() {
        match r.kind.as_str() {
            "double" => {
                let n = r.num(0).unwrap_or(0.0);
                world.answer(&r, Answer::Num(n * 2.0));
            }
            _ => world.answer(&r, Answer::Nil),
        }
        seen.0.push(r);
    }
}

fn add_the_control_layer(mut world: ResMut<ScriptWorld>) {
    world.load_and_run(rubevy::layers::CONTROL).expect("the layer runs");
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Seen>()
    .add_systems(Startup, add_the_control_layer)
    .add_systems(Update, answer);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

fn spawn_script(app: &mut App, src: &str) -> Entity {
    let h = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
    app.world_mut().spawn(Script::new(h)).id()
}

fn publish(app: &mut App, entity: Option<Entity>, name: &str, payload: Answer) {
    app.world_mut().resource_mut::<ScriptWorld>().publish(entity, name, payload);
}

fn seen(app: &App, kind: &str) -> Vec<Request> {
    app.world().resource::<Seen>().0.iter().filter(|r| r.kind == kind).cloned().collect()
}

#[test]
fn a_script_without_the_layer_has_no_on() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Seen>()
    .add_systems(Update, answer);
    spawn_script(
        &mut app,
        r#"Rubevy.ask("has", respond_to?(:on, true).to_s, Rubevy.const_defined?(:Control).to_s).pop"#,
    );
    frames(&mut app, 5);
    let r = &seen(&app, "has")[0];
    assert_eq!((r.text(0), r.text(1)), (Some("false"), Some("false")));
}

#[test]
fn a_handler_hears_waits_and_keeps_its_state() {
    let mut app = app();
    let e = spawn_script(
        &mut app,
        r#"
          on(:scored) do |points, by|           # a list payload fills both
            twice = Rubevy.ask("double", points).pop    # a wait inside the handler
            @total = (@total || 0) + twice
            Rubevy.ask("total", @total, by).pop
          end
          on(:report) { |_| Rubevy.ask("report", @total, self.class.to_s).pop }
          Rubevy::Control.run
        "#,
    );
    frames(&mut app, 3);
    publish(&mut app, Some(e), "scored", Answer::List(vec![3.0, 1.0]));
    frames(&mut app, 3);
    publish(&mut app, Some(e), "scored", Answer::List(vec![4.0, 2.0]));
    frames(&mut app, 3);
    publish(&mut app, Some(e), "report", Answer::Nil);
    frames(&mut app, 3);

    let totals: Vec<(Option<f64>, Option<f64>)> = seen(&app, "total").iter().map(|r| (r.num(0), r.num(1))).collect();
    assert_eq!(totals, vec![(Some(6.0), Some(1.0)), (Some(14.0), Some(2.0))]);
    let report = &seen(&app, "report")[0];
    assert_eq!(report.num(0), Some(14.0), "the other handler reads the same @total");
    assert_eq!(report.text(1), Some("Rubevy::Control"));
}

#[test]
fn two_scripts_have_two_stages() {
    let mut app = app();
    let src = r#"
      on(:add) { |n| @sum = (@sum || 0) + n; Rubevy.ask("sum", @sum).pop }
      Rubevy::Control.run
    "#;
    let a = spawn_script(&mut app, src);
    let b = spawn_script(&mut app, src);
    frames(&mut app, 3);
    publish(&mut app, Some(a), "add", Answer::Num(1.0));
    publish(&mut app, Some(b), "add", Answer::Num(10.0));
    frames(&mut app, 3);
    publish(&mut app, Some(a), "add", Answer::Num(1.0));
    frames(&mut app, 3);
    let mut sums: Vec<f64> = seen(&app, "sum").iter().filter_map(|r| r.num(0)).collect();
    sums.sort_by(f64::total_cmp);
    assert_eq!(sums, vec![1.0, 2.0, 10.0]);
}

#[test]
fn a_handler_that_raises_goes_on_hearing() {
    let mut app = app();
    let e = spawn_script(
        &mut app,
        r#"
          on(:n) do |n|
            raise "odd" if n.to_i.odd?
            Rubevy.ask("even", n).pop
          end
          Rubevy::Control.run
        "#,
    );
    frames(&mut app, 3);
    for n in [1.0, 2.0, 3.0, 4.0] {
        publish(&mut app, Some(e), "n", Answer::Num(n));
        frames(&mut app, 2);
    }
    let evens: Vec<f64> = seen(&app, "even").iter().filter_map(|r| r.num(0)).collect();
    assert_eq!(evens, vec![2.0, 4.0]);
}

#[test]
fn the_handlers_end_with_the_script() {
    let mut app = app();
    let e = spawn_script(
        &mut app,
        r#"
          on(:ping) { |_| Rubevy.ask("pong").pop }
          Rubevy::Control.run
        "#,
    );
    frames(&mut app, 3);
    let before = app.world().resource::<ScriptWorld>().vm.gc_registered.len();
    assert!(before > 0);
    {
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, app.world());
        stop_script(&mut commands, e);
        queue.apply(app.world_mut());
    }
    frames(&mut app, 4);
    assert_eq!(app.world().resource::<ScriptWorld>().subscriptions(), 0);
    publish(&mut app, Some(e), "ping", Answer::Nil);
    frames(&mut app, 3);
    assert!(seen(&app, "pong").is_empty(), "no handler is left to hear it");
    assert_eq!(app.world().resource::<ScriptWorld>().vm.gc_registered.len(), 0, "nothing held");
}

// ----------------------------------------------- the line a handler's exception is logged at

/// A game's prelude that sends `Rubevy.log` back as a question, so that the test can read what the
/// layer logged, and defines a method that raises — two lines of the prelude, then the method's
/// three. The layer calls `Rubevy.log` by name, so this is what it reaches.
const LOGGING_PRELUDE: &str = "module Rubevy\n  def self.log(text); ask('log', text).pop; end\nend\n\
                               def prelude_raises\n  raise 'in the prelude'\nend\n";

/// With a line table: without one a backtrace is empty and there is no line to draw.
fn compile_with_lines(src: &str, name: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: name.into(), debug_info: true, ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// Runs `program` as a script with its prelude's length, publishes `n` once, and answers the
/// lines the layer logged.
fn logged(app: &mut App, program: &Program) -> Vec<String> {
    let h = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile_with_lines(&program.source, &program.name));
    let e = app.world_mut().spawn(Script::new(h).with_prelude_lines(program.prelude_lines)).id();
    frames(app, 3);
    publish(app, Some(e), "n", Answer::Num(1.0));
    frames(app, 3);
    seen(app, "log").iter().filter_map(|r| r.text(0).map(String::from)).collect()
}

/// The handler's `raise` on line 2 of the author's file is logged at `player.rb:2`, not at the
/// line it has in the program with the prelude in front; a raise inside a method the prelude
/// defines is logged at the author's line that called it.
#[test]
fn a_handler_error_is_logged_at_the_authors_line() {
    let mut app = app();
    let program = Program::new(
        LOGGING_PRELUDE,
        "player.rb",
        "on(:n) do |n|\n  raise 'odd' if n.to_i.odd?\nend\non(:n) do |n|\n  prelude_raises\nend\n\
         Rubevy::Control.run\n",
        "",
    );
    assert!(program.prelude_lines > 0, "there is a prelude to take off");
    let mut lines = logged(&mut app, &program);
    lines.sort();
    assert_eq!(
        lines,
        vec![
            String::from("on(:n): RuntimeError: in the prelude (player.rb:5)"),
            String::from("on(:n): RuntimeError: odd (player.rb:2)"),
        ]
    );
}

/// A helper the script `require`s, compiled with a line table and served out of the binary; it
/// raises on its line 12, past the prelude's length, so a prelude taken off it would show.
fn helper_host() -> rubevy::EmbeddedHost {
    let source = format!("{}def helper_go\n  raise 'from the helper'\nend\n", "# padding\n".repeat(10));
    let opts = sabiruby_compiler::Options { filename: "helper.rb".into(), debug_info: true, ..Default::default() };
    let bytes = sabiruby_compiler::compile(source.as_bytes(), &opts).expect("compiles");
    let table: &'static [(&'static str, &'static [u8])] =
        Box::leak(vec![("ruby/helper.mrb", &*Box::leak(bytes.into_boxed_slice()))].into_boxed_slice());
    rubevy::EmbeddedHost::new(&[]).with_binaries(table)
}

/// A raise inside a file the script `require`d is logged at that file's own line: the prelude was
/// put in front of the script, not in front of the helper.
#[test]
fn a_handler_error_in_a_required_file_keeps_its_line() {
    let mut app = app();
    app.world_mut().resource_mut::<ScriptWorld>().require_from(helper_host(), &["ruby"]);
    let program = Program::new(
        LOGGING_PRELUDE,
        "player.rb",
        "require 'helper'\non(:n) { |_| helper_go }\nRubevy::Control.run\n",
        "",
    );
    assert!(program.prelude_lines > 0 && program.prelude_lines < 12);
    assert_eq!(logged(&mut app, &program), vec![String::from("on(:n): RuntimeError: from the helper (helper.rb:12)")]);
}
