//! `Rubevy::Camera`, the first optional Ruby layer (`rubevy::layers::CAMERA`).
//!
//! `tests/camera.rs` is the ground this stands on: what a script can do to a camera with nothing
//! but `Rubevy.find`, `e[:Transform] =` and `e[:Projection] =`, and the three shapes that do not
//! work. This file is the layer over it — the same crate, the same dependencies, no Rust that
//! knows what a camera is — and what it checks is that the awkward parts are hidden: the variant
//! remembered rather than switched, `zoom` meaning the same thing on a 2D camera and a 3D one,
//! two zooms in one tick both counting, and a `follow` that stops when what it follows is gone.
//!
//! The layer is loaded by the **app**, in a `Startup` system, which is the whole of the entry
//! point. An app that does not load it has no `Rubevy::Camera` at all, and the first test is
//! that.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::{Camera2d, Camera3d, OrthographicProjection, PerspectiveProjection, Projection};
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, Request, RubevyPlugin, Script, ScriptWorld};

/// What the scripts asked the game, so a test can read what they saw.
#[derive(Resource, Default)]
struct Seen(Vec<Request>);

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// Every question that is not `camera.world_at` is answered nil, so a script can report by
/// asking. `camera.world_at` is left standing on purpose: the layer throws it and nobody in
/// rubevy answers it, which is what one of the tests is about.
fn answer_nil(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>) {
    for r in world.take_requests() {
        if r.kind != "camera.world_at" {
            world.answer(&r, Answer::Nil);
        }
        seen.0.push(r);
    }
}

/// The one line an app writes to take the layer up.
fn add_the_camera_layer(mut world: ResMut<ScriptWorld>) {
    world.load_and_run(rubevy::layers::CAMERA).expect("the layer runs");
}

fn app(with_layer: bool) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Seen>()
    .register_type::<Transform>()
    .register_type::<Camera2d>()
    .register_type::<Camera3d>()
    .register_type::<Projection>()
    .add_systems(Update, answer_nil);
    if with_layer {
        app.add_systems(Startup, add_the_camera_layer);
    }
    app
}

/// The same app with `Projection` left unregistered, which is what a type nobody registered
/// looks like from Ruby: the read answers nil and the write is refused.
fn app_without_projection() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Seen>()
    .register_type::<Transform>()
    .register_type::<Camera2d>()
    .add_systems(Startup, add_the_camera_layer)
    .add_systems(Update, answer_nil);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(5));
        app.update();
    }
}

fn run(app: &mut App, entity: Entity, src: &str) -> Entity {
    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
    app.world_mut().entity_mut(entity).insert(Script::new(script));
    entity
}

/// A watcher entity with a script on it, beside whatever the test spawned.
fn watch(app: &mut App, src: &str) -> Entity {
    let watcher = app.world_mut().spawn_empty().id();
    run(app, watcher, src)
}

fn spawn_camera_2d(app: &mut App) -> Entity {
    app.world_mut()
        .spawn((
            Camera2d,
            Transform::from_xyz(0.0, 0.0, 0.0),
            Projection::Orthographic(OrthographicProjection::default_2d()),
        ))
        .id()
}

fn spawn_camera_3d(app: &mut App) -> Entity {
    app.world_mut()
        .spawn((
            Camera3d::default(),
            Transform::from_xyz(0.0, 0.0, 10.0),
            Projection::Perspective(PerspectiveProjection::default()),
        ))
        .id()
}

fn ortho_scale(app: &App, camera: Entity) -> f32 {
    match app.world().get::<Projection>(camera).expect("still there") {
        Projection::Orthographic(o) => o.scale,
        other => panic!("not orthographic any more: {other:?}"),
    }
}

fn fov(app: &App, camera: Entity) -> f32 {
    match app.world().get::<Projection>(camera).expect("still there") {
        Projection::Perspective(p) => p.fov,
        other => panic!("not perspective any more: {other:?}"),
    }
}

fn at(app: &App, entity: Entity) -> Vec3 {
    app.world().get::<Transform>(entity).expect("still there").translation
}

/// An app that does not load the layer has no `Rubevy::Camera`. That is what makes it a layer
/// and not part of the host API: the crate carries the bytes, the app says whether its scripts
/// get them.
#[test]
fn without_the_layer_there_is_no_camera_class() {
    for with_layer in [false, true] {
        let mut app = app(with_layer);
        spawn_camera_2d(&mut app);
        watch(
            &mut app,
            r#"
              begin
                what = Rubevy::Camera.name
              rescue NameError
                what = "nothing"
              end
              Rubevy.ask("class", what).pop
            "#,
        );
        frames(&mut app, 20);

        let seen = app.world().resource::<Seen>();
        let r = seen.0.first().expect("the script asked");
        let expected = if with_layer { "Rubevy::Camera" } else { "nothing" };
        assert_eq!(r.text(0), Some(expected), "with_layer = {with_layer}");
    }
}

