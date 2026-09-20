//! `Rubevy.next_frame`: the wait a script could not spell.
//!
//! Until this, the only wait a script had was `sleep`, and `sleep` is real time on a clock that
//! moves in whole ticks of 4 ms (`Vm::task_tick_unit_ms`). "The next frame and no later" is not
//! a length of time, so `sleep 0` could not mean it — and every game that wanted one pass of a
//! script a frame wrote the machinery itself: rubevy_games' garden answers a question of its own
//! (`Rubevy.ask("frame")`) from a system that answers it once a frame, and that is the shape the
//! next game would have written again.
//!
//! What the host does is in `wake_next_frame` (src/lib.rs): the task is moved to a queue of its
//! own and woken at the *head* of the next tick, before the first run of the VM, with the frame
//! number as its answer. The record is `docs/worklog/2026-09-20-next-frame.md`.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, Request, RubevyPlugin, RubevySet, Script, ScriptTask, ScriptWorld};

/// Something for a script to write and read back, so that "the write has landed" can be asked
/// about a frame.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
struct Dial {
    value: f32,
}

/// What the scripts told the game, in the order the answering system took it.
#[derive(Resource, Default)]
struct Seen(Vec<Request>);

/// `FrameStats::carried_reflect` of each frame, which is where a wake the frame time cut short
/// shows up.
#[derive(Resource, Default)]
struct Carried(Vec<u32>);

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

fn record_carried(world: Res<ScriptWorld>, mut carried: ResMut<Carried>) {
    carried.0.push(world.last_frame().carried_reflect);
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Seen>()
    .init_resource::<Carried>()
    .register_type::<Dial>()
    .add_systems(Update, answer_nil.in_set(RubevySet::Answer))
    .add_systems(Update, record_carried.after(RubevySet::Tick));
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

/// One script on an entity of the caller's, so that a test can read the entity back.
fn run(app: &mut App, entity: Entity, src: &str) {
    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
    app.world_mut().entity_mut(entity).insert(Script::new(script));
}

/// What the scripts said under one kind: the numbers of each report, in order.
fn said(app: &App, kind: &str) -> Vec<Vec<f64>> {
    app.world()
        .resource::<Seen>()
        .0
        .iter()
        .filter(|r| r.kind == kind)
        .map(|r| (0..r.args.len()).map(|i| r.num_or(i, f64::NAN)).collect())
        .collect()
}

// ---------------------------------------------------------------------------------------------

/// The promise itself: the frame the script goes on in is the one after the frame it asked in,
/// and the number it is answered is that frame's own.
#[test]
fn a_task_wakes_on_the_frame_after_the_one_it_asked_in() {
    let mut app = app();
    let who = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        who,
        r#"
          f0 = $rubevy[:frame]
          f = Rubevy.next_frame
          Rubevy.ask("woke", f0.to_f, f.to_f, ($rubevy[:frame] - f0).to_f)
          Rubevy.ask("kind", f.class.to_s)
        "#,
    );
    frames(&mut app, 8);

    let woke = said(&app, "woke");
    assert_eq!(woke.len(), 1, "the script reported once: {woke:?}");
    let (f0, f, waited) = (woke[0][0], woke[0][1], woke[0][2]);
    assert_eq!(f, f0 + 1.0, "woken on the very next frame (asked in {f0}, woke in {f})");
    assert_eq!(waited, 1.0, "and $rubevy agrees with what it was answered");
    assert_eq!(
        app.world().resource::<Seen>().0.iter().find(|r| r.kind == "kind").and_then(|r| r.text(0)),
        Some("Integer"),
        "the answer is the Integer $rubevy[:frame] is, not a Float"
    );
}

