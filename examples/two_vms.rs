//! Two VMs in one app: the game's, and one for mods.
//!
//! The game keeps `RubevyPlugin::default()` — the VM it always had, reading `assets/scripts`.
//! The mods get a second one under a name tag, reading `assets/mods` and nothing else:
//!
//! ```ignore
//! struct Mods;
//! app.add_plugins(RubevyPlugin::default())
//!    .add_plugins(RubevyPlugin::<Mods>::for_vm("assets/mods"));
//! ```
//!
//! What this shows is the three things a mod cannot do to the game, printed as it finds them:
//!
//! 1. **It cannot see the game's classes or globals.** `$world_seed` and `Boss` are in the
//!    game's heap; the mod's VM has a heap of its own and reads neither.
//! 2. **Its runaway `loop {}` does not cost the game its frame.** The budget is per VM, so the
//!    mod spends its own and the game's script keeps its beat. (That is also the cost of a
//!    second VM: the worst case per frame is the sum of the VMs' `frame_time`s, which is why
//!    the mod's is set low here.)
//! 3. **It cannot `require` the game's files by name.** `require` goes through the VM's own
//!    load path, and the mod's is `assets/mods`. Its own file loads; the game's does not.
//!    (By *name*: a name that begins with `./`, `../` or `/` skips the load path altogether
//!    and is read straight off the file system, so this is a separation and not a jail —
//!    `docs/host-api.md`, "What this is not".)
//!
//!     cargo run --example two_vms
//!
//! It is headless and ends by itself after a few seconds.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::asset::AssetPlugin;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, RubevyPlugin, RubevySet, Script, ScriptTask, ScriptWorld};

/// The second VM's name tag. An empty struct that implements nothing is all one ever is: it
/// names a VM in the type system and carries no data.
struct Mods;

/// How long the mod is left spinning before the app says what it found.
const RUN_FOR: Duration = Duration::from_secs(3);

/// What the two VMs' scripts have said so far.
#[derive(Resource, Default)]
struct Report {
    /// `1` for `$world_seed`, `2` for `Boss`, `3` for both — from the game's own VM.
    game_peek: Option<f64>,
    /// the same number, from the mod's VM
    mod_peek: Option<f64>,
    /// what the game's script got out of `require "helper"`
    game_required: Option<String>,
    /// what the mod's script got out of the same line
    mod_required: Option<String>,
    /// and out of `require` of a file of its own
    mod_own_required: Option<String>,
    /// how many times the game's script has come round its `sleep 0.016`
    beats: u32,
    /// the entity the mod's script runs on, once it has started
    mod_entity: Option<Entity>,
}

fn main() {
    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(16))),
            LogPlugin::default(),
            AssetPlugin::default(),
            // the app's first VM: the game's own, unchanged and untagged
            RubevyPlugin::default(),
            // and the second, under the name tag, with an asset root of its own
            RubevyPlugin::<Mods>::for_vm("assets/mods"),
        ))
        .init_resource::<Report>()
        .add_systems(Startup, (start_the_game, give_the_mods_less))
        .add_systems(Update, answer_the_game.in_set(RubevySet::Answer))
        .add_systems(Update, answer_the_mods.in_set(RubevySet::<Mods>::answer()))
        .add_systems(Update, (start_the_mod_once_the_game_is_up, say_what_happened))
        .run();
}

fn start_the_game(mut commands: Commands, server: Res<AssetServer>) {
    let game: Handle<MrbAsset> = server.load("scripts/game.mrb");
    commands.spawn(Script::new(game).with_name("game"));
}

/// The second VM's share of the frame. Both numbers mean for `Mods` exactly what they mean for
/// the game's VM — they are simply that VM's own: a tenth of the instructions and an eighth of
/// the time. Nothing here caps the two VMs together; a frame's worst case is the sum.
fn give_the_mods_less(mut mods: ResMut<ScriptWorld<Mods>>) {
    mods.budget = 20_000; // the game's default is 200_000
    mods.frame_time = Some(Duration::from_millis(1)); // the game's default is 8 ms
}

/// The mod starts only once the game's script has said it made `$world_seed` and `Boss`, so
/// that "the mod did not see them" cannot be "the mod looked too early".
fn start_the_mod_once_the_game_is_up(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut report: ResMut<Report>,
) {
    if report.mod_entity.is_some() || report.game_peek.is_none() {
        return;
    }
    // `Script::<Mods>::for_vm` rather than `Script::new`: which VM a script belongs to is part
    // of its type, so this cannot be spawned into the game's VM by mistake
    let m: Handle<MrbAsset> = server.load("mods/mod.mrb");
    report.mod_entity = Some(commands.spawn(Script::<Mods>::for_vm(m).with_name("mod")).id());
}

fn answer_the_game(mut world: ResMut<ScriptWorld>, mut report: ResMut<Report>) {
    for request in world.take_requests() {
        match request.kind.as_str() {
            "peek" => report.game_peek = Some(request.num_or(0, -1.0)),
            "required" => report.game_required = request.text(0).map(String::from),
            "beat" => report.beats += 1,
            _ => {}
        }
        world.answer(&request, Answer::Nil);
    }
}

fn answer_the_mods(mut world: ResMut<ScriptWorld<Mods>>, mut report: ResMut<Report>) {
    for request in world.take_requests() {
        match request.kind.as_str() {
            "peek" => report.mod_peek = Some(request.num_or(0, -1.0)),
            "require" => report.mod_required = request.text(0).map(String::from),
            "own_require" => report.mod_own_required = request.text(0).map(String::from),
            _ => {}
        }
        world.answer(&request, Answer::Nil);
    }
}

/// After [`RUN_FOR`], the three findings and out.
fn say_what_happened(
    time: Res<Time>,
    report: Res<Report>,
    mods: Res<ScriptWorld<Mods>>,
    tasks: Query<&ScriptTask<Mods>>,
    frames: Res<bevy::diagnostic::FrameCount>,
    mut exit: MessageWriter<AppExit>,
) {
    if time.elapsed() < RUN_FOR {
        return;
    }
    let seconds = time.elapsed_secs();
    let spun = report
        .mod_entity
        .and_then(|e| tasks.get(e).ok())
        .map(|task| mods.stats(task).instructions)
        .unwrap_or(0);

    let names = |n: f64| match n as u32 {
        0 => "neither".to_string(),
        1 => "$world_seed".to_string(),
        2 => "Boss".to_string(),
        _ => "$world_seed and Boss".to_string(),
    };
    println!("\n--- two VMs, {seconds:.1} s, {} frames ---", frames.0);
    println!(
        "1. the heap: the game's VM sees {}; the mod's VM sees {}",
        names(report.game_peek.unwrap_or(-1.0)),
        names(report.mod_peek.unwrap_or(-1.0)),
    );
    println!(
        "2. the budget: the mod has been in `loop {{}}` without ever sleeping and has run {spun} \
         instructions, and the game's script still beat {} times in {seconds:.1} s (it asks for \
         one every 16 ms, so about {})",
        report.beats,
        (seconds / 0.016) as u32,
    );
    println!(
        "3. the load path: the game's `require \"helper\"` gave it {:?}; the mod's same line \
         gave it {:?}, while its own `require \"mod_helper\"` gave it {:?}",
        report.game_required.as_deref().unwrap_or("nothing yet"),
        report.mod_required.as_deref().unwrap_or("nothing yet"),
        report.mod_own_required.as_deref().unwrap_or("nothing yet"),
    );
    exit.write(AppExit::Success);
}
