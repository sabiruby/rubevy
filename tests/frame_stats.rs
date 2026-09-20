//! `ScriptWorld::last_frame()` — what one frame's tick did, for a HUD or a panel.
//!
//! The pair to `ScriptWorld::stats`, which is one script. Every number in it is one the tick
//! already had, so what is checked here is that each of them counts the thing its name says, in
//! the four situations where it would be easy for one of them to say the wrong thing: a queue
//! that overflowed, a frame that ran out of time and put questions off, a paused VM, and a
//! second VM beside the first.
//!
//! Nothing here asserts on wall-clock milliseconds as a *bound* — how long a tick takes is a
//! measurement, and `docs/verification/scale.md` is where measurements live. What the one test
//! that looks at the clock asserts is an ordering that holds on any machine: an answer that
//! sleeps 20 ms is the longest answer of the frame it is in.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, RubevyPlugin, RubevySet, Script, ScriptWorld};

/// The name tag of the second VM. An empty struct that implements nothing, which is what a game
/// writes (`tests/two_vms.rs`).
struct Mods;

/// Something for a script to read by name.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
struct Dial {
    value: f32,
}

/// One answer of the game's own, far longer than the `frame_time` the tests that use it set. It
/// sleeps rather than spins so that a machine with other work on it is not made to fight for the
/// core; the tick's clock sees the same either way (`tests/frame_time.rs` uses the same trick).
const ONE_SLOW_ANSWER: Duration = Duration::from_millis(20);

