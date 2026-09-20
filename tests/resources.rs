//! Resources by name: `Rubevy.resource(:Score)` reads one through Bevy's reflection and
//! `Rubevy.set_resource(:Score, hash)` writes it back with the frame's other writes.
//!
//! It is the component road with the entity taken out of it — the same registry lookup, the same
//! `RubyData`, the same "only the fields you name", the same answer inside the tick — so what
//! these tests are really about is the two places where a resource is *not* a component: how the
//! world is asked where it keeps one (Bevy 0.19 puts it on an entity of its own), and the
//! `#[reflect(Resource)]` that a type has to carry for a script to reach it this way.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, Request, RubevyPlugin, RubevySet, Script, ScriptWorld};

/// The requests the game itself took, which must never be one of rubevy's own kinds.
#[derive(Resource, Default)]
struct Seen(Vec<Request>);

#[derive(Resource, Reflect, Debug, Default, PartialEq)]
#[reflect(Resource)]
struct Score {
    points: f32,
    best: f32,
}

/// A resource with something under its top level, so that the walk down is the component's walk
/// down: a `Vec3` is an Array of three, a `bool` is `true`/`false`.
#[derive(Resource, Reflect, Debug, Default, PartialEq)]
#[reflect(Resource)]
struct Weather {
    wind: Vec3,
    raining: bool,
}

/// Registered and reflected as a resource, but never put into the world.
#[derive(Resource, Reflect, Debug, Default)]
#[reflect(Resource)]
struct Ghost {
    seen: f32,
}

/// In the world, but registered nowhere.
#[derive(Resource, Debug, Default)]
struct Secret(#[allow(dead_code)] f32);

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Seen>()
    // `MinimalPlugins` brings no automatic registration, so the test registers what it uses
    .register_type::<Transform>()
    .register_type::<Score>()
    .register_type::<Weather>()
    .register_type::<Ghost>()
    .add_systems(Update, answer_nil);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

fn answer_nil(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>) {
    for r in world.take_requests() {
        world.answer(&r, Answer::Nil);
        seen.0.push(r);
    }
}

/// A script on an entity of its own: nothing here is about the entity, but a script is still a
/// task on one.
fn run(app: &mut App, src: &str) -> Entity {
    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
    app.world_mut().spawn(Script::new(script)).id()
}

#[test]
fn a_script_reads_a_resource_as_a_hash() {
    let mut app = app();
    app.insert_resource(Score { points: 7.0, best: 12.0 });
    app.insert_resource(Weather { wind: Vec3::new(1.0, 2.0, 3.0), raining: true });
    run(
        &mut app,
        r#"
          s = Rubevy.resource(:Score)
          w = Rubevy.resource(:Weather)
          Rubevy.ask("read",
                     s.class.to_s,
                     s[:points], s[:best],
                     w[:wind].length, w[:wind][1],
                     w[:raining].to_s).pop
        "#,
    );
    frames(&mut app, 10);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.text(0), Some("Hash"), "a resource is a Hash of its fields, as a component is");
    assert_eq!((r.num(1), r.num(2)), (Some(7.0), Some(12.0)));
    assert_eq!(r.num(3), Some(3.0), "a Vec3 is three numbers here too");
    assert_eq!(r.num(4), Some(2.0));
    assert_eq!(r.text(5), Some("true"));
}

#[test]
fn a_script_writes_one_field_and_leaves_the_others() {
    let mut app = app();
    app.insert_resource(Score { points: 7.0, best: 12.0 });
    run(
        &mut app,
        r#"
          s = Rubevy.resource(:Score)
          Rubevy.set_resource(:Score, { points: s[:points] + 1.0 })   # `best` is not named
          Rubevy.ask("written").pop
        "#,
    );
    frames(&mut app, 10);

    assert_eq!(app.world().resource::<Seen>().0.len(), 1, "the script got that far");
    assert_eq!(app.world().resource::<Score>(), &Score { points: 8.0, best: 12.0 });
}

