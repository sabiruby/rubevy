//! A `.mrb` that will not load: said once, not once a frame.
//!
//! `start_scripts` used to log a program it could not load and move on, which left the entity
//! exactly as it found it — a `Script` with no `ScriptTask` — so the next frame met the same
//! bytes, parsed them again and wrote the same line again, for as long as the entity lived. A
//! thousand robots of one broken brain were a thousand parses and a thousand lines of log every
//! frame, and the log they were in was the one the game's own messages had to be read out of.
//!
//! What a broken program gets now is the ending every other script gets: one `ScriptEnded` of
//! `ScriptStatus::Failed` carrying what the VM said, and a `ScriptDone` so that the entity is
//! passed over from then on. It is a pause and not a full stop — an asset that changes takes the
//! mark off again, whether it changes under its own handle (a file reloaded, an editor's Apply)
//! or by `replace_script`.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{
    replace_script, Answer, MrbAsset, RubevyPlugin, Script, ScriptDone, ScriptEnded, ScriptStatus,
    ScriptWorld,
};

/// Bytes that are not a RITE binary. Nothing in this test goes through `MrbLoader` (which parses
/// as it reads, and would have refused the file), because what is being tested is the road a game
/// that compiles its own Ruby takes: `Assets::add` with whatever it has, which on a bad day is a
/// truncated download or the output of another version's compiler.
const NOT_BYTECODE: &[u8] = b"this is not a .mrb at all";

/// Other bytes that are also not bytecode, for telling one broken program from two.
const NOR_IS_THIS: &[u8] = b"and neither is this one";

/// The endings the app sent, in order.
#[derive(Resource, Default)]
struct Endings(Vec<(Entity, ScriptStatus, String)>);

fn collect_endings(mut ended: MessageReader<ScriptEnded>, mut endings: ResMut<Endings>) {
    for e in ended.read() {
        endings.0.push((e.entity, e.status, e.value.clone()));
    }
}

/// What the scripts asked, so that a mended script can be seen running.
#[derive(Resource, Default)]
struct Seen(Vec<String>);

fn answer_all(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>) {
    for r in world.take_requests() {
        seen.0.push(r.kind.clone());
        world.answer(&r, Answer::Nil);
    }
}

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn broken(bytes: &[u8]) -> MrbAsset {
    MrbAsset { bytes: bytes.to_vec() }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Endings>()
    .init_resource::<Seen>()
    .add_systems(Update, (answer_all, collect_endings));
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

fn endings(app: &App) -> &Vec<(Entity, ScriptStatus, String)> {
    &app.world().resource::<Endings>().0
}

/// **Twenty frames, one ending.** The number of frames is what makes this a test at all: before,
/// the count of anything said about a broken script was the count of frames it had lived through.
#[test]
fn a_broken_program_is_reported_once_however_long_it_stands() {
    let mut app = app();
    let handle = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(broken(NOT_BYTECODE));
    let entity = app.world_mut().spawn(Script::new(handle)).id();
    frames(&mut app, 20);

    assert_eq!(endings(&app).len(), 1, "said once, not once a frame: {:?}", endings(&app));
    let (who, status, message) = &endings(&app)[0];
    assert_eq!(*who, entity);
    assert_eq!(*status, ScriptStatus::Failed);
    assert!(!message.is_empty(), "and it says what the VM said: {message:?}");
    assert!(
        app.world().entity(entity).contains::<ScriptDone>(),
        "the entity is marked, which is what stops the retrying"
    );
    let world = app.world().resource::<ScriptWorld>();
    assert_eq!(world.broken_programs(), 1);
    assert_eq!(world.loaded_programs(), 0, "nothing of it is in the VM");
}

/// A herd of one broken program is one broken program. The two assets hold the same bytes and are
/// two ids, which is the shape a game that compiles per entity has (`tests/shared_irep.rs`), so
/// what says they are one program is the bytes — the same key the loaded ones are kept under.
#[test]
fn a_herd_of_one_broken_program_parses_it_once() {
    let mut app = app();
    let (a, b) = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        (assets.add(broken(NOT_BYTECODE)), assets.add(broken(NOT_BYTECODE)))
    };
    assert_ne!(a.id(), b.id(), "two assets, as a game's two compiles are");
    for _ in 0..4 {
        app.world_mut().spawn(Script::new(a.clone()));
    }
    app.world_mut().spawn(Script::new(b));
    frames(&mut app, 10);

    // each of the five entities was told, once
    assert_eq!(endings(&app).len(), 5, "every entity is told, once: {:?}", endings(&app));
    assert_eq!(
        app.world().resource::<ScriptWorld>().broken_programs(),
        1,
        "and the parse happened once, for the one program behind both"
    );
}

