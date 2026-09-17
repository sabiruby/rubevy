//! The game's own answers, given **inside the tick** (`ScriptWorld::answer_in_tick`).
//!
//! A component read has cost no frame since the tick became a loop — run the tasks, answer what
//! they parked on, run them again. What is registered here joins that loop: a kind of
//! `Rubevy.ask` the game answers with a closure, called between two runs of the VM with the
//! world as it stands in `RubevySet::Tick`. So the spatial questions a world script asks every
//! frame for every creature ("the nearest plant") come back in the line that asked for them,
//! while a question answered by a system still costs its one frame — and both are in this file,
//! because the difference is the point.
//!
//! The measurements are the script's own: it reads `$rubevy[:frame]` before and after the
//! `Rubevy.ask(...).pop` and sends the difference back with a question nobody waits for.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use rubevy::{Answer, Arg, MrbAsset, Request, RubevyPlugin, RubevySet, Script, ScriptWorld};

/// The second VM's name tag, for the test that a tagged VM has this too.
struct Mods;

#[derive(Component)]
struct Hp(f32);

/// What the scripts reported, in the order they reported it: the questions the *game's* system
/// took (so a kind that is answered in the tick must never show up here).
#[derive(Resource, Default)]
struct Seen(Vec<Request>);

/// What the release sweep had to let go of, by the frame it happened on.
#[derive(Resource, Default)]
struct Released(Vec<(u32, usize)>);

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

/// The game's side of `Rubevy.ask`, in the set it belongs in: it answers everything it is given
/// with a number, and keeps the requests for the assertions.
fn answer_requests(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>) {
    for request in world.take_requests() {
        world.answer(&request, Answer::Num(7.0));
        seen.0.push(request);
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
    .add_systems(Update, answer_requests.in_set(RubevySet::Answer));
    app
}

fn run(app: &mut App, src: &str) -> Entity {
    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
    app.world_mut().spawn(Script::new(script)).id()
}

/// Everything the script says about `kind`, in the numbers a `gap` question carries: how many
/// frames the round trip took, and what came back.
fn asking(kind: &str) -> String {
    format!(
        r#"
          6.times do
            f0 = $rubevy[:frame]
            v = Rubevy.ask("{kind}").pop
            Rubevy.ask("gap", ($rubevy[:frame] - f0).to_f, v)
          end
        "#
    )
}

fn gaps(app: &App) -> Vec<i64> {
    app.world()
        .resource::<Seen>()
        .0
        .iter()
        .filter(|r| r.kind == "gap")
        .map(|r| r.num_or(0, -1.0) as i64)
        .collect()
}

fn answers(app: &App) -> Vec<f64> {
    app.world()
        .resource::<Seen>()
        .0
        .iter()
        .filter(|r| r.kind == "gap")
        .map(|r| r.num_or(1, -1.0))
        .collect()
}

// ---------------------------------------------------------------------------------------------

/// The one this stage is about: a kind the game registered is answered while the scripts are
/// still running, so the script waits no frame at all.
///
/// The same kind is registered twice on purpose. The later closure is the one that answers (and
/// the registration warns), which is what a game reloading its answers in the middle of a level
/// needs — the number `2.0` in every round is the second closure's.
#[test]
fn a_kind_the_game_answers_in_the_tick_costs_no_frame() {
    let mut app = app();
    {
        let mut scripts = app.world_mut().resource_mut::<ScriptWorld>();
        scripts.answer_in_tick("nearest", Box::new(|_world, _request| Answer::Num(1.0)));
        // registered a second time: the later one wins, and rubevy warns
        scripts.answer_in_tick("nearest", Box::new(|_world, _request| Answer::Num(2.0)));
    }
    run(&mut app, &asking("nearest"));
    frames(&mut app, 16);

    assert_eq!(gaps(&app), vec![0, 0, 0, 0, 0, 0], "answered inside the tick that asked");
    assert_eq!(answers(&app), vec![2.0; 6], "the closure registered last is the one that answers");
}

/// A kind nobody registered takes the road it always took: a `Request` for the game's system in
/// `RubevySet::Answer`, and the script wakes on the next frame. Registering one kind changes
/// nothing about the others.
#[test]
fn a_kind_nobody_registered_still_costs_one_frame() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<ScriptWorld>()
        .answer_in_tick("nearest", Box::new(|_world, _request| Answer::Num(1.0)));
    run(&mut app, &asking("elsewhere"));
    frames(&mut app, 16);

    assert_eq!(gaps(&app), vec![1, 1, 1, 1, 1, 1], "a question the game's system answers");
    assert_eq!(answers(&app), vec![7.0; 6], "and it is that system's answer, not a closure's");
}

/// A registered kind is sorted out where the question is made, exactly as rubevy's own four
/// are: a game's answering system never sees it, so it cannot answer one twice or trip over a
/// kind it does not know.
#[test]
fn a_registered_kind_never_reaches_take_requests() {
    let mut app = app();
    app.world_mut()
        .resource_mut::<ScriptWorld>()
        .answer_in_tick("nearest", Box::new(|_world, _request| Answer::Num(1.0)));
    run(
        &mut app,
        r#"
          Rubevy.ask("nearest").pop
          Rubevy.entity.has?(:Transform)
          Rubevy.ask("mine").pop
        "#,
    );
    frames(&mut app, 16);

    let kinds: Vec<&str> =
        app.world().resource::<Seen>().0.iter().map(|r| r.kind.as_str()).collect();
    assert_eq!(kinds, vec!["mine"], "take_requests hands over only what nobody answers in the tick");
}