/// Finding the camera, and what the object says about itself. `find` answers the 2D one where
/// there are both, because that is the order `markers` names them in.
#[test]
fn the_layer_finds_the_camera_and_says_what_it_is() {
    let mut app = app(true);
    let two_d = spawn_camera_2d(&mut app);
    spawn_camera_3d(&mut app);
    watch(
        &mut app,
        r#"
          cam = Rubevy::Camera.find
          all = Rubevy::Camera.find_all
          Rubevy.ask("found",
                     all.length,
                     cam.kind.to_s,
                     cam.variant.to_s,
                     cam.scale,
                     cam.entity.to_i,
                     all[1].kind.to_s,
                     all[1].variant.to_s,
                     cam.position.inspect).pop
        "#,
    );
    frames(&mut app, 20);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.num(0), Some(2.0), "both cameras are found");
    assert_eq!(r.text(1), Some("Camera2d"), "and the 2D one comes first");
    assert_eq!(r.text(2), Some("Orthographic"), "the variant it was read in");
    assert_eq!(r.num(3), Some(1.0), "a camera just read is at 1x");
    assert_eq!(r.num(4), Some(two_d.to_bits() as f64));
    assert_eq!(r.text(5), Some("Camera3d"));
    assert_eq!(r.text(6), Some("Perspective"));
    assert_eq!(r.text(7), Some("[0.0, 0.0, 0.0]"), "position is the translation");
}

/// `move_to` and `pan`, and the 2D rule the layer keeps: z is the drawing order, so nothing
/// writes it unless asked. Two `pan`s in one tick both count — the layer answers what it wrote
/// this frame rather than the world's value, which is a frame behind.
#[test]
fn moving_the_camera_keeps_z_and_two_pans_in_one_tick_both_count() {
    let mut app = app(true);
    let camera = app
        .world_mut()
        .spawn((
            Camera2d,
            Transform::from_xyz(0.0, 0.0, 999.0),
            Projection::Orthographic(OrthographicProjection::default_2d()),
        ))
        .id();
    watch(
        &mut app,
        r#"
          cam = Rubevy::Camera.find
          cam.move_to(10.0, 20.0)
          cam.pan(1.0, 2.0)
          cam.pan(1.0, 2.0)
          Rubevy.ask("moved", cam.position.inspect).pop
        "#,
    );
    frames(&mut app, 20);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.text(0), Some("[12.0, 24.0, 999.0]"), "the layer's own idea of where it is");
    assert_eq!(at(&app, camera), Vec3::new(12.0, 24.0, 999.0), "and the world agrees");
}

/// `zoom 2` comes twice as close on a 2D camera: the orthographic `scale` is halved, and every
/// other field of the projection is untouched (it is a partial write into the current variant).
/// Two zooms in one tick multiply, which is the thing the layer keeps the magnification for.
#[test]
fn zooming_a_2d_camera_halves_its_scale_and_two_zooms_in_one_tick_multiply() {
    let mut app = app(true);
    let camera = spawn_camera_2d(&mut app);
    watch(
        &mut app,
        r#"
          cam = Rubevy::Camera.find
          cam.zoom 2
          cam.zoom 2
          Rubevy.ask("zoomed", cam.scale).pop
        "#,
    );
    frames(&mut app, 20);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.num(0), Some(4.0), "the magnification is the product");
    assert_eq!(ortho_scale(&app, camera), 0.25, "and the scale is its reciprocal");
    match app.world().get::<Projection>(camera).expect("still there") {
        Projection::Orthographic(o) => {
            assert_eq!(o.far, 1000.0, "the fields the layer did not name are untouched");
            assert_eq!(o.near, -1000.0);
        }
        other => panic!("{other:?}"),
    }
}

