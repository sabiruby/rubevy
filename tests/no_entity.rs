//! **A task with no entity is nobody's**: what it asks comes out with `Request::entity == None`,
//! and what it writes and is refused is `RejectedWrite::by == None`.
//!
//! Until 2026-09-26 both were `Some` of an entity nobody had spawned. The natives spelled "this
//! task has no entity" as the number `u64::MAX`, and `Entity::try_from_bits(u64::MAX)` is not
//! `None` — the first test below is that fact, kept so that the reason for the fix is in the
//! tests and not only in the worklog (`docs/worklog/2026-09-26-release-0.2-a.md`, R3).

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, Request, RubevyPlugin, ScriptWorld};

/// What the scripts asked the game, so a test can read what it saw.
#[derive(Resource, Default)]
struct Seen(Vec<Request>);

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn answer_nil(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>) {
    for r in world.take_requests() {
        world.answer(&r, Answer::Nil);
        seen.0.push(r);
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Seen>()
    .register_type::<Transform>()
    .add_systems(Update, answer_nil.in_set(rubevy::RubevySet::Answer));
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

/// A task the host starts itself, outside any `Script`: it has no `@rubevy_entity`.
fn spawn_bare_task(app: &mut App, src: &str) {
    let mut scripts = app.world_mut().resource_mut::<ScriptWorld>();
    let bytes = compile(src).bytes;
    let irep = scripts.vm.load(&bytes).expect("loads");
    scripts.vm.task_spawn(irep, 128, Some("no entity")).expect("spawns");
}

/// Why the natives no longer spell "no entity" as `u64::MAX`: to Bevy those bits are an entity.
#[test]
fn u64_max_is_an_entity_to_bevy() {
    let e = Entity::try_from_bits(u64::MAX).expect("Bevy takes these bits");
    assert_eq!(e.to_bits(), u64::MAX, "and gives the same bits back");
}

#[test]
fn a_question_from_a_task_with_no_entity_has_no_entity() {
    let mut app = app();
    spawn_bare_task(&mut app, "Rubevy.ask('who').pop\n");
    frames(&mut app, 4);
    let seen = app.world().resource::<Seen>();
    let r = seen.0.iter().find(|r| r.kind == "who").expect("the task asked");
    assert_eq!(r.entity, None, "not an entity nobody spawned");
}

/// A script's own task clears its entity and asks: the same `None`, through the other way a task
/// comes to have no entity.
#[test]
fn a_task_that_cleared_its_entity_asks_as_nobody() {
    let mut app = app();
    let h = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(
        "Task.current.instance_variable_set(:@rubevy_entity, nil)\nRubevy.ask('who').pop\n",
    ));
    app.world_mut().spawn(rubevy::Script::new(h));
    frames(&mut app, 4);
    let seen = app.world().resource::<Seen>();
    let r = seen.0.iter().find(|r| r.kind == "who").expect("the script asked");
    assert_eq!(r.entity, None);
}

/// A refused write by a task with no entity is by nobody.
#[test]
fn a_refused_write_by_a_task_with_no_entity_is_by_nobody() {
    let mut app = app();
    spawn_bare_task(&mut app, "Rubevy.set_resource(:NoSuchThing, { a: 1 })\nsleep 1\n");
    frames(&mut app, 4);
    let world = app.world().resource::<ScriptWorld>();
    let refused = world.rejected_writes();
    assert_eq!(refused.len(), 1, "the write was refused: {refused:?}");
    assert_eq!(refused[0].by, None, "and nobody made it");
}
