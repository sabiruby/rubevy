//! What a game adds to the VM before its scripts run.
//!
//! rubevy installs its own host API (`Rubevy`, `Rubevy::Entity`, the prelude) when the plugin
//! builds the [`ScriptWorld`] resource, and the first script does not start until the first
//! `Update`. Between those two moments is `Startup`, and `ScriptWorld::vm` is public, so a game
//! reaches the VM in an ordinary system — no entry point of rubevy's is needed, and this test is
//! here to keep that true.
//!
//! What it installs is `sabiruby_serde::install_json`, which is what the next game wants, plus a
//! `define_fn` of its own. Neither is a dependency of rubevy: `sabiruby-serde` and sabiruby's
//! `macros` feature are dev-dependencies, so the library never carries them.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, RubevyPlugin, RubevySet, Script, ScriptWorld};
use sabiruby::Vm;

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// What the script sent back.
#[derive(Resource, Default)]
struct Said(Vec<String>);

fn answer_requests(mut world: ResMut<ScriptWorld>, mut said: ResMut<Said>) {
    for request in world.take_requests() {
        if let Some(text) = request.text(0) {
            said.0.push(text.to_string());
        }
        world.answer(&request, Answer::Nil);
    }
}

/// The game's own setup, at `Startup`: everything the scripts will find in the VM that rubevy
/// did not put there.
fn install_host_api(mut world: ResMut<ScriptWorld>) {
    let vm = &mut world.vm;
    // JSON, from the crate beside the VM. rubevy knows nothing about it.
    sabiruby_serde::install_json(vm);
    // and a native of the game's own, the other half of what a game does here
    let object = vm.core.object;
    vm.define_fn(object, "arena_size", |_vm: &mut Vm| -> f64 { 240.0 });
}

/// `JSON` and `arena_size` are there from the first line the script runs, which is what
/// `Startup` buys: nothing has to be guarded with `defined?`.
const USES_JSON: &str = r#"
  Rubevy.ask("said", JSON.generate({ "team" => "blue", "hp" => 7 }))
  Rubevy.ask("said", JSON.parse('{"x": 1.5, "tags": ["fast"]}').inspect)
  Rubevy.ask("said", arena_size.to_s)
"#;

#[test]
fn a_game_installs_json_and_a_native_of_its_own_at_startup() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Said>()
    .add_systems(Startup, install_host_api)
    .add_systems(Update, answer_requests.in_set(RubevySet::Answer));

    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(USES_JSON));
    app.world_mut().spawn(Script::new(script));
    for _ in 0..8 {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }

    let said = &app.world().resource::<Said>().0;
    assert_eq!(said.len(), 3, "the script got through all three lines: {said:?}");
    assert_eq!(said[0], r#"{"team":"blue","hp":7}"#);
    assert_eq!(said[1], r#"{"x" => 1.5, "tags" => ["fast"]}"#);
    assert_eq!(said[2], "240.0");
}

/// The resource is there before `Startup` runs, so a game may also do this without a system at
/// all — while it is still building the `App`. Same VM, same result.
#[test]
fn the_vm_is_reachable_while_the_app_is_still_being_built() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Said>()
    .add_systems(Update, answer_requests.in_set(RubevySet::Answer));

    sabiruby_serde::install_json(&mut app.world_mut().resource_mut::<ScriptWorld>().vm);

    let script = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile(r#"Rubevy.ask("said", JSON.generate([1, 2]))"#));
    app.world_mut().spawn(Script::new(script));
    for _ in 0..6 {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }

    assert_eq!(app.world().resource::<Said>().0, vec![String::from("[1,2]")]);
}
