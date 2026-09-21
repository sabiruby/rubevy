//! **A question that waits on the entity that asked it** (`Held`, `App::hold_requests`): that it
//! arrives where a `Query` can find it, that two tasks of one entity may wait side by side, that
//! a kind nobody registered is untouched — and, the half this exists for, that **no way of ending
//! leaves a request nobody will answer behind**: answered, despawned, replaced, ended, raised,
//! overrun, `ScriptTask` removed by hand, paused.
//!
//! The thing every one of those is measured against is `Vm::gc_registered`, which is where a
//! `Rubevy.ask` keeps the queue its script is parked on (`install_host_api`) and a `Arg::Value`
//! keeps the Hash it carried (`RootedValue`). A request that is thrown away without being
//! answered and without being tidied up leaves its entry there for the life of the VM, and that
//! entry is the leak this whole component is against.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{
    replace_script, Answer, Held, HoldRequests, MrbAsset, RubevyPlugin, RubevySet, Script,
    ScriptDone, ScriptTask, ScriptWorld,
};

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// What the game's ordinary answering system saw — the kinds that are *not* held.
#[derive(Resource, Default)]
struct Seen {
    kinds: Vec<String>,
}

/// The frames on which a `Held` was added and removed, as the two signals the component is meant
/// to be read by (`Added` / `RemovedComponents`).
#[derive(Resource, Default)]
struct Watched {
    added: Vec<u32>,
    removed: Vec<u32>,
}

fn answer_the_rest(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>) {
    for r in world.take_requests() {
        seen.kinds.push(r.kind.clone());
        world.answer(&r, Answer::Nil);
    }
}