/// `Rubevy.each_frame` is the loop around it: one pass a frame, every frame, with the frame's
/// delta and its number.
#[test]
fn every_pass_of_each_frame_is_one_frame() {
    const PASSES: usize = 6;
    let mut app = app();
    let who = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        who,
        &format!(
            r#"
              n = 0
              Rubevy.each_frame do |dt, f|
                n += 1
                Rubevy.ask("pass", f.to_f, dt, n.to_f)
                break if n == {PASSES}
              end
              Rubevy.ask("done", n.to_f)
            "#
        ),
    );
    frames(&mut app, 20);

    let passes = said(&app, "pass");
    assert_eq!(passes.len(), PASSES, "the block ran once a frame: {passes:?}");
    for (i, pass) in passes.iter().enumerate() {
        assert_eq!(pass[2], i as f64 + 1.0, "in order: {passes:?}");
        if i > 0 {
            assert_eq!(
                pass[0],
                passes[i - 1][0] + 1.0,
                "one frame apart, every time: {passes:?}"
            );
        }
        assert!(pass[1] > 0.0, "and the block was handed the frame's delta: {passes:?}");
    }
    assert_eq!(said(&app, "done"), vec![vec![PASSES as f64]], "`break` ends `each_frame`");
}

/// What the wait is for, in the smallest case a game has: a write is applied at the end of the
/// frame, and a script that waits one frame reads it back.
#[test]
fn a_write_is_readable_on_the_next_frame() {
    let mut app = app();
    let who = app.world_mut().spawn(Dial { value: 1.0 }).id();
    run(
        &mut app,
        who,
        r#"
          e = Rubevy.entity
          before = e[:Dial][:value]
          e[:Dial] = { value: 5.0 }
          in_the_same_tick = e[:Dial][:value]
          Rubevy.next_frame
          Rubevy.ask("read", before, in_the_same_tick, e[:Dial][:value])
          Rubevy.ask("refused", Rubevy.rejected_writes.length.to_f)
        "#,
    );
    frames(&mut app, 8);

    assert_eq!(
        said(&app, "read"),
        vec![vec![1.0, 1.0, 5.0]],
        "the write is not there in the tick that made it, and is there on the next frame"
    );
    assert_eq!(said(&app, "refused"), vec![vec![0.0]], "and nothing was refused");
}

/// A paused VM is not a frame that is waited through. With `budget = 0` nothing of the scripts
/// runs, and the plugin does not move their clock on either (`ScriptWorld::budget`); this is that
/// rule for a wait that is counted in frames rather than in seconds.
#[test]
fn a_paused_frame_is_not_the_next_frame() {
    const PAUSED: usize = 5;
    let mut app = app();
    let who = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        who,
        r#"
          f0 = $rubevy[:frame]
          Rubevy.ask("asked", f0.to_f)
          f = Rubevy.next_frame
          Rubevy.ask("woke", f0.to_f, f.to_f)
        "#,
    );
    // the frames it takes the script to start and ask
    while said(&app, "asked").is_empty() {
        app.update();
    }
    let asked_in = said(&app, "asked")[0][0];

    app.world_mut().resource_mut::<ScriptWorld>().budget = 0;
    frames(&mut app, PAUSED);
    assert!(said(&app, "woke").is_empty(), "nobody is woken by a paused frame");

    app.world_mut().resource_mut::<ScriptWorld>().budget = 200_000;
    frames(&mut app, 2);
    let woke = said(&app, "woke");
    assert_eq!(woke.len(), 1, "and it wakes once the scripts run again: {woke:?}");
    assert_eq!(
        woke[0][1],
        asked_in + PAUSED as f64 + 1.0,
        "on the first frame the scripts ran in, not on the frame after it asked: {woke:?}"
    );
}

