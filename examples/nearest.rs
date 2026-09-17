//! A question of the game's own, answered **inside the tick** (`ScriptWorld::answer_in_tick`):
//! "which plant is nearest to me". The closure is handed the world as it stands in
//! `RubevySet::Tick` and answers with an entity, so the script has it in the line that asked for
//! it — the same no-frame road a component read takes.
//!
//! The contrast is in the same run. `weather` is an ordinary `Rubevy.ask` answered by a system
//! in `RubevySet::Answer`, and every one of those costs its one frame. The script prints how
//! many frames it waited for each, so the output says it:
//!
//!     nearest: frame 5: the nearest plant is ... (frames waited: 0)
//!     nearest: frame 5: the weather is ...       (frames waited: 1)
//!
//!     cargo run --example nearest

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::asset::AssetPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, Request, RubevyPlugin, RubevySet, Script, ScriptEnded, ScriptWorld};

/// The things the script asks about. It is a component of the game's, and the closure below is
/// ordinary Bevy code — `&World`, not reflection — because the game knows its own types.
#[derive(Component)]
struct Plant;

/// The creature the script sits on, for the system that turns the plants around it.
#[derive(Component)]
struct Creature;

fn main() {
    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(16))),
            LogPlugin::default(),
            AssetPlugin::default(),
            RubevyPlugin::default(),
        ))
        // the script reads `plant[:Transform]` as well, and that goes through the type registry
        // (`MinimalPlugins` registers nothing; `DefaultPlugins` in a real game registers bevy's)
        .register_type::<Transform>()
        .add_systems(Startup, (install_answers, spawn))
        // **before the tick**: what the closure sees is the world as it stands when the scripts
        // run, so a system that moves the world belongs ahead of it
        .add_systems(Update, drift_the_plants.before(RubevySet::Tick))
        .add_systems(Update, (answer_the_weather, report_ended).in_set(RubevySet::Answer))
        .run();
}

/// The game's answer to `Rubevy.ask("nearest", me)`, given inside the tick.
///
/// What the closure may do is read: it is handed `&World` and the `Request`, and it answers with
/// the flat `Answer` — here the entity it found. What is *not* in that world is `ScriptWorld`
/// itself, which the tick has taken out for the length of its loop; so there is no `Vm` here and
/// nothing to spawn or write with, which is the whole reason this is safe to do in the middle of
/// a frame.
fn install_answers(mut scripts: ResMut<ScriptWorld>) {
    scripts.answer_in_tick(
        "nearest",
        Box::new(|world: &World, request: &Request| {
            let Some(from) = request.entity_arg(0).and_then(|e| world.get::<Transform>(e)) else {
                return Answer::Nil;
            };
            let mut best: Option<(Entity, f32)> = None;
            for entity in world.iter_entities() {
                let (Some(_), Some(at)) = (entity.get::<Plant>(), entity.get::<Transform>()) else {
                    continue;
                };
                let d = at.translation.distance(from.translation);
                if best.is_none_or(|(_, so_far)| d < so_far) {
                    best = Some((entity.id(), d));
                }
            }
            best.map_or(Answer::Nil, |(entity, _)| Answer::Entity(entity))
        }),
    );
}

fn spawn(mut commands: Commands, server: Res<AssetServer>) {
    for i in 0..5 {
        let angle = std::f32::consts::TAU * i as f32 / 5.0;
        commands.spawn((Plant, Transform::from_xyz(angle.cos() * 6.0, 0.0, angle.sin() * 6.0)));
    }
    let script: Handle<MrbAsset> = server.load("scripts/nearest.mrb");
    commands.spawn((
        Creature,
        Script::new(script).with_name("nearest"),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
}

/// The plants turn around the creature, so that "the nearest" is a different one as the script
/// goes on — and so that the answer really is this frame's and not a cached one.
fn drift_the_plants(mut plants: Query<&mut Transform, With<Plant>>, time: Res<Time>) {
    let by = Quat::from_rotation_y(time.delta_secs());
    for mut at in &mut plants {
        at.translation = by * at.translation;
    }
}

/// The other half of the contrast: an ordinary answering system. It sees the question on the
/// frame it was asked and answers it there, so the script wakes on the next frame — one frame,
/// which is what `RubevySet::Answer` is for.
fn answer_the_weather(mut scripts: ResMut<ScriptWorld>, frame: Res<bevy::diagnostic::FrameCount>) {
    for request in scripts.take_requests() {
        let sky = if frame.0.is_multiple_of(2) { "clear" } else { "rain" };
        scripts.answer(&request, Answer::Text(sky.to_string()));
    }
}

fn report_ended(mut ended: MessageReader<ScriptEnded>, mut exit: MessageWriter<AppExit>) {
    for e in ended.read() {
        info!("host: script on {:?} ended: {:?} {}", e.entity, e.status, e.value);
        exit.write(AppExit::Success);
    }
}
