//! **Stopping a script** (`stop_script`, `stop_script_for`): the script's task ends, what it
//! subscribed to is closed, what it was waiting on in a `Held` is let go of, and — the half this
//! exists for — it does **not** start again on the next frame.
//!
//! Until 0.2.0 the rustdoc and `docs/host-api.md` said a stop was removing the `ScriptTask`. It is
//! not: a `Script` left on the entity is turned into a task again by the plugin, and the script
//! runs from its first line. One embedding met exactly that — a scene stopped on the way back to
//! the title screen, playing again a frame later (`docs/worklog/2026-09-26-release-0.2-a.md`, R1).

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::ecs::world::CommandQueue;
use bevy::prelude::*;
use rubevy::{
    stop_script, stop_script_for, Answer, Held, HoldRequests, MrbAsset, RubevyPlugin, RubevySet,
    Script, ScriptDone, ScriptTask, ScriptWorld,
};

/// The second VM of the test that has two.
struct Mods;

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// What the game was asked, kind by kind, in order.
#[derive(Resource, Default)]
struct Seen(Vec<String>);

fn answer_all(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>) {
    for r in world.take_requests() {
        seen.0.push(r.kind.clone());
        world.answer(&r, Answer::Nil);
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .hold_requests("wait")
    .init_resource::<Seen>()
    .add_systems(Update, answer_all.in_set(RubevySet::Answer));
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

fn count(app: &App, kind: &str) -> usize {
    app.world().resource::<Seen>().0.iter().filter(|k| *k == kind).count()
}

fn registered(app: &App) -> usize {
    app.world().resource::<ScriptWorld>().vm.gc_registered.len()
}

/// Runs frames until `f` says yes. The bound is the test's patience, not a number of rubevy's.
fn until(app: &mut App, at_most: usize, f: impl Fn(&App) -> bool) {
    for _ in 0..at_most {
        if f(app) {
            return;
        }
        frames(app, 1);
    }
    assert!(f(app), "still not true after {at_most} frames");
}

/// `stop_script` through a `Commands`, the way a system calls it.
fn stop(app: &mut App, e: Entity) {
    let mut queue = CommandQueue::default();
    let mut commands = Commands::new(&mut queue, app.world());
    stop_script(&mut commands, e);
    queue.apply(app.world_mut());
}

/// **What `stop_script` is for**: the script stops and stays stopped. Its first line asks
/// `start`, so a script started again would say so.
#[test]
fn a_stopped_script_does_not_start_again() {
    let mut app = app();
    let e = spawn_script(&mut app, "Rubevy.ask('start').pop\nloop { Rubevy.ask('tick').pop }\n");
    until(&mut app, 20, |app| count(app, "tick") >= 3);

    stop(&mut app, e);
    assert!(app.world().entity(e).get::<ScriptTask>().is_none());
    assert!(app.world().entity(e).get::<Script>().is_none());
    frames(&mut app, 3);
    let ticks = count(&app, "tick");
    frames(&mut app, 20);
    assert_eq!(count(&app, "tick"), ticks, "the stopped script asks nothing more");
    assert_eq!(count(&app, "start"), 1, "and it did not start over");
    assert!(app.world().entity(e).get::<ScriptTask>().is_none(), "no task was made again");
    assert_eq!(registered(&app), 0, "and nothing of it is registered");
}

/// **What the rustdoc used to call a stop**, kept as the reason for the function: removing the
/// `ScriptTask` alone leaves the `Script`, and the plugin runs it again from its first line.
#[test]
fn removing_the_task_alone_starts_the_script_again() {
    let mut app = app();
    let e = spawn_script(&mut app, "Rubevy.ask('start').pop\nloop { Rubevy.ask('tick').pop }\n");
    until(&mut app, 20, |app| count(app, "tick") >= 3);
    app.world_mut().entity_mut(e).remove::<ScriptTask>();
    until(&mut app, 20, |app| count(app, "start") == 2);
    assert!(app.world().entity(e).get::<ScriptTask>().is_some(), "a new task, from the top");
}

/// A child waiting on a subscription hears `Rubevy::Unsubscribed`; a child waiting on a held
/// question hears `Rubevy::Unanswered`. Both unwind and end, and nothing of the script is left.
#[test]
fn what_it_was_waiting_on_goes_with_it() {
    let mut app = app();
    let e = spawn_script(
        &mut app,
        "hits = Rubevy.subscribe(:hit)\n\
         Task.new do\n  begin\n    hits.pop\n  rescue Rubevy::Unsubscribed\n    Rubevy.ask('unsubscribed').pop\n  end\nend\n\
         Task.new do\n  begin\n    Rubevy.ask('wait').pop\n  rescue Rubevy::Unanswered\n    Rubevy.ask('unanswered').pop\n  end\nend\n\
         sleep 10\n",
    );
    until(&mut app, 20, |app| app.world().entity(e).get::<Held>().is_some());
    assert_eq!(app.world().resource::<ScriptWorld>().subscriptions(), 1);

    stop(&mut app, e);
    assert!(app.world().entity(e).get::<Held>().is_none(), "the held question went with it");
    assert_eq!(app.world().resource::<ScriptWorld>().subscriptions(), 0, "and the subscription");
    until(&mut app, 20, |app| count(app, "unsubscribed") == 1 && count(app, "unanswered") == 1);
    frames(&mut app, 3);
    assert_eq!(registered(&app), 0, "the script's task, the queues, the children: all let go of");
}

/// A script that has already run to its end is stopped too: `ScriptDone` comes off with the rest,
/// so the entity is one with no script at all, and a `Script` inserted later starts as a new one.
#[test]
fn a_script_that_ended_is_stopped_and_the_entity_can_have_another() {
    let mut app = app();
    let e = spawn_script(&mut app, "Rubevy.ask('first').pop\n");
    until(&mut app, 20, |app| app.world().entity(e).get::<ScriptDone>().is_some());

    stop(&mut app, e);
    assert!(app.world().entity(e).get::<ScriptDone>().is_none());
    assert!(app.world().entity(e).get::<ScriptTask>().is_none());
    frames(&mut app, 2);
    assert_eq!(registered(&app), 0, "the ended task was let go of once, not twice");

    let next = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile("Rubevy.ask('second').pop\n"));
    app.world_mut().entity_mut(e).insert(Script::new(next));
    until(&mut app, 20, |app| count(app, "second") == 1);
}

/// **A second VM**: `stop_script_for::<Mods>` stops that VM's script and leaves the first VM's
/// script on the same entity running.
#[test]
fn stop_script_for_stops_one_vm_and_not_the_other() {
    let mut app = app();
    app.add_plugins(RubevyPlugin::<Mods>::for_vm("assets"));
    let (first, second) = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        (
            assets.add(compile("loop { Rubevy.ask('first').pop }\n")),
            assets.add(compile("loop { sleep 0 }\n")),
        )
    };
    let e = app.world_mut().spawn((Script::new(first), Script::<Mods>::for_vm(second))).id();
    until(&mut app, 20, |app| {
        app.world().entity(e).get::<ScriptTask<Mods>>().is_some() && count(app, "first") >= 2
    });

    let mut queue = CommandQueue::default();
    let mut commands = Commands::new(&mut queue, app.world());
    stop_script_for::<Mods>(&mut commands, e);
    queue.apply(app.world_mut());

    let asked = count(&app, "first");
    frames(&mut app, 10);
    assert!(app.world().entity(e).get::<ScriptTask<Mods>>().is_none(), "the mods' script stopped");
    assert!(app.world().entity(e).get::<Script<Mods>>().is_none());
    assert!(app.world().entity(e).get::<ScriptTask>().is_some(), "the first VM's is still there");
    assert!(count(&app, "first") > asked, "and still running");
}