/// What the closure sees is the world as it stands in `RubevySet::Tick` — this frame's, not the
/// one before it. The proof is a system in `.before(RubevySet::Tick)` that writes the frame
/// number into a component: the closure reads it back, and the number the script gets is the
/// number of the frame it asked in.
#[test]
fn the_closure_sees_the_world_this_frame_left_it_in() {
    fn stamp_the_frame(mut hp: Query<&mut Hp>, frame: Res<FrameCount>) {
        for mut hp in &mut hp {
            hp.0 = frame.0 as f32;
        }
    }

    let mut app = app();
    app.add_systems(Update, stamp_the_frame.before(RubevySet::Tick));
    app.world_mut().resource_mut::<ScriptWorld>().answer_in_tick(
        "hp",
        Box::new(|world: &World, request: &Request| {
            match request.entity_arg(0).and_then(|e| world.get::<Hp>(e)) {
                Some(hp) => Answer::Num(hp.0 as f64),
                None => Answer::Nil,
            }
        }),
    );
    let entity = run(
        &mut app,
        r#"
          e = Rubevy.entity
          3.times do
            v = Rubevy.ask("hp", e).pop
            Rubevy.ask("saw", v, $rubevy[:frame].to_f)
            sleep 0.05
          end
        "#,
    );
    app.world_mut().entity_mut(entity).insert(Hp(-1.0));
    // the `sleep 0.05` is real time, and a frame here is the 2 ms of `frames` plus what it costs
    frames(&mut app, 40);

    let seen = app.world().resource::<Seen>();
    let saw: Vec<(f64, f64)> =
        seen.0.iter().filter(|r| r.kind == "saw").map(|r| (r.num_or(0, -1.0), r.num_or(1, -2.0))).collect();
    assert_eq!(saw.len(), 3, "three rounds");
    for (value, frame) in saw {
        assert_eq!(value, frame, "the closure read what this frame's system had just written");
    }
}

/// A second VM under a name tag has its own answerers, registered on its own `ScriptWorld<Mods>`
/// — the kinds of one VM are as separate as everything else about the two.
#[test]
fn a_named_vm_registers_its_own() {
    fn answer_mods(mut world: ResMut<ScriptWorld<Mods>>, mut seen: ResMut<Seen>) {
        for request in world.take_requests() {
            world.answer(&request, Answer::Num(7.0));
            seen.0.push(request);
        }
    }

    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::<Mods>::for_vm("assets"),
    ))
    .init_resource::<Seen>()
    .add_systems(Update, answer_mods.in_set(RubevySet::<Mods>::answer()));
    app.world_mut()
        .resource_mut::<ScriptWorld<Mods>>()
        .answer_in_tick("nearest", Box::new(|_world, _request| Answer::Num(3.0)));

    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(&asking("nearest")));
    app.world_mut().spawn(Script::<Mods>::for_vm(script));
    frames(&mut app, 16);

    assert_eq!(gaps(&app), vec![0, 0, 0, 0, 0, 0], "the mods' VM answers in its own tick");
    assert_eq!(answers(&app), vec![3.0; 6], "with the mods' VM's own closure");
}

/// An argument that is a Hash or an Array reaches the closure as the `Arg::Value` it is, and the
/// value it names is let go of the way every other request's is: the request is dropped inside
/// the tick, and the release sweep at the head of the **next** frame's `Deliver` unregisters it.
///
/// Reading *into* such a value needs the `Vm`, which the closure is not handed — so what this
/// checks is what a closure really can do with one: see that it is there, and that it is not a
/// number or a string. A game that has to look inside answers in a system instead.
#[test]
fn the_closure_sees_an_arg_value_and_it_is_released_on_the_next_frame() {
    /// The release sweep, one step ahead of the plugin's own (which is inside `Deliver`): what
    /// this takes, that one would have.
    fn sweep(mut world: ResMut<ScriptWorld>, mut released: ResMut<Released>, frame: Res<FrameCount>) {
        let n = world.release_dropped_values();
        if n > 0 {
            released.0.push((frame.0, n));
        }
    }

    let mut app = app();
    app.init_resource::<Released>().add_systems(Update, sweep.before(RubevySet::Deliver));
    app.world_mut().resource_mut::<ScriptWorld>().answer_in_tick(
        "carry",
        Box::new(|_world: &World, request: &Request| {
            let is_value = matches!(request.args.first(), Some(Arg::Value(_)));
            let readable = request.value(0).is_some();
            let not_a_number = request.num(0).is_none() && request.text(0).is_none();
            Answer::Num(if is_value && readable && not_a_number { 1.0 } else { 0.0 })
        }),
    );
    run(
        &mut app,
        r#"
          f0 = $rubevy[:frame]
          v = Rubevy.ask("carry", { r: 2.0, tags: ["a"] }).pop
          Rubevy.ask("carried", v, ($rubevy[:frame] - f0).to_f, $rubevy[:frame].to_f)
        "#,
    );
    frames(&mut app, 16);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.iter().find(|r| r.kind == "carried").expect("the script reported");
    assert_eq!(r.num(0), Some(1.0), "the closure was handed the Hash as a value");
    assert_eq!(r.num(1), Some(0.0), "and answered inside the tick");
    let asked_on = r.num_or(2, -1.0) as u32;

    let released = &app.world().resource::<Released>().0;
    assert_eq!(released.len(), 1, "one value, let go of once: {released:?}");
    assert_eq!(released[0].1, 1, "the Hash");
    assert_eq!(
        released[0].0,
        asked_on + 1,
        "the value outlived its request by a frame, as every other request's does"
    );
}
