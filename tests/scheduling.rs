//! Where a host's answering system sits in the frame, measured in frames.
//!
//! The script itself is the instrument: it reads `$rubevy[:frame]` before and after every
//! `Rubevy.ask(...).pop` and sends the difference back with a question nobody waits for. So the
//! numbers here are what a game's author would see from Ruby, not what the schedule looks like
//! from Rust.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, RubevyPlugin, RubevySet, Script, ScriptWorld};

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// The round trips the script measured, in frames.
#[derive(Resource, Default)]
struct Gaps(Vec<i64>);

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

/// The game's side of `Rubevy.ask`, answering everything at once — the shape
/// `docs/host-api.md` shows.
fn answer_requests(mut world: ResMut<ScriptWorld>, mut gaps: ResMut<Gaps>) {
    for request in world.take_requests() {
        if request.kind == "gap" {
            gaps.0.push(request.num_or(0, -1.0) as i64);
        }
        world.answer(&request, Answer::Nil);
    }
}

/// Six round trips, each one measured in frames by the script that makes it. The `gap` question
/// is not popped — an unanswered-for question is how a script tells the game something without
/// waiting — so it does not itself cost a frame.
const COUNTING: &str = r#"
  6.times do
    f0 = $rubevy[:frame]
    Rubevy.ask("ping").pop
    Rubevy.ask("gap", ($rubevy[:frame] - f0).to_f)
  end
"#;

/// The same measurement against the four kinds rubevy answers itself, which no host system is
/// involved in.
const COUNTING_COMPONENTS: &str = r#"
  e = Rubevy.entity
  6.times do
    f0 = $rubevy[:frame]
    e[:Transform]
    Rubevy.ask("gap", ($rubevy[:frame] - f0).to_f)
  end
"#;

/// A host system in [`RubevySet::Answer`] makes a round trip cost exactly one frame: the
/// question is asked and answered inside frame N, and the script has the answer in frame N+1.
#[test]
fn a_host_answering_in_the_answer_set_costs_one_frame() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Gaps>()
    .add_systems(Update, answer_requests.in_set(RubevySet::Answer));

    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(COUNTING));
    app.world_mut().spawn(Script::new(script));
    frames(&mut app, 16);

    let gaps = &app.world().resource::<Gaps>().0;
    assert_eq!(gaps, &vec![1, 1, 1, 1, 1, 1], "every round trip is one frame");
}

/// The questions rubevy answers itself cost **no** frame: the tick runs the scripts, answers the
/// reads they stopped on out of the world it is holding, and runs them again, so `e[:Transform]`
/// gives its value back in the line that asked for it. This was `1` for as long as the answer
/// was a system of its own at the end of the frame (`answer_components`, the 1.0 rubevy_games
/// measured for `entity[:Transform]`); it is the number this whole change is about.
///
/// A question the *game* answers is untouched and still costs one frame — the test above — which
/// is the point of keeping both here: the two kinds of question no longer cost the same, and the
/// difference is who answers.
#[test]
fn a_component_read_costs_no_frame() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Gaps>()
    // `MinimalPlugins` brings no `TransformPlugin`, so the test registers what it reads
    .register_type::<Transform>()
    .add_systems(Update, answer_requests.in_set(RubevySet::Answer));

    let script =
        app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(COUNTING_COMPONENTS));
    app.world_mut().spawn((Script::new(script), Transform::from_xyz(1.0, 2.0, 3.0)));
    frames(&mut app, 16);

    let gaps = &app.world().resource::<Gaps>().0;
    assert_eq!(gaps, &vec![0, 0, 0, 0, 0, 0], "a component read is answered inside the tick");
}

/// The set is what a host orders against, and it is ordered: `Deliver` before `Tick` before
/// `Answer`. A system in `Deliver` sees the requests of the *previous* frame — which is the
/// second placement that happens to cost one frame, and the reason `Answer` is the one
/// documented: only there does a system see the question the same frame it was asked, which is
/// what a host needs when the answer depends on the rest of the frame.
#[test]
fn a_host_answering_in_the_deliver_set_sees_the_previous_frame() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Gaps>()
    .add_systems(Update, answer_requests.in_set(RubevySet::Deliver));

    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(COUNTING));
    app.world_mut().spawn(Script::new(script));
    frames(&mut app, 16);

    let gaps = &app.world().resource::<Gaps>().0;
    assert_eq!(gaps, &vec![1, 1, 1, 1, 1, 1], "still one frame, one frame later");
}

