//! **Where a script stopped** (`ScriptEnded::at`): the innermost frame of the exception's
//! backtrace that is past the prelude, in the lines the author of the file can see.
//!
//! Every game that lets somebody write Ruby needs this, and until now every one of them wrote it
//! itself: a task that has ended keeps no frames — `ScriptWorld::stats` answers `frames: []` and
//! `location: None` at the very moment the ending arrives — so rubevy_games' Factory put a
//! `rescue` in its own prelude that read `error.backtrace.first` while the exception still had
//! one, worked the prelude's length off the line, and left the answer on the task in an instance
//! variable for the game to read back (about thirty lines across two files). The exception itself
//! carries the frames, though, and that is what these tests are about.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{MrbAsset, Program, RubevyPlugin, Script, ScriptEnded, ScriptStatus, ScriptWorld};

/// One ending, as this test reads it: what it was, what it said, and where it stopped.
type Ending = (ScriptStatus, String, Option<(String, u32)>);

/// The endings the app sent, in order.
#[derive(Resource, Default)]
struct Endings(Vec<Ending>);

fn collect(mut ended: MessageReader<ScriptEnded>, mut endings: ResMut<Endings>) {
    for e in ended.read() {
        endings.0.push((e.status, e.value.clone(), e.at.clone()));
    }
}

/// With a line table, which is what a game that means to show its players where they went wrong
/// compiles with — and which is **not** the default of either compiler (`debug_info: false`).
fn compile(src: &str, name: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options {
        filename: name.into(),
        debug_info: true,
        ..Default::default()
    };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Endings>()
    .add_systems(Update, collect);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

/// Runs until one ending has arrived, and answers it.
fn ending(app: &mut App, at_most: usize) -> Ending {
    for _ in 0..at_most {
        if let Some(e) = app.world().resource::<Endings>().0.first() {
            return e.clone();
        }
        frames(app, 1);
    }
    panic!("no ending after {at_most} frames");
}

/// A plain script, no prelude: the line is the line.
#[test]
fn the_line_a_script_raised_on() {
    let mut app = app();
    let source = "a = 1\nb = 2\nraise 'boom'\n";
    let h = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(source, "brain.rb"));
    app.world_mut().spawn(Script::new(h));
    let (status, value, at) = ending(&mut app, 30);
    assert_eq!(status, ScriptStatus::Failed);
    assert!(value.contains("boom"), "{value}");
    assert_eq!(at, Some((String::from("brain.rb"), 3)));
}

/// **A prelude in front of the author's file**, which is the shape both sample games compile: the
/// line reported is the author's own, not the compiled program's.
#[test]
fn a_prelude_is_taken_off_the_line() {
    let mut app = app();
    let prelude = "class Robot\n  def go\n    yield\n  end\nend\n";
    let players = "Robot.new.go do\n  raise 'the player wrote this'\nend\n";
    let program = Program::new(prelude, "player.rb", players, "");
    // the author's `raise` is on their line 2, which is line `prelude_lines + 2` of the program.
    // Seven, not six: `Program::new` puts a blank line and the separator comment between the two
    // files, and counts the text it really put in front (`Program::prelude_lines`)
    assert_eq!(program.prelude_lines, 7);
    let h = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile(&program.source, &program.name));
    app.world_mut().spawn(
        Script::new(h).with_name(&program.name).with_prelude_lines(program.prelude_lines),
    );
    let (_, value, at) = ending(&mut app, 30);
    assert!(value.contains("the player wrote this"), "{value}");
    assert_eq!(at, Some((String::from("player.rb"), 2)));
}

