//! A paused VM does not age. `ScriptWorld::budget = 0` runs no instruction, and while it is
//! zero the plugin does not move mruby-task's clock on either — so a script that was half way
//! through a `sleep` when the pause began is still half way through it when the pause ends.
//!
//! The script is the instrument again: it tells the game when it started sleeping and when it
//! woke, with questions it does not wait for, and the game writes down the wall clock of each.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, RubevyPlugin, RubevySet, Script, ScriptWorld};

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// `(what the script said, the frame it reached the game on, the seconds on Bevy's clock)`.
#[derive(Resource, Default)]
struct Marks(Vec<(i64, u32, f32)>);

fn answer_requests(
    mut world: ResMut<ScriptWorld>,
    mut marks: ResMut<Marks>,
    frame: Res<bevy::diagnostic::FrameCount>,
    time: Res<Time>,
) {
    for request in world.take_requests() {
        if request.kind == "mark" {
            marks.0.push((request.num_or(0, -1.0) as i64, frame.0, time.elapsed_secs()));
        }
        world.answer(&request, Answer::Nil);
    }
}

/// 0 before the sleep, 1 after it. Neither question is popped, so neither costs the script a
/// frame — they are how it says something to the game rather than asks it.
const SLEEPER: &str = r#"
  Rubevy.ask("mark", 0.0)
  sleep 0.1
  Rubevy.ask("mark", 1.0)
"#;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Marks>()
    .add_systems(Update, answer_requests.in_set(RubevySet::Answer));
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

fn start(app: &mut App, src: &str) {
    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
    app.world_mut().spawn(Script::new(script));
}

/// A hundred paused frames — about a fifth of a second of real time, twice the sleep — and the
/// script still has its whole `sleep 0.1` in front of it when the budget comes back.
#[test]
fn a_pause_does_not_spend_a_sleep() {
    let mut app = app();
    start(&mut app, SLEEPER);

    // let the script start and reach the `sleep`
    frames(&mut app, 4);
    assert_eq!(app.world().resource::<Marks>().0.len(), 1, "it said it was starting");

    // pause, and let a lot of real time pass
    app.world_mut().resource_mut::<ScriptWorld>().budget = 0;
    frames(&mut app, 100);
    assert_eq!(app.world().resource::<Marks>().0.len(), 1, "nothing ran while it was paused");

    // resume
    app.world_mut().resource_mut::<ScriptWorld>().budget = 200_000;
    let resumed_at = app.world().resource::<Time>().elapsed_secs();
    let resumed_frame = app.world().resource::<bevy::diagnostic::FrameCount>().0;

    frames(&mut app, 120);
    let marks = &app.world().resource::<Marks>().0;
    assert_eq!(marks.len(), 2, "it woke: {marks:?}");
    let (_, woke_frame, woke_at) = marks[1];

    // the point: it did not wake on the first frame after the pause
    assert!(
        woke_frame > resumed_frame + 1,
        "woke on frame {woke_frame}, one after the resume at {resumed_frame} — the clock ran on \
         while it was paused"
    );
    // and it slept for something like the 0.1 s it asked for, counted from the resume. Half of
    // it is the assertion, because how much of the sleep was already spent before the pause
    // depends on how long the first four frames took.
    let slept = woke_at - resumed_at;
    assert!(slept > 0.05, "it slept {slept} s after the resume, not most of its 0.1 s");
}

/// The control: with no pause the same script wakes about 0.1 s after it started, which is what
/// says the measurement above is looking at the right thing.
#[test]
fn without_a_pause_it_sleeps_the_time_it_asked_for() {
    let mut app = app();
    start(&mut app, SLEEPER);
    frames(&mut app, 120);

    let marks = &app.world().resource::<Marks>().0;
    assert_eq!(marks.len(), 2, "it woke: {marks:?}");
    let slept = marks[1].2 - marks[0].2;
    assert!((0.05..0.5).contains(&slept), "slept {slept} s, not about 0.1");
}