/// The same seam the component write has, for the same reason: `apply_resource_writes` is at the
/// end of the frame, so a read after a write in one tick answers the value the tick began with.
#[test]
fn a_read_after_a_write_in_the_same_tick_is_still_the_old_value() {
    let mut app = app();
    app.insert_resource(Score { points: 7.0, best: 12.0 });
    run(
        &mut app,
        r#"
          f0 = $rubevy[:frame]
          before = Rubevy.resource(:Score)[:points]
          Rubevy.set_resource(:Score, { points: 99.0 })
          again = Rubevy.resource(:Score)[:points]     # the write has not landed yet
          f1 = $rubevy[:frame]
          sleep 0.05                                   # a frame goes by, and with it the write
          after = Rubevy.resource(:Score)[:points]
          Rubevy.ask("score", before, again, after, (f1 - f0).to_f).pop
        "#,
    );
    frames(&mut app, 40);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.num(0), Some(7.0), "what the resource says when the tick begins");
    assert_eq!(r.num(1), Some(7.0), "and still says after a write in the same tick");
    assert_eq!(r.num(2), Some(99.0), "the write is there in a later frame");
    assert_eq!(r.num(3), Some(0.0), "both reads and the write were one and the same tick");
}

/// Four ways of not being there, all of them nil rather than an error — the rule the component
/// read already keeps.
#[test]
fn what_is_not_a_reachable_resource_reads_as_nil() {
    let mut app = app();
    app.insert_resource(Secret(1.0));
    // a component type that is registered and really is on an entity: still not a resource
    app.world_mut().spawn(Transform::from_xyz(1.0, 2.0, 3.0));
    run(
        &mut app,
        r#"
          Rubevy.ask("missing",
                     Rubevy.resource(:Ghost).inspect,      # registered, not in the world
                     Rubevy.resource(:Secret).inspect,     # in the world, not registered
                     Rubevy.resource(:Nonesuch).inspect,   # no such type at all
                     Rubevy.resource(:Transform).inspect). # registered, but a component
            pop
        "#,
    );
    frames(&mut app, 10);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.text(0), Some("nil"), "a resource nothing inserted");
    assert_eq!(r.text(1), Some("nil"), "a resource nobody registered");
    assert_eq!(r.text(2), Some("nil"), "a name of no type");
    assert_eq!(r.text(3), Some("nil"), "a component is not reachable as a resource");
}

/// A write to something that is not a reachable resource is logged and skipped, and — this is
/// the part worth a test — the frame's other writes still happen.
#[test]
fn a_write_to_what_is_not_a_resource_does_not_stop_the_others() {
    let mut app = app();
    app.insert_resource(Score { points: 7.0, best: 12.0 });
    run(
        &mut app,
        r#"
          Rubevy.set_resource(:Nonesuch, { points: 1.0 })
          Rubevy.set_resource(:Ghost, { seen: 1.0 })
          Rubevy.set_resource(:Score, { points: 5.0 })
          Rubevy.ask("written").pop
        "#,
    );
    frames(&mut app, 10);

    assert_eq!(app.world().resource::<Score>(), &Score { points: 5.0, best: 12.0 });
}

/// A generic type's name carries its parameters, because that is what bevy_reflect's short type
/// path is (`Option<Vec<usize>>` is `"Option<Vec<usize>>"`). Nothing in rubevy spells it: the
/// lookup is the component's lookup.
///
/// `Time<Virtual>` also settles which of the two registrations is at work here. This crate takes
/// bevy with `default-features = false`, so the `reflect_auto_register` feature — bevy's default
/// way of registering every `Reflect` type in the build — is off; `TimePlugin`, which
/// `MinimalPlugins` adds, still calls `register_type` for the four `Time`s by hand
/// (`bevy_time-0.19.1/src/lib.rs:76-79`). So this test reads a resource *nobody in this file
/// registered*, and would not if the plugin had stopped doing it.
#[test]
fn a_generic_resource_is_named_with_its_parameters() {
    let mut app = app();
    run(
        &mut app,
        r#"
          t = Rubevy.resource("Time<Virtual>")
          Rubevy.ask("time",
                     t.class.to_s,
                     t.key?(:delta_secs).to_s,
                     Rubevy.resource("Time<()>").class.to_s,    # `Time` is `Time<()>`
                     Rubevy.resource(:Time).inspect,            # and is not reachable as `Time`
                     Rubevy.resource("Time<Nonesuch>").inspect).pop
        "#,
    );
    frames(&mut app, 10);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.text(0), Some("Hash"), "`Time<Virtual>` is reachable under that very name");
    assert_eq!(r.text(1), Some("true"));
    assert_eq!(r.text(2), Some("Hash"), "a defaulted parameter is written out, not left off");
    assert_eq!(r.text(3), Some("nil"), "so the bare name finds nothing");
    assert_eq!(r.text(4), Some("nil"), "a parameter of no type is a name of no type");
}

