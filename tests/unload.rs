//! `ScriptWorld::unload_programs`: the program of a replaced script goes back to the VM once
//! nothing runs it, and one that is still running, or that left a method behind, stays.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{replace_script, Answer, MrbAsset, RubevyPlugin, Script, ScriptDone, ScriptTask, ScriptWorld};

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn answer_all(mut world: ResMut<ScriptWorld>) {
    for r in world.take_requests() {
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
    .add_systems(Update, answer_all);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

fn replace(app: &mut App, entity: Entity, script: Script) {
    let mut queue = bevy::ecs::world::CommandQueue::default();
    let mut commands = Commands::new(&mut queue, app.world());
    replace_script(&mut commands, entity, script);
    queue.apply(app.world_mut());
}

fn unload(app: &mut App) -> usize {
    app.world_mut().resource_mut::<ScriptWorld>().unload_programs()
}

fn loaded(app: &App) -> usize {
    app.world().resource::<ScriptWorld>().loaded_programs()
}

#[test]
fn a_replaced_program_goes_back_and_a_running_one_stays() {
    let mut app = app();
    let (old, new) = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        (
            assets.add(compile("loop { Rubevy.ask('old').pop }")),
            assets.add(compile("loop { Rubevy.ask('new').pop }")),
        )
    };
    let entity = app.world_mut().spawn(Script::new(old)).id();
    frames(&mut app, 3);
    assert_eq!(loaded(&app), 1);
    assert_eq!(unload(&mut app), 0, "the program a task is running in stays");

    replace(&mut app, entity, Script::new(new));
    frames(&mut app, 3);
    assert_eq!(loaded(&app), 2);
    assert_eq!(unload(&mut app), 1, "the old program is handed back");
    assert_eq!(loaded(&app), 1);
    frames(&mut app, 3);
    assert_eq!(unload(&mut app), 0, "the new one is still running");
}

/// A `def` in the program leaves a method made of its code: the program stays after its task
/// has gone, because calling that method would run it.
#[test]
fn a_program_that_defined_a_method_stays() {
    let mut app = app();
    let (old, new) = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        (
            assets.add(compile("def greet; :hi; end\nloop { Rubevy.ask('old').pop }")),
            assets.add(compile("loop { Rubevy.ask(greet.to_s).pop }")),
        )
    };
    let entity = app.world_mut().spawn(Script::new(old)).id();
    frames(&mut app, 3);
    replace(&mut app, entity, Script::new(new));
    frames(&mut app, 3);
    assert_eq!(unload(&mut app), 0, "`greet` is still there to be called");
    assert_eq!(loaded(&app), 2);
}

/// A program handed back and then started again is loaded again, and runs.
#[test]
fn a_program_handed_back_is_loaded_again_when_it_is_started() {
    let mut app = app();
    let (a, b) = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        (
            assets.add(compile("loop { Rubevy.ask('a').pop }")),
            assets.add(compile("loop { Rubevy.ask('b').pop }")),
        )
    };
    let entity = app.world_mut().spawn(Script::new(a.clone())).id();
    frames(&mut app, 3);
    replace(&mut app, entity, Script::new(b));
    frames(&mut app, 3);
    assert_eq!(unload(&mut app), 1);
    replace(&mut app, entity, Script::new(a));
    frames(&mut app, 3);
    assert_eq!(loaded(&app), 2, "`a` is loaded again beside `b`");
    let e = app.world().entity(entity);
    assert!(e.contains::<ScriptTask>() && !e.contains::<ScriptDone>(), "the script started again runs");
}
