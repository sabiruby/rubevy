//! What a component read costs, measured rather than reasoned about.
//!
//! Both tests are `#[ignore]`d: they are a measuring instrument, not an assertion about rubevy —
//! the numbers they print are of the machine they ran on. The run they were written for is in
//! `docs/worklog/2026-09-17-sync-reads.md`, before and after the reads became synchronous.
//!
//!     cargo test --release --test read_cost -- --ignored --nocapture
//!
//! `one_task_that_does_nothing_but_read` is the one that says what a round of the tick's answer
//! loop costs: every read it makes is a round of its own (the task parks, the loop answers it,
//! the loop runs the VM again), so the frame's wall clock divided by the reads in it is one
//! round — the re-entry into `task_run_limits`, the answer, and the Ruby the script spent asking.

use std::time::{Duration, Instant};

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, RubevyPlugin, Script, ScriptWorld};

#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
struct Hp {
    current: f32,
    max: f32,
}

/// The same two fields as a resource, so that the only difference between the two reads measured
/// below is the road, not the value being built in the VM.
#[derive(Resource, Reflect, Debug, Default)]
#[reflect(Resource)]
struct Score {
    points: f32,
    best: f32,
}

/// What the scripts reported (`Rubevy.ask` nobody pops).
#[derive(Resource, Default)]
struct Said(Vec<f64>);

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn answer_nil(mut world: ResMut<ScriptWorld>, mut said: ResMut<Said>) {
    for r in world.take_requests() {
        said.0.push(r.num_or(0, -1.0));
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
    .init_resource::<Said>()
    .register_type::<Hp>()
    .register_type::<Score>()
    .insert_resource(Score { points: 7.0, best: 10.0 })
    .add_systems(Update, answer_nil);
    app
}

/// Runs `n` frames and answers with the wall clock of each `app.update()`.
fn timed_frames(app: &mut App, n: usize) -> Vec<Duration> {
    let mut times = Vec::with_capacity(n);
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(4));
        let at = Instant::now();
        app.update();
        times.push(at.elapsed());
    }
    times
}

fn mean(times: &[Duration]) -> Duration {
    times.iter().sum::<Duration>() / times.len().max(1) as u32
}

fn median(times: &[Duration]) -> Duration {
    let mut sorted = times.to_vec();
    sorted.sort();
    sorted[sorted.len() / 2]
}

/// Twenty-four scripts, each reading its own component four times a frame — the shape of a world
/// where every creature looks at itself every frame, which is what the synchronous read is for.
///
/// The control is the same script with the four reads taken out, so the difference between the
/// two is the reads and nothing else.
#[test]
#[ignore]
fn twenty_four_tasks_reading_four_times_a_frame() {
    const READING: &str = r#"
      e = Rubevy.entity
      loop do
        4.times { e[:Hp] }
        sleep 0.004
      end
    "#;
    const CONTROL: &str = r#"
      e = Rubevy.entity
      loop do
        4.times { e }
        sleep 0.004
      end
    "#;

    for (what, src) in [("24 tasks x 4 reads a frame", READING), ("the same with no reads", CONTROL)] {
        let mut app = app();
        let asset = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
        for _ in 0..24 {
            app.world_mut().spawn((Script::new(asset.clone()), Hp { current: 7.0, max: 10.0 }));
        }
        // the first frames are the assets arriving and the scripts starting
        timed_frames(&mut app, 20);
        let times = timed_frames(&mut app, 200);
        println!(
            "{what}: mean {:?}, median {:?}, max {:?} per frame",
            mean(&times),
            median(&times),
            times.iter().max().unwrap()
        );
    }
}

/// One script that does nothing but read, with no `sleep` in it at all: it reads until the frame
/// it started in is over, and says how many reads it got through.
///
/// That count is the number of times the answer loop went round in one tick, because a read is
/// what ends a round. It is printed twice: once with the plugin's own limits (a budget of 200,000
/// instructions and a frame time of 8 ms, where the clock is what stops it) and once with the
/// frame time taken off (where the budget is).
#[test]
#[ignore]
fn one_task_that_does_nothing_but_read() {
    const READS_UNTIL_THE_FRAME_ENDS: &str = r#"
      e = Rubevy.entity
      f0 = $rubevy[:frame]
      n = 0
      loop do
        e[:Hp]
        n += 1
        break if $rubevy[:frame] != f0
      end
      Rubevy.ask("rounds", n.to_f)
    "#;

    for (what, frame_time) in
        [("budget 200,000 and 8 ms", Some(Duration::from_millis(8))), ("budget 200,000, no clock", None)]
    {
        let mut app = app();
        app.world_mut().resource_mut::<ScriptWorld>().frame_time = frame_time;
        let asset = app
            .world_mut()
            .resource_mut::<Assets<MrbAsset>>()
            .add(compile(READS_UNTIL_THE_FRAME_ENDS));
        app.world_mut().spawn((Script::new(asset), Hp { current: 7.0, max: 10.0 }));

        let times = timed_frames(&mut app, 40);
        let said = &app.world().resource::<Said>().0;
        let rounds = said.first().copied().unwrap_or(-1.0);
        let longest = times.iter().max().unwrap();
        println!(
            "{what}: {rounds} reads in the frame they started in, the longest frame was {longest:?} \
             ({:?} a read)",
            longest.div_f64(rounds.max(1.0))
        );
    }
}

/// What `Rubevy.resource(:Score)` costs beside `e[:Hp]`, measured the same way and in the same
/// run so that the machine is the same machine.
///
/// The two scripts are the same script with one line changed, and both read a two-field struct
/// of `f32`, so what the difference between the two counts is worth saying is the road and not
/// the value: a component read looks the name up (cached), finds the entity the script named and
/// reflects; a resource read looks the name up (cached), asks the world which entity holds that
/// resource — `Components::get_valid_id` and `World::resource_entities`, both a lookup by index —
/// and reflects the same way.
///
/// It prints reads-per-frame, which is rounds of the tick's answer loop, exactly as
/// [`one_task_that_does_nothing_but_read`] does; the frame's wall clock divided by that count is
/// one round.
#[test]
#[ignore]
fn a_resource_read_beside_a_component_read() {
    const COMPONENT: &str = r#"
      e = Rubevy.entity
      f0 = $rubevy[:frame]
      n = 0
      loop do
        e[:Hp]
        n += 1
        break if $rubevy[:frame] != f0
      end
      Rubevy.ask("rounds", n.to_f)
    "#;
    const RESOURCE: &str = r#"
      e = Rubevy.entity
      f0 = $rubevy[:frame]
      n = 0
      loop do
        Rubevy.resource(:Score)
        n += 1
        break if $rubevy[:frame] != f0
      end
      Rubevy.ask("rounds", n.to_f)
    "#;

    for (what, src) in [("a component read", COMPONENT), ("a resource read", RESOURCE)] {
        let mut app = app();
        let asset = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
        app.world_mut().spawn((Script::new(asset), Hp { current: 7.0, max: 10.0 }));

        let times = timed_frames(&mut app, 40);
        let said = &app.world().resource::<Said>().0;
        let rounds = said.first().copied().unwrap_or(-1.0);
        let longest = times.iter().max().unwrap();
        println!(
            "{what}: {rounds} in the frame they started in, the longest frame was {longest:?} \
             ({:?} a read)",
            longest.div_f64(rounds.max(1.0))
        );
    }
}