/// The same word on a 3D camera, where the number is an angle: `zoom 2` is not `fov / 2` but the
/// angle whose half-tangent is halved, because it is `tan(fov / 2)` that sets how big things
/// look. `zoom 2` then `zoom 0.5` comes back exactly where it started.
#[test]
fn zooming_a_3d_camera_halves_the_tangent_of_half_its_fov() {
    let mut app = app(true);
    let camera = spawn_camera_3d(&mut app);
    let before = fov(&app, camera);
    watch(&mut app, r#"Rubevy::Camera.find_all[0].zoom 2; Rubevy.ask("zoomed").pop"#);
    frames(&mut app, 20);
    let after = fov(&app, camera);

    let want = 2.0 * ((before / 2.0).tan() / 2.0).atan();
    assert!((after - want).abs() < 1e-6, "fov {after} is not {want}");
    assert!(after < before, "a closer camera sees a narrower angle");
    assert!(after > before / 2.0, "and the angle is not simply halved: {after} vs {}", before / 2.0);

    // and back again
    let mut app = self::app(true);
    let camera = spawn_camera_3d(&mut app);
    watch(
        &mut app,
        r#"
          cam = Rubevy::Camera.find_all[0]
          cam.zoom 2
          cam.zoom 0.5
          Rubevy.ask("back", cam.scale).pop
        "#,
    );
    frames(&mut app, 20);
    assert_eq!(app.world().resource::<Seen>().0[0].num(0), Some(1.0), "back at 1x");
    assert!((fov(&app, camera) - before).abs() < 1e-6, "and back at the angle it started at");
}

/// A camera whose `Projection` this layer cannot read — here because nobody registered the type,
/// which is what an unregistered type looks like from Ruby: nil — still pans, and says so about
/// zooming rather than writing something made up.
#[test]
fn a_camera_without_a_projection_pans_but_does_not_zoom() {
    let mut app = app_without_projection();
    let camera = spawn_camera_2d(&mut app);
    watch(
        &mut app,
        r#"
          cam = Rubevy::Camera.find
          Rubevy.ask("no projection",
                     cam.variant.inspect,
                     cam.scale.inspect,
                     cam.zoom(2).inspect,
                     cam.move_to(5.0, 6.0).class.to_s).pop
        "#,
    );
    frames(&mut app, 20);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.text(0), Some("nil"), "no variant was read");
    assert_eq!(r.text(1), Some("nil"), "so there is no magnification to answer");
    assert_eq!(r.text(2), Some("nil"), "and zoom says so instead of writing");
    assert_eq!(r.text(3), Some("Rubevy::Camera"), "but moving it still works");
    assert_eq!(at(&app, camera), Vec3::new(5.0, 6.0, 0.0));
}

/// `follow` is a task of its own that keeps the offset the camera had. It stops on `unfollow`,
/// and the camera stays where the following left it.
#[test]
fn following_an_entity_moves_the_camera_until_it_is_told_to_stop() {
    let mut app = app(true);
    let camera = spawn_camera_2d(&mut app);
    let target = app.world_mut().spawn(Transform::from_xyz(100.0, 0.0, 0.0)).id();
    // the script waits for the game to say when to stop, rather than for a length of time: a
    // test that slept would be a test of how busy the machine is
    watch(
        &mut app,
        r#"
          cam = Rubevy::Camera.find
          who = Rubevy.find(:Transform).find { |e| e[:Transform][:translation][0] > 50.0 }
          stop = Rubevy.subscribe(:stop)
          cam.follow who
          Rubevy.ask("following", cam.following?.to_s).pop
          stop.pop
          cam.unfollow
          Rubevy.ask("stopped", cam.following?.to_s).pop
        "#,
    );

    frames(&mut app, 10);
    assert!(
        (at(&app, camera).x - 100.0).abs() < 0.001,
        "the camera came to the target: {:?}",
        at(&app, camera)
    );

    // the target moves on, and the camera with it
    app.world_mut().entity_mut(target).insert(Transform::from_xyz(300.0, 40.0, 0.0));
    frames(&mut app, 10);
    assert!((at(&app, camera).x - 300.0).abs() < 0.001, "it followed: {:?}", at(&app, camera));
    assert_eq!(at(&app, camera).z, 0.0, "and never wrote z");

    // the game says stop, and then the target moves without the camera
    app.world_mut().resource_mut::<ScriptWorld>().publish(None, "stop", Answer::Bool(true));
    frames(&mut app, 20);
    let kinds: Vec<&str> = app.world().resource::<Seen>().0.iter().map(|r| &r.kind[..]).collect();
    assert!(kinds.contains(&"stopped"), "the script got as far as unfollow: {kinds:?}");
    app.world_mut().entity_mut(target).insert(Transform::from_xyz(900.0, 0.0, 0.0));
    frames(&mut app, 20);
    assert!((at(&app, camera).x - 300.0).abs() < 0.001, "it stayed: {:?}", at(&app, camera));

    let seen = app.world().resource::<Seen>();
    assert_eq!(seen.0[0].text(0), Some("true"), "following? while it follows");
    assert_eq!(seen.0[1].text(0), Some("false"), "and not after unfollow");
}

