//! One program, one irep: a thousand entities running the same `.mrb` load it once, and what
//! makes two scripts the same program is the program — not the asset it arrived in.
//!
//! The VM counts its ireps in `Vm::ireps`, which is how these tests see the difference. The
//! number is also what a game watching an editor's Apply button would watch: SabiRuby has no way
//! to give an irep back, so every *new* text is one more for the life of the app, and the same
//! text twice is none.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{replace_script, Answer, MrbAsset, RubevyPlugin, Script, ScriptWorld};

/// What the scripts asked, by kind, in the order they asked it.
#[derive(Resource, Default)]
struct Seen {
    kinds: Vec<String>,
    nums: Vec<f64>,
}

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn answer_all(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>) {
    for r in world.take_requests() {
        seen.kinds.push(r.kind.clone());
        seen.nums.push(r.num_or(0, f64::NAN));
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
    .init_resource::<Seen>()
    .add_systems(Update, answer_all);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

fn ireps(app: &App) -> usize {
    app.world().resource::<ScriptWorld>().vm.ireps.len()
}

/// The count that matters: what one entity costs the VM and what a hundred cost it are the same
/// number, because the program is loaded once and the hundred tasks are spawned from that one
/// irep.
#[test]
fn a_hundred_entities_of_one_program_load_it_once() {
    let mut app = app();
    let handle = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile("loop { Rubevy.ask('alive').pop }"));
    // the asset has to have arrived before `start_scripts` counts for anything
    app.update();
    let before = ireps(&app);

    app.world_mut().spawn(Script::new(handle.clone()));
    frames(&mut app, 2);
    let with_one = ireps(&app) - before;
    assert!(with_one > 0, "the first script did load the program");

    for _ in 0..99 {
        app.world_mut().spawn(Script::new(handle.clone()));
    }
    frames(&mut app, 2);
    assert_eq!(ireps(&app) - before, with_one, "the other ninety-nine loaded nothing");
    assert_eq!(app.world().resource::<ScriptWorld>().loaded_programs(), 1);
    // and all hundred really are running
    let seen = app.world().resource::<Seen>();
    assert!(seen.kinds.iter().filter(|k| *k == "alive").count() >= 100);
}

/// The same program in another asset is the same program. This is the shape both sample games
/// have: `Assets::add` hands out a fresh id for every compile, and garden compiles a species'
/// file again for every animal born into it — keyed by asset id, a herd would be a herd of
/// ireps.
#[test]
fn the_same_text_in_another_asset_is_the_same_irep() {
    let mut app = app();
    let src = "loop { Rubevy.ask('alive').pop }";
    let (a, b) = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        (assets.add(compile(src)), assets.add(compile(src)))
    };
    assert_ne!(a.id(), b.id(), "two assets, as a game's two compiles are");
    app.update();
    let before = ireps(&app);

    app.world_mut().spawn(Script::new(a));
    frames(&mut app, 2);
    let with_one = ireps(&app) - before;

    app.world_mut().spawn(Script::new(b));
    frames(&mut app, 2);
    assert_eq!(ireps(&app) - before, with_one, "the second asset loaded nothing");
    assert_eq!(app.world().resource::<ScriptWorld>().loaded_programs(), 1);
}

/// Two tasks of one irep do not reach each other through it. The script mutates a string
/// literal, which is the one thing an irep holds that a script can change: it is copied out of
/// the pool where it is reached, so each task gets its own — both report six characters, not six
/// and nine.
#[test]
fn tasks_that_share_an_irep_do_not_share_its_literals() {
    let mut app = app();
    let handle = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(
        r#"
        s = "abc"
        3.times { s << "!" }
        Rubevy.ask('len', s.length.to_f)
        loop { Rubevy.ask('alive').pop }
        "#,
    ));
    app.update();
    for _ in 0..5 {
        app.world_mut().spawn(Script::new(handle.clone()));
    }
    frames(&mut app, 6);

    let seen = app.world().resource::<Seen>();
    let lens: Vec<f64> = seen
        .kinds
        .iter()
        .zip(&seen.nums)
        .filter(|(k, _)| *k == "len")
        .map(|(_, n)| *n)
        .collect();
    assert_eq!(lens.len(), 5, "all five reported");
    assert!(lens.iter().all(|n| *n == 6.0), "each task had its own string: {lens:?}");
    assert_eq!(app.world().resource::<ScriptWorld>().loaded_programs(), 1);
}