#[derive(Resource, Default)]
struct Said(Vec<(String, f64)>);

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn answer_nil(mut scripts: ResMut<ScriptWorld>, mut said: ResMut<Said>) {
    for r in scripts.take_requests() {
        said.0.push((r.kind.clone(), r.num_or(0, -1.0)));
        scripts.answer(&r, Answer::Nil);
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Said>()
    .register_type::<Dial>()
    .add_systems(Update, answer_nil.in_set(RubevySet::Answer));
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

fn spawn(app: &mut App, src: &str) -> Entity {
    let asset = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
    app.world_mut().spawn((Script::new(asset), Dial { value: 7.0 })).id()
}

/// Runs frames until the tick answered something, so that a test about a working frame is not
/// looking at one of the frames a script spends starting up. Answers the frame it stopped on.
fn until_answers(app: &mut App, n: usize) -> u32 {
    for _ in 0..n {
        frames(app, 1);
        let f = app.world().resource::<ScriptWorld>().last_frame();
        if f.reflect_answers > 0 {
            return f.frame;
        }
    }
    panic!("no frame answered a question in {n} frames");
}

// ---------------------------------------------------------------------------------------------

/// The plain case: a script that reads its own component three times and then sleeps. Three
/// reads are three answers and four rounds of the loop (three that answered somebody and one
/// that found nothing left to answer), and the numbers about the frame as a whole are there.
#[test]
fn what_a_working_frame_counts() {
    let mut app = app();
    spawn(
        &mut app,
        r#"
          e = Rubevy.entity
          loop do
            a = e[:Dial][:value]
            b = e[:Dial][:value]
            c = e[:Dial][:value]
            Rubevy.ask("sum", a + b + c)
            sleep 0.5
          end
        "#,
    );
    let at = until_answers(&mut app, 20);

    let f = app.world().resource::<ScriptWorld>().last_frame();
    assert_eq!(f.frame, at, "the frame it is of is bevy's frame count");
    assert_eq!(f.reflect_answers, 3, "three component reads, answered inside the tick");
    assert_eq!(f.in_tick_answers, 0, "the game registered no in-tick answerer");
    assert_eq!(f.rounds, 4, "three rounds that answered somebody, and one that found nobody");
    assert!(f.instructions > 0, "the scripts ran: {}", f.instructions);
    assert!(f.instructions < app.world().resource::<ScriptWorld>().budget, "and did not run out");
    assert_eq!(f.carried_reflect, 0, "nothing was put off");
    assert_eq!(f.carried_in_tick, 0);
    assert_eq!(f.dropped, 0, "nothing was published at all");
    assert!(f.time_ns > 0, "the tick took some time");
    assert!(f.more_to_run, "the script is sleeping, which is something that will run again");
    assert_eq!(f.loaded_programs, 1, "one program, whatever entities run it");

    // the same field after a frame in which the script is asleep: the counters are of *that*
    // frame, not of everything since the VM started
    frames(&mut app, 1);
    let idle = app.world().resource::<ScriptWorld>().last_frame();
    assert!(idle.frame > f.frame, "a later frame");
    assert_eq!(idle.reflect_answers, 0, "it answered nobody");
    assert_eq!(idle.rounds, 1, "one round, which ran nothing and answered nobody");
}

/// A `ScriptWorld` nothing has ticked yet answers the default rather than nothing: a HUD that
/// reads it on the first frame of the app gets zeroes, not a panic and not an `Option` to unwrap.
#[test]
fn before_the_first_tick_it_is_all_zeroes() {
    let app = app();
    let f = app.world().resource::<ScriptWorld>().last_frame();
    assert_eq!(f.frame, 0);
    assert_eq!(f.instructions, 0);
    assert_eq!(f.rounds, 0);
    assert!(!f.more_to_run, "there is no script in the VM yet");
    assert_eq!(f.longest_answer_ns, None);
}

/// What a full queue dropped is in the frame that dropped it, and in no other frame: the field
/// is the difference of `ScriptWorld::dropped`, which itself only ever climbs.
#[test]
fn what_a_full_queue_dropped_is_in_the_frame_that_dropped_it() {
    let mut app = app();
    spawn(
        &mut app,
        r#"
          q = Rubevy.subscribe(:belt, limit: 4)
          Rubevy.ask("ready", 0.0)
          sleep 3600                      # never reads, so the queue overflows
        "#,
    );
    for _ in 0..20 {
        frames(&mut app, 1);
        if app.world().resource::<ScriptWorld>().subscriptions() == 1 {
            break;
        }
    }
    assert_eq!(app.world().resource::<ScriptWorld>().subscriptions(), 1, "subscribed");
    assert_eq!(app.world().resource::<ScriptWorld>().last_frame().dropped, 0);

    // ten into a queue of four: six fall out
    {
        let mut scripts = app.world_mut().resource_mut::<ScriptWorld>();
        for i in 0..10 {
            scripts.publish(None, "belt", Answer::Num(i as f64));
        }
    }
    frames(&mut app, 1);
    let f = app.world().resource::<ScriptWorld>().last_frame();
    assert_eq!(f.dropped, 6, "ten published into a queue of four");
    assert_eq!(app.world().resource::<ScriptWorld>().dropped(), 6, "and the VM's total agrees");

    frames(&mut app, 1);
    assert_eq!(
        app.world().resource::<ScriptWorld>().last_frame().dropped,
        0,
        "the next frame dropped nothing of its own, though the VM's total still says six"
    );
    assert_eq!(app.world().resource::<ScriptWorld>().dropped(), 6);
}

/// A frame that runs out of time says how many questions it put off, and the longest answer it
/// made is the one that used the time up.
///
/// Three scripts park on the game's slow answer in the same round. The tick makes one of them —
/// a late tick always makes one — and the other two are carried, in the order they were asked
/// in, to be answered before anything the next frame asks.
#[test]
fn a_frame_that_ran_out_of_time_says_what_it_put_off() {
    let mut app = app();
    app.world_mut().resource_mut::<ScriptWorld>().answer_in_tick(
        "slow",
        Box::new(|_world, _request| {
            std::thread::sleep(ONE_SLOW_ANSWER);
            Answer::Num(1.0)
        }),
    );
    for i in 0..3 {
        spawn(
            &mut app,
            &format!(
                r#"
                  Rubevy.ask("slow", {i}.0).pop
                  Rubevy.ask("done", {i}.0)
                  sleep 3600
                "#
            ),
        );
    }

    // the frame in which all three park on "slow" and one of them is answered
    let mut carried_on_the_late_frame = 0;
    let mut longest = 0u64;
    for _ in 0..20 {
        frames(&mut app, 1);
        let f = app.world().resource::<ScriptWorld>().last_frame();
        if f.in_tick_answers > 0 {
            carried_on_the_late_frame = f.carried_in_tick;
            longest = f.longest_answer_ns.expect("the default frame_time means a clock was read");
            assert_eq!(f.in_tick_answers, 1, "a late tick makes one answer of the game's kinds");
            break;
        }
    }
    assert_eq!(carried_on_the_late_frame, 2, "the two it did not reach went to the next frame");
    assert!(
        longest >= ONE_SLOW_ANSWER.as_nanos() as u64,
        "the longest answer of that frame was the 20 ms closure, not {longest} ns"
    );

    // and they are answered, one a frame, each frame being late for the same reason
    for _ in 0..20 {
        frames(&mut app, 1);
        if app.world().resource::<Said>().0.iter().filter(|(k, _)| k == "done").count() == 3 {
            break;
        }
    }
    let done: Vec<f64> =
        app.world().resource::<Said>().0.iter().filter(|(k, _)| k == "done").map(|(_, n)| *n).collect();
    assert_eq!(done, vec![0.0, 1.0, 2.0], "in the order they asked");
    let f = app.world().resource::<ScriptWorld>().last_frame();
    assert_eq!(f.carried_in_tick, 0, "and nothing is left over at the end");
}

/// With no `frame_time` there is no deadline, so the tick reads no clock while it answers and
/// `longest_answer_ns` has nothing to say — which it says with `None` rather than a zero that
/// would read as "an answer took no time at all". Everything else is still counted, and the
/// questions that a bounded tick would have carried are all answered in the tick that asked.
#[test]
fn with_no_frame_time_the_longest_answer_is_not_measured() {
    let mut app = app();
    app.world_mut().resource_mut::<ScriptWorld>().frame_time = None;
    spawn(
        &mut app,
        r#"
          e = Rubevy.entity
          loop do
            Rubevy.ask("v", e[:Dial][:value])
            sleep 0.5
          end
        "#,
    );
    until_answers(&mut app, 20);

    let f = app.world().resource::<ScriptWorld>().last_frame();
    assert_eq!(f.longest_answer_ns, None, "no deadline, no clock, nothing to report");
    assert_eq!(f.reflect_answers, 1);
    assert_eq!(f.carried_reflect, 0);
    assert!(f.time_ns > 0, "the wall clock of the tick is taken either way");
}

/// A paused VM (`budget = 0`) is still ticked, and what it says of the frame is that it ran
/// nothing — with `more_to_run` telling a HUD the difference between a VM that is paused and one
/// whose scripts have all ended.
#[test]
fn a_paused_frame_ran_nothing_and_still_has_something_to_run() {
    let mut app = app();
    spawn(
        &mut app,
        r#"
          e = Rubevy.entity
          loop do
            Rubevy.ask("v", e[:Dial][:value])
            sleep 0.05
          end
        "#,
    );
    until_answers(&mut app, 20);

    app.world_mut().resource_mut::<ScriptWorld>().budget = 0;
    frames(&mut app, 3);
    let f = app.world().resource::<ScriptWorld>().last_frame();
    assert_eq!(f.instructions, 0, "not one instruction ran");
    assert_eq!(f.rounds, 0, "the loop ends before its first round, which is what the pause is");
    assert_eq!(f.reflect_answers, 0);
    assert_eq!(f.in_tick_answers, 0);
    assert_eq!(f.longest_answer_ns, None, "no answer was made, so none was the longest");
    assert!(f.more_to_run, "the scripts are still there, waiting for the budget");
    assert_eq!(f.loaded_programs, 1, "which is not a number about the frame");

    // and it goes on where it left off
    app.world_mut().resource_mut::<ScriptWorld>().budget = 200_000;
    let after = until_answers(&mut app, 20);
    assert!(after > f.frame);
    assert!(app.world().resource::<ScriptWorld>().last_frame().instructions > 0);
}

/// A VM whose scripts have all ended says so: nothing left that would run, however many frames
/// go by. This is the other side of the pause above — the two frames look the same in every
/// field but this one.
#[test]
fn a_vm_whose_scripts_have_ended_has_nothing_more_to_run() {
    let mut app = app();
    spawn(&mut app, r#"Rubevy.ask("once", 1.0)"#);
    for _ in 0..20 {
        frames(&mut app, 1);
        if !app.world().resource::<ScriptWorld>().last_frame().more_to_run {
            break;
        }
    }
    let f = app.world().resource::<ScriptWorld>().last_frame();
    assert!(!f.more_to_run, "the one script ran to its end");
    assert_eq!(f.loaded_programs, 1, "the program it ran is still loaded");
}

/// Two VMs, two sets of numbers. The second VM's tick is its own: the first one's instructions,
/// answers and programs are not in it, which is the same promise `budget` and `ScriptStats`
/// make.
#[test]
fn the_second_vm_has_its_own_frame() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
        RubevyPlugin::<Mods>::for_vm("assets"),
    ))
    .init_resource::<Said>()
    .register_type::<Dial>()
    .add_systems(Update, answer_nil.in_set(RubevySet::Answer));

    // the first VM reads a component every frame; the second one sleeps through the run
    let busy = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(
        r#"
          e = Rubevy.entity
          loop do
            Rubevy.ask("v", e[:Dial][:value])
            sleep 0.004
          end
        "#,
    ));
    app.world_mut().spawn((Script::new(busy), Dial { value: 1.0 }));
    let quiet = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile("sleep 3600"));
    app.world_mut().spawn(Script::<Mods>::for_vm(quiet));

    let mut saw = false;
    for _ in 0..20 {
        frames(&mut app, 1);
        let first = app.world().resource::<ScriptWorld>().last_frame();
        let mods = app.world().resource::<ScriptWorld<Mods>>().last_frame();
        assert_eq!(mods.reflect_answers, 0, "the second VM's script asks nothing");
        assert_eq!(mods.loaded_programs, 1, "and holds one program of its own, not two");
        if first.reflect_answers > 0 {
            assert!(first.instructions > mods.instructions, "the busy VM ran more");
            assert_eq!(first.frame, mods.frame, "both ticks are of the same bevy frame");
            assert!(mods.more_to_run, "the sleeper will run again");
            saw = true;
            break;
        }
    }
    assert!(saw, "the first VM's script never read anything");

    // pausing one does not quiet the other
    app.world_mut().resource_mut::<ScriptWorld<Mods>>().budget = 0;
    let mut first_still_runs = false;
    for _ in 0..20 {
        frames(&mut app, 1);
        assert_eq!(
            app.world().resource::<ScriptWorld<Mods>>().last_frame().rounds,
            0,
            "the paused VM is the paused one"
        );
        if app.world().resource::<ScriptWorld>().last_frame().reflect_answers > 0 {
            first_still_runs = true;
            break;
        }
    }
    assert!(first_still_runs);
}

/// `loaded_programs` is the VM's standing count and not the frame's: a second distinct program
/// makes it two, and a second entity running one of them makes it neither three nor two again.
#[test]
fn loaded_programs_counts_programs_and_not_entities() {
    let mut app = app();
    let one = compile(r#"loop { Rubevy.ask("a", 1.0); sleep 0.01 }"#);
    let two = compile(r#"loop { Rubevy.ask("b", 2.0); sleep 0.01 }"#);
    let a = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(one);
    let b = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(two);
    app.world_mut().spawn(Script::new(a.clone()));
    app.world_mut().spawn(Script::new(a));
    app.world_mut().spawn(Script::new(b));
    frames(&mut app, 8);

    let f = app.world().resource::<ScriptWorld>().last_frame();
    assert_eq!(f.loaded_programs, 2, "three entities, two programs");
    assert_eq!(f.loaded_programs, app.world().resource::<ScriptWorld>().loaded_programs());
}