/// Two broken programs are two, so what is remembered is the program and not "something failed".
#[test]
fn two_broken_programs_are_two() {
    let mut app = app();
    let (a, b) = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        (assets.add(broken(NOT_BYTECODE)), assets.add(broken(NOR_IS_THIS)))
    };
    app.world_mut().spawn(Script::new(a));
    app.world_mut().spawn(Script::new(b));
    frames(&mut app, 10);

    assert_eq!(endings(&app).len(), 2);
    assert_eq!(app.world().resource::<ScriptWorld>().broken_programs(), 2);
}

/// **A file mended under its own handle runs.** This is the asset server's hot reload and an
/// editor's Apply: the same id, other bytes. Being marked once is a pause, not a verdict.
#[test]
fn an_asset_mended_under_its_own_handle_starts_the_script() {
    let mut app = app();
    let handle = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(broken(NOT_BYTECODE));
    let entity = app.world_mut().spawn(Script::new(handle.clone())).id();
    frames(&mut app, 6);
    assert_eq!(endings(&app).len(), 1, "it failed first");
    assert!(app.world().resource::<Seen>().0.is_empty(), "and ran nothing");

    app.world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .insert(handle.id(), compile("loop { Rubevy.ask('alive').pop }"))
        .expect("the asset is there to replace");
    frames(&mut app, 6);

    assert!(
        app.world().resource::<Seen>().0.iter().any(|k| k == "alive"),
        "the mended file runs: {:?}",
        app.world().resource::<Seen>().0
    );
    assert!(!app.world().entity(entity).contains::<ScriptDone>(), "and is not marked any more");
    assert_eq!(app.world().resource::<ScriptWorld>().loaded_programs(), 1);
    // the broken bytes stay remembered: they are still a program this VM could not load, and
    // anything that hands them to it again gets the answer without a parse
    assert_eq!(app.world().resource::<ScriptWorld>().broken_programs(), 1);
}

/// The other way an asset changes: another asset altogether. `replace_script` takes the
/// `ScriptDone` off itself, which is what makes it work here without knowing about any of this.
#[test]
fn replacing_a_broken_script_starts_the_new_one() {
    let mut app = app();
    let (bad, good) = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        (assets.add(broken(NOT_BYTECODE)), assets.add(compile("loop { Rubevy.ask('alive').pop }")))
    };
    let entity = app.world_mut().spawn(Script::new(bad)).id();
    frames(&mut app, 6);
    assert_eq!(endings(&app).len(), 1);

    let mut queue = bevy::ecs::world::CommandQueue::default();
    let mut commands = Commands::new(&mut queue, app.world());
    replace_script(&mut commands, entity, Script::new(good));
    queue.apply(app.world_mut());
    frames(&mut app, 6);

    assert!(app.world().resource::<Seen>().0.iter().any(|k| k == "alive"), "the new script runs");
    assert_eq!(endings(&app).len(), 1, "and nothing new was said about the old one");
}

/// A script that runs and ends is unchanged by any of this: one ending, `Finished`, with the
/// value the task answered. The two roads into `ScriptEnded` meet here.
#[test]
fn a_script_that_runs_to_its_end_still_ends_the_way_it_did() {
    let mut app = app();
    let handle = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile("41 + 1"));
    let entity = app.world_mut().spawn(Script::new(handle)).id();
    frames(&mut app, 8);

    assert_eq!(endings(&app).len(), 1);
    assert_eq!(endings(&app)[0].0, entity);
    assert_eq!(endings(&app)[0].1, ScriptStatus::Finished);
    assert_eq!(endings(&app)[0].2, "42");
}