/// A script given another text runs the other text. The map is keyed by the program, so a
/// changed program cannot be found in it — there is nothing to invalidate, and no window in
/// which the old irep could be handed out for the new text.
#[test]
fn a_script_replaced_with_another_text_runs_it() {
    let mut app = app();
    let (old, new) = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        (
            assets.add(compile("loop { Rubevy.ask('old').pop }")),
            assets.add(compile("loop { Rubevy.ask('new').pop }")),
        )
    };
    let entity = app.world_mut().spawn(Script::new(old)).id();
    frames(&mut app, 8);
    assert!(app.world().resource::<Seen>().kinds.iter().any(|k| k == "old"));

    let mut queue = bevy::ecs::world::CommandQueue::default();
    let mut commands = Commands::new(&mut queue, app.world());
    replace_script(&mut commands, entity, Script::new(new));
    queue.apply(app.world_mut());
    frames(&mut app, 8);

    assert!(app.world().resource::<Seen>().kinds.iter().any(|k| k == "new"), "the new text runs");
    assert_eq!(app.world().resource::<ScriptWorld>().loaded_programs(), 2);
}

/// A file changed on disk and reloaded: the *same* asset, other bytes. A script started after
/// the reload runs what the file says now.
#[test]
fn a_reloaded_asset_starts_the_new_code() {
    let mut app = app();
    let handle = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile("loop { Rubevy.ask('old').pop }"));
    app.world_mut().spawn(Script::new(handle.clone()));
    frames(&mut app, 8);
    assert!(app.world().resource::<Seen>().kinds.iter().any(|k| k == "old"));

    // what the asset server does when the file underneath a handle changes: the asset behind
    // that id is another one now, and the handle is the one it always was
    let id = handle.id();
    app.world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .insert(id, compile("loop { Rubevy.ask('new').pop }"))
        .expect("the asset is there to replace");
    app.world_mut().spawn(Script::new(handle));
    frames(&mut app, 8);

    assert!(
        app.world().resource::<Seen>().kinds.iter().any(|k| k == "new"),
        "the script started after the reload runs the reloaded text"
    );
}

/// What an editor's Apply button costs. Applying the same text over and over costs the VM
/// nothing after the first time; applying a *different* text each time is a new irep each time,
/// which the VM cannot give back (SabiRuby 0.5.2 never removes from `Vm::ireps`) — the number is
/// here so that it is a number and not a suspicion.
#[test]
fn applying_the_same_text_again_costs_nothing_and_a_new_one_costs_an_irep() {
    let mut app = app();
    let handle = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile("loop { Rubevy.ask('alive').pop }"));
    let entity = app.world_mut().spawn(Script::new(handle)).id();
    frames(&mut app, 4);
    let after_first = ireps(&app);

    // ten applies of the same text, each through the asset a compile would have made
    for _ in 0..10 {
        let handle = app
            .world_mut()
            .resource_mut::<Assets<MrbAsset>>()
            .add(compile("loop { Rubevy.ask('alive').pop }"));
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, app.world());
        replace_script(&mut commands, entity, Script::new(handle));
        queue.apply(app.world_mut());
        frames(&mut app, 3);
    }
    assert_eq!(ireps(&app), after_first, "ten applies of the same text, no irep");
    assert_eq!(app.world().resource::<ScriptWorld>().loaded_programs(), 1);

    // ten applies of ten different texts
    let before_ten = ireps(&app);
    for i in 0..10 {
        let src = format!("x = {i}\nloop {{ Rubevy.ask('alive').pop }}\n");
        let handle = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(&src));
        let mut queue = bevy::ecs::world::CommandQueue::default();
        let mut commands = Commands::new(&mut queue, app.world());
        replace_script(&mut commands, entity, Script::new(handle));
        queue.apply(app.world_mut());
        frames(&mut app, 3);
    }
    let per_apply = (ireps(&app) - before_ten) as f64 / 10.0;
    assert!(per_apply > 0.0, "a new text is a new irep");
    assert_eq!(app.world().resource::<ScriptWorld>().loaded_programs(), 11);
    // this script is one irep per program (no blocks of its own beyond the one `loop` takes),
    // which is what the growth of an unbounded editor session is counted in
    assert_eq!(per_apply, 2.0, "one irep for the top level and one for the block");
}
