//! **A question whose answer is an action that takes frames** — `Held` and `App::hold_requests`.
//!
//!     cargo run --example walk_to
//!
//! The script says `walk_to(x, y)` and that line does not come back until the walking is over:
//!
//! ```ruby
//! walk_to 8, 0          # → true when it arrived, false where something stopped it
//! walk_to 0, 0
//! ```
//!
//! What makes that possible is one registration and two systems. The registration says that
//! `"walk_to"` waits on the entity that asked it, so the question arrives as a [`Held`] component
//! instead of in [`ScriptWorld::take_requests`]; the first system sees `Added<Held>` and sets the
//! thing walking; the second answers out of the component when it has arrived. Nothing here keeps
//! a table of who is waiting, and nothing here tidies up after a walker that is despawned or
//! given another script — that is what the component is for.
//!
//! The vocabulary is deliberately not a game's. rubevy has no idea what walking is: `Walking` and
//! `Where` below are this example's own components, and the only thing rubevy contributes is the
//! waiting.

use std::time::Duration;

use bevy::app::{AppExit, ScheduleRunnerPlugin};
use bevy::prelude::*;
use rubevy::{Answer, Held, HoldRequests, MrbAsset, RubevyPlugin, RubevySet, Script, ScriptWorld};

/// Where a walker is, in the flattest terms this example can get away with.
#[derive(Component, Debug)]
struct Where {
    at: Vec2,
}

/// What a walker is on its way to, and how fast. Put on by [`start_walking`] and taken off by
/// [`finish_walking`], so the two systems need no table between them either.
#[derive(Component, Debug)]
struct Walking {
    to: Vec2,
    /// Metres a second. It is this example's number and an app would read it from its own data;
    /// what it has to be is "slow enough that the walking takes several frames", which is the
    /// whole point of the thing being measured.
    speed: f32,
}

/// The one question this app holds. The string is written twice — here and in the script — the
/// way every `Rubevy.ask` kind is.
const WALK_TO: &str = "walk_to";

fn main() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(16))),
        bevy::asset::AssetPlugin::default(),
        bevy::log::LogPlugin::default(),
        RubevyPlugin::default(),
    ))
    // **the one line that makes `walk_to` a question that waits**
    .hold_requests(WALK_TO)
    .add_systems(Startup, spawn_a_walker)
    .add_systems(
        Update,
        (start_walking, walk, finish_walking, stop_when_the_script_ends)
            .chain()
            .in_set(RubevySet::Answer),
    );
    app.run();
}

fn spawn_a_walker(mut commands: Commands, mut assets: ResMut<Assets<MrbAsset>>) {
    let source = concat!(
        "def walk_to(x, y)\n",
        "  Rubevy.ask('walk_to', x, y).pop\n",   // the line that waits
        "end\n",
        "3.times do |i|\n",
        "  arrived = walk_to(8 - i * 4, 0)\n",
        "  puts \"leg #{i}: #{arrived ? 'there' : 'stopped'}\"\n",
        "end\n",
    );
    let opts = sabiruby_compiler::Options { filename: "walker.rb".into(), ..Default::default() };
    let bytes = sabiruby_compiler::compile(source.as_bytes(), &opts).expect("the script compiles");
    let script = assets.add(MrbAsset { bytes });
    commands.spawn((
        Where { at: Vec2::ZERO },
        Script::new(script).with_name("walker.rb"),
    ));
}

/// **A question has just arrived: start the action.** `Added<Held>` is the signal — the component
/// is put on the entity in `RubevySet::Tick`, so a system in `RubevySet::Answer` sees it on the
/// frame the script asked.
///
/// The arguments are read off the request without taking it: the question goes on waiting.
fn start_walking(mut commands: Commands, asked: Query<(Entity, &Held), Added<Held>>) {
    for (entity, held) in &asked {
        let Some(request) = held.waiting().iter().find(|r| r.kind == WALK_TO) else { continue };
        let to = Vec2::new(request.num_or(0, 0.0) as f32, request.num_or(1, 0.0) as f32);
        commands.entity(entity).insert(Walking { to, speed: 4.0 });
    }
}

/// The action itself, which knows nothing about scripts.
///
/// The last step lands **on** the target rather than near it, so that having arrived is a thing
/// the next system can say exactly instead of against a tolerance nobody could justify.
fn walk(time: Res<Time>, mut walkers: Query<(&mut Where, &Walking)>) {
    for (mut here, walking) in &mut walkers {
        let step = walking.speed * time.delta_secs();
        let left = walking.to - here.at;
        if left.length() <= step {
            here.at = walking.to;
        } else {
            here.at += left.normalize_or_zero() * step;
        }
    }
}

/// **The action is over: answer.** The component hands over the request that was waiting, the
/// script's `walk_to` comes back with `true`, and its next line runs on the next frame.
fn finish_walking(
    mut commands: Commands,
    mut scripts: ResMut<ScriptWorld>,
    mut walkers: Query<(Entity, &Where, &Walking, &mut Held)>,
) {
    for (entity, here, walking, mut held) in &mut walkers {
        if here.at != walking.to {
            continue;
        }
        held.answer(&mut scripts, WALK_TO, Answer::Bool(true));
        commands.entity(entity).remove::<Walking>();
        info!("arrived at {:?}", here.at);
    }
}

/// The script runs out of legs and ends; with nothing left to walk, so does the app.
fn stop_when_the_script_ends(
    walkers: Query<(), With<Where>>,
    done: Query<(), (With<Where>, With<rubevy::ScriptDone>)>,
    mut exit: MessageWriter<AppExit>,
) {
    if !walkers.is_empty() && walkers.iter().len() == done.iter().len() {
        exit.write(AppExit::Success);
    }
}