/// **Raised inside the prelude, called from the author's file.** The innermost frame is the
/// DSL's; the one that is reported is the author's line that called it, because that is the line
/// the author can do something about.
#[test]
fn a_raise_inside_the_prelude_is_reported_at_the_line_that_called_it() {
    let mut app = app();
    let prelude = "class Robot\n  def go(n)\n    raise 'the DSL refuses' if n > 3\n  end\nend\n";
    let players = "r = Robot.new\nr.go 1\nr.go 9\n";
    let program = Program::new(prelude, "player.rb", players, "");
    let h = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile(&program.source, &program.name));
    app.world_mut().spawn(Script::new(h).with_prelude_lines(program.prelude_lines));
    let (_, value, at) = ending(&mut app, 30);
    assert!(value.contains("the DSL refuses"), "{value}");
    assert_eq!(at, Some((String::from("player.rb"), 3)), "the author's line 3 is `r.go 9`");
}

/// **Nothing but the prelude raised**: the author's file was never reached, so there is no line
/// of theirs to point at. What went wrong is still in `value`.
#[test]
fn a_prelude_that_raises_before_the_authors_file_says_no_line() {
    let mut app = app();
    let prelude = "raise 'this file defines no robot'\n";
    let program = Program::new(prelude, "player.rb", "# nothing here\n", "");
    let h = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile(&program.source, &program.name));
    app.world_mut().spawn(Script::new(h).with_prelude_lines(program.prelude_lines));
    let (status, value, at) = ending(&mut app, 30);
    assert_eq!(status, ScriptStatus::Failed);
    assert!(value.contains("defines no robot"), "{value}");
    assert_eq!(at, None);
}

/// A script that ran to its end has no place: what it ended with is a value, not an exception.
#[test]
fn a_script_that_finished_has_no_place() {
    let mut app = app();
    let h = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile("1 + 1\n", "brain.rb"));
    app.world_mut().spawn(Script::new(h));
    let (status, _, at) = ending(&mut app, 30);
    assert_eq!(status, ScriptStatus::Finished);
    assert_eq!(at, None);
}

/// **No debug information, no place.** It is the default of both the compilers rubevy's callers
/// use, and a program built without a line table has an empty backtrace: the ending still says
/// what went wrong and says nothing it cannot know about where.
#[test]
fn without_a_line_table_there_is_no_place() {
    let mut app = app();
    let opts = sabiruby_compiler::Options { filename: "brain.rb".into(), ..Default::default() };
    assert!(!opts.debug_info, "the default this test is about");
    let bytes = sabiruby_compiler::compile(b"raise 'boom'\n", &opts).expect("compiles");
    let h = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(MrbAsset { bytes });
    app.world_mut().spawn(Script::new(h));
    let (status, value, at) = ending(&mut app, 30);
    assert_eq!(status, ScriptStatus::Failed);
    assert!(value.contains("boom"), "{value}");
    assert_eq!(at, None);
}

/// **A `Task::Overrun` has a place too**, which is the one a game could not reach for itself: it
/// is an `Exception` and not a `StandardError`, so a `rescue => e` in a game's own prelude never
/// saw it (Factory reported `inserter.rb:?` for exactly this case).
#[test]
fn an_overrun_says_where_it_was_cut_off() {
    let mut app = app();
    let source = "a = (1..200000).to_a\na.sort { |x, y| y <=> x }\n";
    let h = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(source, "brain.rb"));
    app.world_mut().spawn(Script::new(h));
    let (status, value, at) = ending(&mut app, 200);
    assert_eq!(status, ScriptStatus::Failed);
    assert!(value.contains("Overrun"), "{value}");
    assert_eq!(at, Some((String::from("brain.rb"), 2)), "the `sort` it was cut off inside");
}

/// A script that never started — a `.mrb` that will not load — has no lines at all.
#[test]
fn a_script_that_never_started_has_no_place() {
    let mut app = app();
    let h = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(MrbAsset { bytes: b"this is not a .mrb".to_vec() });
    app.world_mut().spawn(Script::new(h));
    let (status, _, at) = ending(&mut app, 30);
    assert_eq!(status, ScriptStatus::Failed);
    assert_eq!(at, None);
}