/// A camera that follows something that is despawned stops by itself: the target's `Transform`
/// reads nil, which is what a despawned entity answers, and the task ends there.
#[test]
fn following_stops_when_the_target_is_gone() {
    let mut app = app(true);
    let camera = spawn_camera_2d(&mut app);
    let target = app.world_mut().spawn(Transform::from_xyz(100.0, 0.0, 0.0)).id();
    watch(
        &mut app,
        r#"
          cam = Rubevy::Camera.find
          who = Rubevy.find(:Transform).find { |e| e[:Transform][:translation][0] > 50.0 }
          cam.follow who
          sleep 0.2
          Rubevy.ask("done", cam.position.inspect).pop
        "#,
    );

    frames(&mut app, 10);
    let went = at(&app, camera);
    assert!((went.x - 100.0).abs() < 0.001, "it followed first: {went:?}");

    app.world_mut().entity_mut(target).despawn();
    frames(&mut app, 60);
    assert_eq!(at(&app, camera), went, "the camera stayed where the target left it");

    // the script ran on: the following task ended rather than raising or parking for ever
    let seen = app.world().resource::<Seen>();
    assert_eq!(seen.0.len(), 1, "the script reached its last line");
    assert_eq!(seen.0[0].kind, "done");
}

/// `world_at` is a question the game answers, and the layer throws it without waiting: what it
/// answers is the queue. Nobody in rubevy answers `camera.world_at` — a `pop` on it would park
/// that task for ever — so the layer hands the queue back and the script decides.
#[test]
fn world_at_hands_back_a_queue_that_nobody_answers() {
    let mut app = app(true);
    let camera = spawn_camera_2d(&mut app);
    watch(
        &mut app,
        r#"
          cam = Rubevy::Camera.find
          q = cam.world_at(120.0, 40.0)
          Rubevy.ask("asked", q.class.to_s).pop
        "#,
    );
    frames(&mut app, 20);

    let seen = app.world().resource::<Seen>();
    let asked: Vec<&Request> = seen.0.iter().filter(|r| r.kind == "camera.world_at").collect();
    assert_eq!(asked.len(), 1, "the question reached the game");
    assert_eq!(asked[0].entity_arg(0), Some(camera), "with the camera in it, since a game has several");
    assert_eq!(asked[0].num(1), Some(120.0));
    assert_eq!(asked[0].num(2), Some(40.0));

    let reported = seen.0.iter().find(|r| r.kind == "asked").expect("the script carried on");
    assert_eq!(reported.text(0), Some("Task::Queue"), "and it was not waited on");
}

/// The layer's own view of what the world refused (`Rubevy.rejected_writes`, filtered to this
/// camera). Here the camera's `Projection` is not registered, so every zoom is refused — and the
/// script can find that out, which before this was only in the host's log.
#[test]
fn a_camera_whose_writes_are_refused_can_say_so() {
    let mut app = app_without_projection();
    spawn_camera_2d(&mut app);
    watch(
        &mut app,
        r#"
          cam = Rubevy::Camera.find
          cam.move_to(1.0, 2.0)                  # this one lands
          cam.entity[:Projection] = { Orthographic: [ { scale: 4.0 } ] }
          sleep 0
          bad = cam.rejected_writes
          Rubevy.ask("refused", bad.length, bad[0][:name], bad[0][:reason], cam.variant.inspect).pop
        "#,
    );
    frames(&mut app, 20);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.num(0), Some(1.0), "one refusal, and the move beside it is not in it");
    assert_eq!(r.text(1), Some("Projection"));
    assert_eq!(r.text(2), Some("no registered component named Projection"));
    assert_eq!(r.text(3), Some("nil"), "and the layer read no variant either");
}

/// `reload` is how a script is told to look again, for the one thing the layer cannot see: the
/// host changing the projection from Rust. The magnification goes back to 1 and the new value
/// becomes what `zoom` is measured from.
#[test]
fn reload_takes_the_projection_as_it_now_stands() {
    let mut app = app(true);
    let camera = spawn_camera_2d(&mut app);
    watch(
        &mut app,
        r#"
          cam = Rubevy::Camera.find
          cam.zoom 2
          Rubevy.ask("first", cam.scale).pop
          sleep 0.2                              # the host changes the projection meanwhile
          cam.reload
          Rubevy.ask("reloaded", cam.scale).pop
          cam.zoom 2
          Rubevy.ask("again", cam.scale).pop
        "#,
    );
    frames(&mut app, 10);
    // the game sets the projection itself, as a level or a settings menu would
    if let Some(mut projection) = app.world_mut().get_mut::<Projection>(camera) {
        *projection = Projection::Orthographic(OrthographicProjection {
            scale: 8.0,
            ..OrthographicProjection::default_2d()
        });
    }
    frames(&mut app, 70);

    let seen = app.world().resource::<Seen>();
    let said: Vec<(String, Option<f64>)> =
        seen.0.iter().map(|r| (r.kind.clone(), r.num(0))).collect();
    assert_eq!(said[0], ("first".into(), Some(2.0)));
    assert_eq!(said[1], ("reloaded".into(), Some(1.0)), "a reload is 1x again");
    assert_eq!(said[2], ("again".into(), Some(2.0)));
    assert_eq!(ortho_scale(&app, camera), 4.0, "measured from the 8.0 the host put there");
}
