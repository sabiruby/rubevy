//! How deep a value is followed across the boundary (`ScriptWorld::max_depth`).
//!
//! The limit was a `const` of 16 with no recorded reason (R10 of
//! `docs/plans/generalize-plan.md`, `docs/numbers.md`). The default is still 16 and nothing
//! about an ordinary component changed; what is new is that an app whose own components go
//! deeper can say so, and an app that wants a shallower boundary can say that too. These tests
//! are the setting doing something in both directions — the read and the write.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, Request, RubevyPlugin, Script, ScriptWorld};

#[derive(Resource, Default)]
struct Seen(Vec<Request>);

/// Four structs one inside the next, so that the number at the bottom stands four levels below
/// the component itself: `Deep` is level 0, `b` 1, `c` 2, `d` 3 and `n` 4.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
struct Deep {
    b: B,
}

#[derive(Reflect, Debug, Default)]
struct B {
    c: C,
}

#[derive(Reflect, Debug, Default)]
struct C {
    d: D,
}

#[derive(Reflect, Debug, Default)]
struct D {
    n: f32,
}

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn answer_nil(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>) {
    for r in world.take_requests() {
        world.answer(&r, Answer::Nil);
        seen.0.push(r);
    }
}

/// `depth` is what the app sets at `Startup`; `None` leaves the default alone.
fn app(depth: Option<usize>) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Seen>()
    .register_type::<Deep>()
    .register_type::<B>()
    .register_type::<C>()
    .register_type::<D>()
    .add_systems(Update, answer_nil);
    if let Some(depth) = depth {
        app.add_systems(Startup, move |mut world: ResMut<ScriptWorld>| {
            world.set_max_depth(depth);
        });
    }
    app
}

/// 5 ms a frame, so that the `sleep 0.05` in the writing scripts below is over well inside the
/// frames they are given: the VM's clock is real time, and a machine under load makes frames
/// longer but never shorter.
fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(5));
        app.update();
    }
}

fn run(app: &mut App, entity: Entity, src: &str) {
    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
    app.world_mut().entity_mut(entity).insert(Script::new(script));
}

const READS_THE_BOTTOM: &str = r#"
  v = Rubevy.entity[:Deep]
  bottom = v[:b] && v[:b][:c] && v[:b][:c][:d]
  Rubevy.ask("read", bottom.inspect).pop
"#;

/// The default is unchanged: four levels below the component is still read.
#[test]
fn the_default_reads_a_component_four_levels_down() {
    let mut app = app(None);
    let entity = app.world_mut().spawn(Deep { b: B { c: C { d: D { n: 7.0 } } } }).id();
    run(&mut app, entity, READS_THE_BOTTOM);
    frames(&mut app, 10);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.text(0), Some("{n: 7.0}"), "the whole component came across");
}

/// The same component, the same script, a shallower setting: the walk stops and what is past it
/// is nil rather than a Hash.
#[test]
fn a_shallower_setting_stops_the_read() {
    let mut app = app(Some(2));
    let entity = app.world_mut().spawn(Deep { b: B { c: C { d: D { n: 7.0 } } } }).id();
    run(&mut app, entity, READS_THE_BOTTOM);
    frames(&mut app, 10);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.text(0), Some("nil"), "level 3 is past a max_depth of 2");
    assert_eq!(
        app.world().resource::<ScriptWorld>().max_depth(),
        2,
        "and the setting reads back"
    );
}

const WRITES_THE_BOTTOM: &str = r#"
  Rubevy.entity[:Deep] = { b: { c: { d: { n: 9.0 } } } }
  sleep 0.05
  w = Rubevy.rejected_writes
  Rubevy.ask("wrote", w.length.to_f, w.empty? ? "" : w[0][:reason]).pop
"#;

/// A write is walked by the same number, and the default takes all four levels of it.
#[test]
fn the_default_writes_a_component_four_levels_down() {
    let mut app = app(None);
    let entity = app.world_mut().spawn(Deep::default()).id();
    run(&mut app, entity, WRITES_THE_BOTTOM);
    frames(&mut app, 40);
    assert_eq!(
        app.world().get::<Deep>(entity).expect("still there").b.c.d.n,
        9.0,
        "the default wrote all four levels"
    );
    assert_eq!(app.world().resource::<Seen>().0[0].num(0), Some(0.0), "and refused nothing");
}

/// A shallower setting refuses the part of a write that is too deep, and says so where a script
/// can read it (`Rubevy.rejected_writes`).
#[test]
fn a_shallower_setting_stops_a_write() {
    let mut app = app(Some(2));
    let entity = app.world_mut().spawn(Deep::default()).id();
    run(&mut app, entity, WRITES_THE_BOTTOM);
    frames(&mut app, 40);
    assert_eq!(
        app.world().get::<Deep>(entity).expect("still there").b.c.d.n,
        0.0,
        "the write stopped before the bottom"
    );
    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.num(0), Some(1.0), "and the script was told: {:?}", r.text(1));
    // The refusal names where the walk stopped. What it says there is a consequence of *which*
    // of the two walks the limit bit first: the Hash is read out of the VM before it is applied
    // to anything, and that read stops at the same depth — so the entry past the boundary
    // arrives as a nil key and a nil value, and it is the nil key the write reports. The path
    // is right, the sentence is about the nil.
    assert!(
        r.text(1).is_some_and(|t| t.starts_with("b.c: ")),
        "the path it stopped at: {:?}",
        r.text(1)
    );
}