/// When the frame time cannot wake everybody, the rest wake on the frame after — and they are the
/// *first* woken there, so a queue of waiters moves round rather than the same few going first
/// for ever.
///
/// The frame time here is one nanosecond, which is past by the time the first task has been
/// woken: one wake a frame, and the frames the three tasks report are three frames in the order
/// they asked in. (A frame time that short is not a setting a game would use — the run of the VM
/// is cut with everything else, which is why the limit is put back before the tasks are let run.
/// What it buys the test is a cut that falls in the same place on every machine.)
#[test]
fn a_wake_the_frame_time_cut_short_goes_on_at_the_head_of_the_next_frame() {
    const TASKS: usize = 3;
    let mut app = app();
    for i in 0..TASKS {
        let entity = app.world_mut().spawn_empty().id();
        run(
            &mut app,
            entity,
            &format!(
                r#"
                  Rubevy.ask("ready", {i}.to_f)
                  f = Rubevy.next_frame
                  Rubevy.ask("woke", {i}.to_f, f.to_f)
                "#
            ),
        );
    }
    // every task has asked and is waiting: the report is made in the same round as the question
    while said(&app, "ready").len() < TASKS {
        app.update();
    }

    app.world_mut().resource_mut::<ScriptWorld>().frame_time = Some(Duration::from_nanos(1));
    app.world_mut().resource_mut::<Carried>().0.clear();
    frames(&mut app, TASKS);
    let carried = app.world().resource::<Carried>().0.clone();
    assert!(
        carried.first().copied().unwrap_or(0) > 0,
        "the first of those frames left somebody waiting: {carried:?}"
    );
    assert_eq!(carried.last(), Some(&0), "and by the last one the queue is empty: {carried:?}");
    assert!(
        carried.windows(2).all(|w| w[0] > w[1]),
        "the queue shrinks every frame — nobody is passed over: {carried:?}"
    );

    app.world_mut().resource_mut::<ScriptWorld>().frame_time = Some(Duration::from_millis(8));
    frames(&mut app, 4);

    let woke = said(&app, "woke");
    assert_eq!(woke.len(), TASKS, "all three woke: {woke:?}");
    for (i, report) in woke.iter().enumerate() {
        assert_eq!(report[0], i as f64, "in the order they asked in: {woke:?}");
        if i > 0 {
            assert!(
                report[1] > woke[i - 1][1],
                "each on a later frame than the one before it: {woke:?}"
            );
        }
    }
}

/// The kind is rubevy's own (`RESERVED_KINDS`): a game's answering system never sees it, and a
/// game that registers an answerer for that name does not take it over.
#[test]
fn the_game_never_sees_the_question() {
    let mut app = app();
    app.world_mut().resource_mut::<ScriptWorld>().answer_in_tick(
        "frame.next",
        Box::new(|_world, _request| Answer::Num(-1.0)),
    );
    let who = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        who,
        r#"
          f = Rubevy.next_frame
          Rubevy.ask("woke", f.to_f)
        "#,
    );
    frames(&mut app, 8);

    let woke = said(&app, "woke");
    assert_eq!(woke.len(), 1, "the script was answered: {woke:?}");
    assert!(woke[0][0] > 0.0, "by rubevy and not by the game's -1: {woke:?}");
    assert!(
        app.world().resource::<Seen>().0.iter().all(|r| r.kind != "frame.next"),
        "and the question never reached `take_requests`"
    );
}

/// Two tasks of one script wait side by side: a task made with `Task.new` waits for the same
/// frame as the script that made it, and both go on in it.
#[test]
fn two_tasks_wait_for_the_same_frame() {
    let mut app = app();
    let who = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        who,
        r#"
          Task.new(name: "second") do
            f = Rubevy.next_frame
            Rubevy.ask("child", f.to_f)
          end
          f = Rubevy.next_frame
          Rubevy.ask("parent", f.to_f)
        "#,
    );
    frames(&mut app, 8);

    let parent = said(&app, "parent");
    let child = said(&app, "child");
    assert_eq!(parent.len(), 1, "the script woke: {parent:?}");
    assert_eq!(child.len(), 1, "and so did its task: {child:?}");
    assert_eq!(parent[0][0], child[0][0], "on the same frame: {parent:?} {child:?}");
}

// ---------------------------------------------------------------------------------------------