#[test]
fn a_resource_question_never_reaches_the_game() {
    let mut app = app();
    app.insert_resource(Score::default());
    run(
        &mut app,
        r#"
          Rubevy.resource(:Score)
          Rubevy.resource(:Ghost)
          Rubevy.ask("mine").pop
        "#,
    );
    frames(&mut app, 15);

    let kinds: Vec<&str> =
        app.world().resource::<Seen>().0.iter().map(|r| r.kind.as_str()).collect();
    assert_eq!(kinds, vec!["mine"], "take_requests hands over only the game's own kinds");
}

// ------------------------------------------------------------------ a second VM

/// The name tag of a second VM, as `tests/two_vms.rs` writes one.
struct Mods;

/// Two VMs, one world. A resource belongs to the app and not to a VM, so both scripts see the
/// same `Score` — and the point of the test is that the second VM *has* the road at all: the
/// cache, the requests and the writes are all per-`ScriptWorld<M>`, so a VM under a name tag
/// reads and writes resources on its own.
#[test]
fn a_second_vm_reads_and_writes_the_same_resource() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
        RubevyPlugin::<Mods>::for_vm("assets"),
    ))
    .init_resource::<Seen>()
    .register_type::<Score>()
    .insert_resource(Score { points: 1.0, best: 100.0 })
    .add_systems(Update, answer_nil.in_set(RubevySet::<()>::answer()))
    .add_systems(Update, answer_mods.in_set(RubevySet::<Mods>::answer()));

    let game = compile(
        r#"
          Rubevy.ask("game", Rubevy.resource(:Score)[:points]).pop
        "#,
    );
    let modded = compile(
        r#"
          s = Rubevy.resource(:Score)
          Rubevy.set_resource(:Score, { points: s[:points] + 10.0 })
          Rubevy.ask("mod", s[:best]).pop
        "#,
    );
    let (game, modded) = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        (assets.add(game), assets.add(modded))
    };
    app.world_mut().spawn(Script::new(game));
    app.world_mut().spawn(Script::<Mods>::for_vm(modded));
    frames(&mut app, 10);

    let seen = app.world().resource::<Seen>();
    let kinds: Vec<&str> = seen.0.iter().map(|r| r.kind.as_str()).collect();
    assert!(kinds.contains(&"game"), "the first VM's script ran: {kinds:?}");
    assert!(kinds.contains(&"mod"), "and so did the second VM's: {kinds:?}");
    let game = seen.0.iter().find(|r| r.kind == "game").expect("the game's script asked");
    let modded = seen.0.iter().find(|r| r.kind == "mod").expect("the mod's script asked");
    assert_eq!(game.num(0), Some(1.0), "both VMs read the world's one Score");
    assert_eq!(modded.num(0), Some(100.0));
    assert_eq!(
        app.world().resource::<Score>(),
        &Score { points: 11.0, best: 100.0 },
        "and the second VM's write landed"
    );
}

/// The second VM's questions, into the same list: what is being told apart here is the VMs, not
/// the questions.
fn answer_mods(mut world: ResMut<ScriptWorld<Mods>>, mut seen: ResMut<Seen>) {
    for r in world.take_requests() {
        world.answer(&r, Answer::Nil);
        seen.0.push(r);
    }
}