/// The whole road once more, as a game writes it: the `Program` it built the script from hands
/// the script its `prelude_lines`, and the ending says a line the player can find in the file
/// they are editing.
#[test]
fn the_shape_a_game_writes() {
    let mut app = app();
    let prelude = "def robot(&b)\n  $robot = b\nend\ndef run_robot\n  $robot.call\nend\n";
    let players = "robot do\n  x = 1\n  x.no_such_method\nend\n";
    let program = Program::new(prelude, "player.rb", players, "run_robot");
    let h = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile(&program.source, &program.name));
    app.world_mut().spawn(
        Script::new(h).with_name(&program.name).with_prelude_lines(program.prelude_lines),
    );
    let (status, value, at) = ending(&mut app, 30);
    assert_eq!(status, ScriptStatus::Failed);
    assert!(value.contains("no_such_method"), "{value}");
    assert_eq!(at, Some((String::from("player.rb"), 3)));
    // and nothing of this needed the VM afterwards
    assert!(app.world().resource::<ScriptWorld>().broken_programs() == 0);
}

// --------------------------------------------- the same lines while it runs (`ScriptWorld::stats`)

/// Spawns `program` with its prelude and runs until its task is parked, then answers what
/// `ScriptWorld::stats` says of it.
fn stats_of(app: &mut App, program: &Program) -> rubevy::ScriptStats {
    let h = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile(&program.source, &program.name));
    let e = app
        .world_mut()
        .spawn(Script::new(h).with_name(&program.name).with_prelude_lines(program.prelude_lines))
        .id();
    frames(app, 4);
    let task = *app.world().entity(e).get::<rubevy::ScriptTask>().expect("started");
    app.world().resource::<ScriptWorld>().stats(&task)
}

/// **Parked inside a method the prelude defines** — the DSL's `scan`, which is where a script of a
/// game that wraps its questions spends most of its life. `stats` answers the author's line that
/// called it, in the author's numbers, the way `ScriptEnded::at` does for a script that raised;
/// the prelude's own frame is not in `frames`, because its line is not one the author can find.
#[test]
fn stats_answers_in_the_authors_lines() {
    let mut app = app();
    let prelude = "def scan\n  Rubevy.ask('scan').pop\nend\n";
    let players = "a = 1\nscan\n";
    let program = Program::new(prelude, "player.rb", players, "");
    let stats = stats_of(&mut app, &program);
    assert_eq!(stats.location, Some((String::from("player.rb"), 2)), "the author's `scan`");
    assert_eq!(stats.frames, vec![(String::from("player.rb"), 2)], "and not the prelude's line 2");
}

/// Parked in the author's own lines, the location is that line — moved by the prelude, not the
/// compiled program's.
#[test]
fn stats_takes_the_prelude_off_a_line_of_the_authors_own() {
    let mut app = app();
    let prelude = "X = 1\nY = 2\n";
    let players = "a = 1\nb = 2\nRubevy.ask('here').pop\n";
    let program = Program::new(prelude, "player.rb", players, "");
    assert!(program.prelude_lines > 0);
    let stats = stats_of(&mut app, &program);
    assert_eq!(stats.location, Some((String::from("player.rb"), 3)));
}

/// With no prelude, `stats` is what the VM says: nothing taken off, nothing left out.
#[test]
fn stats_without_a_prelude_is_the_vms_own() {
    let mut app = app();
    let program = Program::new("", "brain.rb", "def wait\n  Rubevy.ask('w').pop\nend\nwait\n", "");
    let h = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile(&program.source, &program.name));
    let e = app.world_mut().spawn(Script::new(h)).id();
    frames(&mut app, 4);
    let task = *app.world().entity(e).get::<rubevy::ScriptTask>().expect("started");
    let world = app.world().resource::<ScriptWorld>();
    let stats = world.stats(&task);
    assert_eq!(stats.frames, world.vm.task_frames(task.task()));
    assert_eq!(stats.location, world.vm.task_location(task.task()));
}