fn watch(
    started: Query<Entity, Added<Held>>,
    mut stopped: RemovedComponents<Held>,
    frame: Res<bevy::diagnostic::FrameCount>,
    mut watched: ResMut<Watched>,
) {
    for _ in &started {
        watched.added.push(frame.0);
    }
    for _ in stopped.read() {
        watched.removed.push(frame.0);
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
    .init_resource::<Watched>()
    .add_systems(Update, (answer_the_rest, watch).in_set(RubevySet::Answer));
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

/// How many objects the VM is being told to keep for the host's sake. Nothing else in these
/// tests registers anything, so this is the task of each running script plus one for each
/// question that is waiting plus one for each Hash a waiting question carried.
fn registered(app: &App) -> usize {
    app.world().resource::<ScriptWorld>().vm.gc_registered.len()
}

/// Runs frames until `f` says yes, at most `at_most` of them, and says how many it took. The
/// bound is the test's own patience and not a number about rubevy: every one of these settles in
/// two or three frames, and the margin is for a machine that is busy.
fn until(app: &mut App, at_most: usize, f: impl Fn(&App) -> bool) -> usize {
    for i in 0..at_most {
        if f(app) {
            return i;
        }
        frames(app, 1);
    }
    assert!(f(app), "still not true after {at_most} frames");
    at_most
}

// --------------------------------------------------------------------------- it arrives

/// The question lands on the entity that asked, where `Added<Held>` finds it, and the answer
/// given through the component wakes the script.
#[test]
fn a_held_question_waits_on_the_entity_that_asked() {
    let mut app = app();
    let e = spawn_script(&mut app, "r = Rubevy.ask('wait').pop\nRubevy.ask('after', r).pop\n");
    until(&mut app, 20, |app| app.world().entity(e).get::<Held>().is_some());

    {
        let held = app.world().entity(e).get::<Held>().expect("waiting");
        assert_eq!(held.len(), 1);
        assert!(held.has("wait"));
        assert_eq!(held.waiting()[0].kind, "wait");
        assert_eq!(held.waiting()[0].entity, Some(e));
    }
    // and the game's own answering system never saw it
    assert!(!app.world().resource::<Seen>().kinds.iter().any(|k| k == "wait"));
    // one `Added<Held>`, on the frame the question was asked
    assert_eq!(app.world().resource::<Watched>().added.len(), 1);

    // answer it the way a game does
    app.world_mut().run_system_once_held(e, "wait", Answer::Num(7.0));
    until(&mut app, 20, |app| {
        app.world().resource::<Seen>().kinds.iter().any(|k| k == "after")
    });
    // the script really got the number back
    assert!(app.world().resource::<Seen>().kinds.iter().any(|k| k == "after"));
    // and the component went with the last answer
    assert!(app.world().entity(e).get::<Held>().is_none(), "the empty Held is taken off");
    assert_eq!(app.world().resource::<Watched>().removed.len(), 1);
}

/// One entity, two tasks, the same kind at the same time: both wait, and `answer` takes the one
/// that has waited longest.
#[test]
fn two_tasks_of_one_entity_wait_side_by_side() {
    let mut app = app();
    let e = spawn_script(
        &mut app,
        "Task.new { Rubevy.ask('wait', 2).pop; Rubevy.ask('second').pop }\n\
         sleep 0.02\n\
         Rubevy.ask('wait', 1).pop\n\
         Rubevy.ask('first').pop\n",
    );
    until(&mut app, 40, |app| {
        app.world().entity(e).get::<Held>().is_some_and(|h| h.len() == 2)
    });
    {
        let held = app.world().entity(e).get::<Held>().expect("two waiting");
        assert_eq!(held.len(), 2);
        // oldest first: the child task asked before the script did
        assert_eq!(held.waiting()[0].num(0), Some(2.0));
        assert_eq!(held.waiting()[1].num(0), Some(1.0));
    }
    app.world_mut().run_system_once_held(e, "wait", Answer::Nil);
    until(&mut app, 20, |app| {
        app.world().resource::<Seen>().kinds.iter().any(|k| k == "second")
    });
    // the *child's* question was the one answered, and the script's is still standing
    let kinds = &app.world().resource::<Seen>().kinds;
    assert!(kinds.iter().any(|k| k == "second"), "the older question was answered");
    assert!(!kinds.iter().any(|k| k == "first"), "the younger one is still waiting");
    assert_eq!(app.world().entity(e).get::<Held>().expect("one left").len(), 1);
}

/// **A loop of held questions**: answered, asked again on the next frame, answered again. Each
/// one is its own `Added<Held>` — the component really goes when it empties and really comes
/// back, rather than the entity looking as if it had never stopped waiting.
///
/// It is the shape `loop { move }` has, and it is the one that was wrong first: the sweep queues
/// the removal of an empty `Held` in front of the insertion of the frame's questions, so a
/// request pushed into the component that was on its way out went with it and the script was
/// parked for good.
#[test]
fn a_question_asked_again_is_a_new_arrival() {
    let mut app = app();
    let e = spawn_script(&mut app, "3.times { Rubevy.ask('wait').pop }\nRubevy.ask('after').pop\n");
    for turn in 1..=3 {
        until(&mut app, 30, |app| app.world().entity(e).get::<Held>().is_some());
        assert_eq!(
            app.world().resource::<Watched>().added.len(),
            turn,
            "turn {turn}: one Added<Held> per question"
        );
        app.world_mut().run_system_once_held(e, "wait", Answer::Nil);
        frames(&mut app, 1);
    }
    until(&mut app, 30, |app| {
        app.world().resource::<Seen>().kinds.iter().any(|k| k == "after")
    });
    let watched = app.world().resource::<Watched>();
    assert_eq!(watched.added.len(), 3);
    assert_eq!(watched.removed.len(), 3, "and one RemovedComponents per answer");
    assert_eq!(registered(&app), 1, "the task, and nothing else");
}

/// A kind nobody registered goes on coming out of `take_requests`, and a held kind asked by a
/// task with no entity does too — it has nowhere to wait.
#[test]
fn what_is_not_held() {
    let mut app = app();
    let e = spawn_script(&mut app, "loop { Rubevy.ask('elsewhere').pop }\n");
    until(&mut app, 20, |app| {
        app.world().resource::<Seen>().kinds.iter().any(|k| k == "elsewhere")
    });
    assert!(app.world().entity(e).get::<Held>().is_none());

    // a task the host started itself, outside any `Script`: it has no entity
    {
        let mut scripts = app.world_mut().resource_mut::<ScriptWorld>();
        let bytes = compile("Rubevy.ask('wait').pop\n").bytes;
        let irep = scripts.vm.load(&bytes).expect("loads");
        scripts.vm.task_spawn(irep, 128, Some("no entity")).expect("spawns");
    }
    until(&mut app, 20, |app| {
        app.world().resource::<Seen>().kinds.iter().any(|k| k == "wait")
    });
    assert!(
        app.world().resource::<Seen>().kinds.iter().any(|k| k == "wait"),
        "a held kind with no entity to wait on goes to the game's answering system"
    );
}

// ------------------------------------------------------- nothing is left waiting for nobody

/// The ordinary ending: the game answers, and the registration the question held goes with the
/// answer.
#[test]
fn an_answer_lets_the_queue_go() {
    let mut app = app();
    let e = spawn_script(&mut app, "Rubevy.ask('wait').pop\nsleep 10\n");
    until(&mut app, 20, |app| app.world().entity(e).get::<Held>().is_some());
    // the task and the queue the question is parked on
    assert_eq!(registered(&app), 2);
    app.world_mut().run_system_once_held(e, "wait", Answer::Nil);
    frames(&mut app, 3);
    assert_eq!(registered(&app), 1, "the queue is let go of; the task is still running");
}

/// A question carrying a Hash: answering releases the queue *and* the value, which is the two
/// halves of `Rubevy.ask`'s registration.
#[test]
fn a_question_that_carried_a_hash() {
    let mut app = app();
    let e = spawn_script(&mut app, "Rubevy.ask('wait', { a: 1 }).pop\nsleep 10\n");
    until(&mut app, 20, |app| app.world().entity(e).get::<Held>().is_some());
    assert_eq!(registered(&app), 3, "the task, the queue, and the Hash");
    assert!(app.world().entity(e).get::<Held>().expect("waiting").waiting()[0].value(0).is_some());
    app.world_mut().run_system_once_held(e, "wait", Answer::Nil);
    // the Hash is let go of in two steps: the `Drop` of the last `RootedValue` puts it on the
    // release queue, and the next frame's sweep unregisters it
    frames(&mut app, 3);
    assert_eq!(registered(&app), 1);
}

/// **Despawned while waiting.** Bevy takes the component with the entity, the removal hook lets
/// the queue and the value go, and nothing of the script is left registered.
#[test]
fn a_despawned_entity_leaves_no_question_behind() {
    let mut app = app();
    let e = spawn_script(&mut app, "Rubevy.ask('wait', { a: 1 }).pop\nsleep 10\n");
    until(&mut app, 20, |app| app.world().entity(e).get::<Held>().is_some());
    assert_eq!(registered(&app), 3);
    app.world_mut().despawn(e);
    frames(&mut app, 3);
    assert_eq!(registered(&app), 0, "the task, the queue and the Hash are all let go of");
}

/// **Replaced while waiting.** `replace_script` takes the `Held` with the `ScriptTask`.
#[test]
fn replace_script_takes_the_waiting_question_with_it() {
    let mut app = app();
    let e = spawn_script(&mut app, "Rubevy.ask('wait', { a: 1 }).pop\nsleep 10\n");
    until(&mut app, 20, |app| app.world().entity(e).get::<Held>().is_some());
    assert_eq!(registered(&app), 3);

    let next = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile("sleep 10\n"));
    let mut queue = bevy::ecs::world::CommandQueue::default();
    let mut commands = Commands::new(&mut queue, app.world());
    replace_script(&mut commands, e, Script::new(next));
    queue.apply(app.world_mut());

    assert!(app.world().entity(e).get::<Held>().is_none(), "the swap took it");
    frames(&mut app, 4);
    // what is registered now is the new script's task and nothing else
    assert_eq!(registered(&app), 1);
    assert!(app.world().entity(e).get::<ScriptTask>().is_some(), "the new script runs");
}

/// **The `ScriptTask` taken off by hand**, which is how a game stops a script without replacing
/// it. The next tick sweeps the `Held` that is left standing on an entity with no script.
#[test]
fn a_script_stopped_by_hand_is_swept() {
    let mut app = app();
    let e = spawn_script(&mut app, "Rubevy.ask('wait', { a: 1 }).pop\nsleep 10\n");
    until(&mut app, 20, |app| app.world().entity(e).get::<Held>().is_some());
    assert_eq!(registered(&app), 3);
    // the `Script` goes too: `start_scripts` turns a `Script` with no task back into a task, so
    // taking the task off by itself is a restart and not a stop
    app.world_mut().entity_mut(e).remove::<ScriptTask>().remove::<Script>();
    frames(&mut app, 3);
    assert!(app.world().entity(e).get::<Held>().is_none(), "swept");
    assert_eq!(registered(&app), 0);
}

/// **The script ran to its end** while a question of its own was still waiting — a task it made
/// with `Task.new` asked and the script came back without waiting for it. The frame that sends
/// `ScriptEnded` is the frame the `Held` goes.
#[test]
fn a_script_that_ends_takes_its_waiting_questions_with_it() {
    let mut app = app();
    let e = spawn_script(&mut app, "Task.new { Rubevy.ask('wait', { a: 1 }).pop }\nsleep 0.02\n");
    until(&mut app, 40, |app| app.world().entity(e).get::<Held>().is_some());
    until(&mut app, 40, |app| app.world().entity(e).get::<ScriptDone>().is_some());
    assert!(app.world().entity(e).get::<Held>().is_none(), "it went with the ending");
    frames(&mut app, 3);
    assert_eq!(registered(&app), 0);
}

/// **The script raised** in the same tick as it asked. There is no script left to wait for, so
/// the question is not held at all: it goes to the game's own answering system, which is where a
/// question nobody is waiting on went before there was a `Held`.
#[test]
fn a_script_that_raised_asks_but_does_not_wait() {
    let mut app = app();
    let e = spawn_script(&mut app, "Rubevy.ask('wait', { a: 1 })\nraise 'boom'\n");
    until(&mut app, 40, |app| app.world().entity(e).get::<ScriptDone>().is_some());
    frames(&mut app, 3);
    assert!(app.world().entity(e).get::<Held>().is_none());
    assert!(app.world().resource::<Seen>().kinds.iter().any(|k| k == "wait"));
    assert_eq!(registered(&app), 0);
}

/// **The script raised** while a question asked on an earlier frame was waiting — which is the
/// ordinary shape: the script is inside `move` and something in it raises later.
#[test]
fn a_script_that_raised_takes_its_waiting_questions_with_it() {
    let mut app = app();
    let e = spawn_script(
        &mut app,
        "Task.new { Rubevy.ask('wait', { a: 1 }).pop }\nsleep 0.02\nraise 'boom'\n",
    );
    until(&mut app, 60, |app| app.world().entity(e).get::<Held>().is_some());
    until(&mut app, 60, |app| app.world().entity(e).get::<ScriptDone>().is_some());
    assert!(app.world().entity(e).get::<Held>().is_none(), "it went with the ending");
    frames(&mut app, 3);
    assert_eq!(registered(&app), 0);
}

/// **The script overran** — a native that cannot be switched out, which is an `Exception` and not
/// a `StandardError`, so no `rescue` in the script can have caught it.
#[test]
fn a_script_that_overran_takes_its_waiting_questions_with_it() {
    let mut app = app();
    let e = spawn_script(
        &mut app,
        "Task.new { Rubevy.ask('wait', { a: 1 }).pop }\n\
         sleep 0.02\n\
         a = (1..200000).to_a\n\
         a.sort { |x, y| y <=> x }\n",
    );
    until(&mut app, 60, |app| app.world().entity(e).get::<Held>().is_some());
    until(&mut app, 200, |app| app.world().entity(e).get::<ScriptDone>().is_some());
    assert!(app.world().entity(e).get::<Held>().is_none());
    frames(&mut app, 3);
    assert_eq!(registered(&app), 0);
}

/// **A paused VM keeps what is waiting.** Nothing runs, so nothing is asked and nothing is lost;
/// an answer given during the pause reaches the script when the scripts run again.
#[test]
fn a_pause_keeps_what_is_waiting() {
    let mut app = app();
    let e = spawn_script(&mut app, "Rubevy.ask('wait').pop\nRubevy.ask('after').pop\n");
    until(&mut app, 20, |app| app.world().entity(e).get::<Held>().is_some());
    app.world_mut().resource_mut::<ScriptWorld>().budget = 0;
    frames(&mut app, 10);
    assert_eq!(
        app.world().entity(e).get::<Held>().map(|h| h.len()),
        Some(1),
        "a pause answers nothing and loses nothing"
    );
    assert_eq!(registered(&app), 2);
    // answered while paused, read when the pause ends
    app.world_mut().run_system_once_held(e, "wait", Answer::Nil);
    frames(&mut app, 3);
    assert!(!app.world().resource::<Seen>().kinds.iter().any(|k| k == "after"));
    app.world_mut().resource_mut::<ScriptWorld>().budget = 200_000;
    until(&mut app, 20, |app| {
        app.world().resource::<Seen>().kinds.iter().any(|k| k == "after")
    });
}

/// Registering a kind rubevy answers itself, or one the game answers inside the tick, does not
/// take it away from where it is answered.
#[test]
fn what_cannot_be_held() {
    let mut app = app();
    {
        let mut scripts = app.world_mut().resource_mut::<ScriptWorld>();
        scripts.answer_in_tick("intick", Box::new(|_w: &World, _r| Answer::Num(1.0)));
        scripts.hold_requests("component.get");
        scripts.hold_requests("intick");
        assert_eq!(scripts.held_kinds(), 1, "only `wait`, from the app builder");
    }
    let e = spawn_script(
        &mut app,
        "Rubevy.ask('intick').pop\nRubevy.entity[:Transform]\nRubevy.ask('after').pop\n",
    );
    until(&mut app, 20, |app| {
        app.world().resource::<Seen>().kinds.iter().any(|k| k == "after")
    });
    assert!(app.world().entity(e).get::<Held>().is_none());
}

// --------------------------------------------------------------------------- the small helper

/// `held.answer(..)` as a game writes it, run once against the world — the tests above are not
/// systems and this is the shortest way to stand where one stands.
trait AnswerHeld {
    fn run_system_once_held(&mut self, entity: Entity, kind: &str, answer: Answer);
}

impl AnswerHeld for World {
    fn run_system_once_held(&mut self, entity: Entity, kind: &str, answer: Answer) {
        self.resource_scope(|world: &mut World, mut scripts: Mut<ScriptWorld>| {
            let Some(mut held) = world.get_mut::<Held>(entity) else { return };
            held.answer(&mut scripts, kind, answer);
        });
    }
}