/// **What one `Rubevy.next_frame` costs, in instructions.** A measuring instrument and not an
/// assertion: it prints, and the numbers are of the machine it ran on
/// (`tests/read_cost.rs` is the same shape).
///
///     cargo test --release --test next_frame -- --ignored --nocapture
///
/// The two scripts differ by a hundred waits and nothing else, so the difference divided by a
/// hundred is one wait: the `Rubevy.ask`, the `pop` that parks the task, and the resuming of it
/// when the host answers. Each wait is a frame, so the tall one needs a hundred frames more than
/// the short one — which is the other number a game wants from this: a wait costs a frame, and
/// what it costs inside that frame is this.
///
/// `sleep 0` is measured beside it, the same way and in the same run, because it is what a script
/// that wants "about a frame" wrote before this existed and still writes where "about" is enough
/// (`Rubevy::Camera#follow`). It is the cheaper of the two and it has to be: it is a wait the VM
/// settles by itself, with no question for the host to take off a queue and no answer to push
/// back.
#[test]
#[ignore]
fn what_one_next_frame_costs() {
    for (what, wait) in [("next_frame", "Rubevy.next_frame"), ("sleep 0", "sleep 0")] {
        let counts = [10usize, 110];
        let mut spent = Vec::new();
        for n in counts {
            let mut app = app();
            let who = app.world_mut().spawn_empty().id();
            run(
                &mut app,
                who,
                &format!(
                    r#"
                      {n}.times {{ {wait} }}
                      Rubevy.ask("done", {n}.to_f)
                    "#
                ),
            );
            // the frames are paced, because `sleep 0` is a wait in *real time* (the VM's clock is
            // Bevy's) and a frame that takes microseconds moves it not at all: the control would
            // never get through its ten waits. `next_frame` does not care either way.
            for _ in 0..n + 20 {
                std::thread::sleep(Duration::from_millis(5));
                app.update();
            }
            let task = *app.world().entity(who).get::<ScriptTask>().expect("a task");
            let stats = app.world().resource::<ScriptWorld>().stats(&task);
            assert_eq!(said(&app, "done").len(), 1, "the script got through its {n} {what}s");
            spent.push((n, stats.instructions));
        }
        let (n0, i0) = spent[0];
        let (n1, i1) = spent[1];
        let each = (i1 - i0) as f64 / (n1 - n0) as f64;
        println!("{what}: {spent:?}");
        println!("one {what} costs {each:.1} instructions of the budget");
    }
}

/// **What a waiting task costs the frame it is waiting in.** The waking is an answer a task, so
/// a game that has every script waiting on a frame pays for that many answers at the head of
/// every tick; this is that, for two numbers of tasks, as `FrameStats::time_ns` and the
/// instructions beside it.
///
///     cargo test --release --test next_frame -- --ignored --nocapture
///
/// Run it on a quiet machine: the tick's wall clock is what it measures.
#[test]
#[ignore]
fn what_a_frame_of_waiting_tasks_costs() {
    for tasks in [100usize, 1000] {
        let mut app = app();
        for _ in 0..tasks {
            let entity = app.world_mut().spawn_empty().id();
            run(&mut app, entity, "loop { Rubevy.next_frame }");
        }
        // the scripts start, reach their loop and spread out over the first frames
        frames(&mut app, 20);
        let mut ticks: Vec<u64> = Vec::new();
        let mut insn: Vec<u64> = Vec::new();
        for _ in 0..60 {
            app.update();
            let stats = app.world().resource::<ScriptWorld>().last_frame();
            ticks.push(stats.time_ns);
            insn.push(stats.instructions);
        }
        ticks.sort_unstable();
        insn.sort_unstable();
        let median = ticks[ticks.len() / 2];
        println!(
            "{tasks} tasks waiting: tick {:?} at the median, {} instructions, {:.1} ns a task",
            Duration::from_nanos(median),
            insn[insn.len() / 2],
            median as f64 / tasks as f64
        );
    }
}
