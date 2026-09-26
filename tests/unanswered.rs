//! **A question the game lets go of without answering** (`Rubevy::Unanswered`).
//!
//! A `Request` that is dropped instead of being handed to `ScriptWorld::answer` used to keep its
//! queue registered with the collector for the life of the VM, and whatever was parked on the
//! queue stayed parked there for as long. Since 0.2.0 the last clone going closes the queue at
//! the next frame's sweep, and the task waiting on it raises `Rubevy::Unanswered` in its `pop`
//! (the author's decision of 2026-09-26; `docs/worklog/2026-09-26-release-0.2-a.md`, R2).
//!
//! What is counted is `Vm::gc_registered`, as in `tests/held.rs`: a question that is neither
//! answered nor closed leaves its queue there.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, Request, RubevyPlugin, RubevySet, Script, ScriptWorld};

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// What the game saw of the kinds it answers: the kind and its first argument as text.
#[derive(Resource, Default)]
struct Seen(Vec<(String, Option<String>)>);

/// Requests the test has taken out of the game's hands, to drop or answer when it says.
#[derive(Resource, Default)]
struct Kept(Vec<Request>);

/// The game: a question of kind `drop` is let go of on the spot, one of kind `keep` is put aside
/// for the test, everything else is answered with nil.
fn answer(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>, mut kept: ResMut<Kept>) {
    for r in world.take_requests() {
        match r.kind.as_str() {
            "drop" => drop(r),
            "keep" => kept.0.push(r),
            _ => {
                seen.0.push((r.kind.clone(), r.text(0).map(str::to_string)));
                world.answer(&r, Answer::Nil);
            }
        }
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
    .init_resource::<Kept>()
    .add_systems(Update, answer.in_set(RubevySet::Answer));
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

fn registered(app: &App) -> usize {
    app.world().resource::<ScriptWorld>().vm.gc_registered.len()
}

fn seen(app: &App, kind: &str) -> Vec<Option<String>> {
    app.world().resource::<Seen>().0.iter().filter(|(k, _)| k == kind).map(|(_, a)| a.clone()).collect()
}

/// Runs frames until `f` says yes. The bound is the test's patience, not a number of rubevy's:
/// everything here settles in two or three frames.
fn until(app: &mut App, at_most: usize, f: impl Fn(&App) -> bool) {
    for _ in 0..at_most {
        if f(app) {
            return;
        }
        frames(app, 1);
    }
    assert!(f(app), "still not true after {at_most} frames");
}

/// The script's own task, parked on a question the game drops: it wakes with
/// `Rubevy::Unanswered`, rescues it, and goes on.
#[test]
fn a_dropped_question_raises_unanswered_in_the_task_that_waits() {
    let mut app = app();
    spawn_script(
        &mut app,
        r#"
          begin
            Rubevy.ask("drop").pop
            Rubevy.ask("after", "answered").pop
          rescue Rubevy::Unanswered => e
            Rubevy.ask("after", e.class.to_s).pop
          end
          sleep 10
        "#,
    );
    until(&mut app, 20, |app| !seen(app, "after").is_empty());
    assert_eq!(seen(&app, "after"), vec![Some(String::from("Rubevy::Unanswered"))]);
    // the script's task and nothing else: the dropped queue is let go of, and the question it
    // asked after is answered
    frames(&mut app, 2);
    assert_eq!(registered(&app), 1);
}

/// A task the script made with `Task.new`, which is the one nothing else stops: it unwinds
/// through its `ensure`, and an unrescued `Rubevy::Unanswered` ends it.
#[test]
fn a_child_task_unwinds_through_its_ensure() {
    let mut app = app();
    spawn_script(
        &mut app,
        r#"
          Task.new do
            begin
              Rubevy.ask("drop").pop
            ensure
              Rubevy.ask("ensure", "ran").pop
            end
            Rubevy.ask("after", "not reached").pop
          end
          sleep 10
        "#,
    );
    until(&mut app, 20, |app| !seen(app, "ensure").is_empty());
    frames(&mut app, 3);
    assert_eq!(seen(&app, "ensure"), vec![Some(String::from("ran"))]);
    assert!(seen(&app, "after").is_empty(), "the raise went on out of the task");
    assert_eq!(registered(&app), 1, "the script's task; the child has ended and holds nothing");
}

/// A `pop` that comes *after* the question was dropped — the script asked, went on, and waits
/// later — raises too: the queue is closed, not merely empty.
#[test]
fn a_pop_after_the_drop_raises_as_well() {
    let mut app = app();
    spawn_script(
        &mut app,
        r#"
          q = Rubevy.ask("drop")
          sleep 0.05
          begin
            q.pop
          rescue Rubevy::Unanswered
            Rubevy.ask("after", "raised").pop
          end
          sleep 10
        "#,
    );
    until(&mut app, 60, |app| !seen(app, "after").is_empty());
    assert_eq!(seen(&app, "after"), vec![Some(String::from("raised"))]);
}

/// A clone keeps the question open: the queue is closed when the **last** of them goes, and not
/// before — a game may keep one copy in a table and look at another.
#[test]
fn a_question_is_closed_when_its_last_clone_goes() {
    let mut app = app();
    spawn_script(
        &mut app,
        r#"
          begin
            Rubevy.ask("keep").pop
          rescue Rubevy::Unanswered
            Rubevy.ask("after", "raised").pop
          end
          sleep 10
        "#,
    );
    until(&mut app, 20, |app| !app.world().resource::<Kept>().0.is_empty());
    let first = app.world_mut().resource_mut::<Kept>().0.pop().expect("kept");
    let second = first.clone();
    drop(first);
    frames(&mut app, 4);
    assert!(seen(&app, "after").is_empty(), "the clone still holds it open");
    assert_eq!(registered(&app), 2, "the task and the queue");
    drop(second);
    until(&mut app, 20, |app| !seen(app, "after").is_empty());
    assert_eq!(seen(&app, "after"), vec![Some(String::from("raised"))]);
}

/// Answered and then dropped is the ordinary ending, and it closes nothing: the answer is what
/// the script gets.
#[test]
fn an_answered_question_is_not_closed_when_it_is_dropped() {
    let mut app = app();
    spawn_script(
        &mut app,
        r#"
          begin
            v = Rubevy.ask("keep").pop
            Rubevy.ask("after", v).pop
          rescue Rubevy::Unanswered
            Rubevy.ask("after", "raised").pop
          end
          sleep 10
        "#,
    );
    until(&mut app, 20, |app| !app.world().resource::<Kept>().0.is_empty());
    let r = app.world_mut().resource_mut::<Kept>().0.pop().expect("kept");
    app.world_mut().resource_mut::<ScriptWorld>().answer(&r, Answer::Text("the answer".into()));
    drop(r);
    until(&mut app, 20, |app| !seen(app, "after").is_empty());
    assert_eq!(seen(&app, "after"), vec![Some(String::from("the answer"))]);
}

/// **A second answer is not given.** It used to be pushed into the queue after the first — so a
/// script that popped the same queue twice got both — and to unregister the queue a second time.
#[test]
fn a_second_answer_is_refused() {
    let mut app = app();
    spawn_script(
        &mut app,
        r##"
          q = Rubevy.ask("keep")
          first = q.pop
          second = q.pop(timeout_ms: 40)
          Rubevy.ask("after", "#{first.inspect} #{second.inspect}").pop
          sleep 10
        "##,
    );
    until(&mut app, 20, |app| !app.world().resource::<Kept>().0.is_empty());
    let r = app.world_mut().resource_mut::<Kept>().0.pop().expect("kept");
    let copy = r.clone();
    {
        let mut world = app.world_mut().resource_mut::<ScriptWorld>();
        world.answer(&r, Answer::Num(1.0));
        world.answer(&copy, Answer::Num(2.0));
    }
    until(&mut app, 60, |app| !seen(app, "after").is_empty());
    assert_eq!(seen(&app, "after"), vec![Some(String::from("1.0 nil"))]);
}

/// A second `answer_value` does not run its closure: the refusal comes before anything is built
/// in the VM, the same as for `answer`.
#[test]
fn a_second_answer_value_does_not_build() {
    let mut app = app();
    spawn_script(&mut app, "Rubevy.ask('keep').pop\nsleep 10\n");
    until(&mut app, 20, |app| !app.world().resource::<Kept>().0.is_empty());
    let r = app.world_mut().resource_mut::<Kept>().0.pop().expect("kept");
    let copy = r.clone();
    let mut built = 0;
    {
        let mut world = app.world_mut().resource_mut::<ScriptWorld>();
        world.answer_value(&r, |_vm| {
            built += 1;
            sabiruby::Value::Int(1)
        });
        world.answer_value(&copy, |_vm| {
            built += 1;
            sabiruby::Value::Int(2)
        });
    }
    assert_eq!(built, 1, "the second closure was not run");
}
