//! A camera driven from Ruby, with nothing added to the crate for it.
//!
//! Bevy's camera is three ordinary reflected components — the marker (`Camera2d`, `Camera3d`),
//! the `Transform` that places it, and `Projection`, an enum whose current variant holds the
//! numbers that zoom it. So `Rubevy.find`, `e[:Transform] =` and `e[:Projection] =` should be
//! enough, and this file is the question of whether they are: the answers are in
//! `docs/worklog/2026-09-20-camera-from-ruby.md` and they decide the shape of the optional Ruby
//! camera layer.
//!
//! `bevy_camera` is a dev-dependency feature only. The crate itself does not know what a camera
//! is and must not start to.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::{Camera2d, Camera3d, OrthographicProjection, PerspectiveProjection, Projection};
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, Request, RubevyPlugin, Script, ScriptWorld};

/// What the script asked the game, so a test can read the values it saw.
#[derive(Resource, Default)]
struct Seen(Vec<Request>);

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// `MinimalPlugins` registers nothing, and this crate is not built with bevy's
/// `reflect_auto_register`, so every type a script may name is registered here — exactly as
/// `tests/components.rs` registers `Transform`. In a real game `DefaultPlugins` does it.
fn app() -> App {
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

fn run(app: &mut App, entity: Entity, src: &str) {
    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
    app.world_mut().entity_mut(entity).insert(Script::new(script));
}

/// The camera is not spawned with a `Camera` (and so with a render target) because none of this
/// renders: `Camera2d`'s required components would pull in a chain that wants a window. The
/// three components a script touches are put on by hand, which is also what says which three
/// they are.
fn spawn_camera_2d(app: &mut App) -> Entity {
    app.world_mut()
        .spawn((
            Camera2d,
            Transform::from_xyz(0.0, 0.0, 0.0),
            Projection::Orthographic(OrthographicProjection::default_2d()),
        ))
        .id()
}

fn projection_scale(app: &App, camera: Entity) -> f32 {
    match app.world().get::<Projection>(camera).expect("still there") {
        Projection::Orthographic(o) => o.scale,
        other => panic!("not orthographic any more: {other:?}"),
    }
}

/// `Camera2d` is a unit struct with `#[reflect(Component)]` (bevy_camera 0.19.1
/// `src/components.rs:9-16`), so it is findable by name and reads as an empty Hash — there is
/// nothing in it. Finding the camera and moving it is the whole of panning.
#[test]
fn a_script_finds_the_camera_by_name_and_moves_it() {
    let mut app = app();
    let camera = spawn_camera_2d(&mut app);
    let watcher = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        watcher,
        r#"
          found = Rubevy.find(:Camera2d)
          cam = found[0]
          tf = cam[:Transform]
          tf[:translation][0] += 120.0
          tf[:translation][1] -= 40.0
          cam[:Transform] = tf
          Rubevy.ask("panned",
                     found.length,
                     cam.class.to_s,
                     cam[:Camera2d].inspect,
                     cam.has?(:Projection).to_s).pop
        "#,
    );
    frames(&mut app, 10);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.num(0), Some(1.0), "one camera in the world");
    assert_eq!(r.text(1), Some("Rubevy::Entity"));
    assert_eq!(r.text(2), Some("{}"), "a unit-struct marker is an empty Hash, not nil");
    assert_eq!(r.text(3), Some("true"));
    let t = app.world().get::<Transform>(camera).expect("still there");
    assert_eq!(t.translation, Vec3::new(120.0, -40.0, 0.0), "the camera moved");
}

/// What a script sees when it reads `Projection`: a one-entry Hash naming the current variant,
/// whose value is a one-element Array (the variant is a tuple of one) holding the projection's
/// own Hash. `scale` is in there, and so is `scaling_mode`, itself a unit variant read as a
/// Symbol.
#[test]
fn reading_the_projection_names_the_variant_and_its_fields() {
    let mut app = app();
    let _camera = spawn_camera_2d(&mut app);
    let watcher = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        watcher,
        r#"
          p9 = Rubevy.find(:Camera2d)[0][:Projection]
          inner = p9[:Orthographic][0]
          Rubevy.ask("read",
                     p9.class.to_s,
                     p9.keys.length,
                     p9.keys[0].to_s,
                     p9.keys[0].class.to_s,
                     p9[:Orthographic].class.to_s,
                     p9[:Orthographic].length,
                     inner.class.to_s,
                     inner[:scale],
                     inner[:near],
                     inner[:far],
                     inner[:scaling_mode].to_s,
                     inner[:scaling_mode].class.to_s,
                     inner[:viewport_origin].inspect,
                     inner[:area].keys.map { |k| k.to_s }.sort.join(",")).pop
        "#,
    );
    frames(&mut app, 10);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.text(0), Some("Hash"), "a tuple variant is a one-entry Hash");
    assert_eq!(r.num(1), Some(1.0));
    assert_eq!(r.text(2), Some("Orthographic"), "the variant it is in right now");
    assert_eq!(r.text(3), Some("Symbol"), "named by a Symbol, as an enum always is");
    assert_eq!(r.text(4), Some("Array"), "the variant's fields, by position");
    assert_eq!(r.num(5), Some(1.0), "Projection::Orthographic holds exactly one");
    assert_eq!(r.text(6), Some("Hash"), "and that one is the OrthographicProjection struct");
    assert_eq!(r.num(7), Some(1.0), "the default scale");
    assert_eq!(r.num(8), Some(-1000.0), "default_2d's near, which is behind the camera");
    assert_eq!(r.num(9), Some(1000.0), "and its far");
    assert_eq!(r.text(10), Some("WindowSize"), "a unit variant nested two deep");
    assert_eq!(r.text(11), Some("Symbol"));
    assert_eq!(r.text(12), Some("[0.5, 0.5]"), "a Vec2 is an Array");
    assert_eq!(r.text(13), Some("max,min"), "Rect is a struct of two Vec2");
}

/// Zooming: the variant the value is already in may have its fields written, and that is the one
/// case `src/reflect.rs`'s `apply_enum` allows for a tuple variant. `[{scale: …}]` is the Array
/// of the variant's fields with a partial Hash for the one field inside — the other fields of
/// `OrthographicProjection` keep their values, exactly as a partial component write does.
#[test]
fn a_script_zooms_by_writing_scale_into_the_current_variant() {
    let mut app = app();
    let camera = spawn_camera_2d(&mut app);
    let watcher = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        watcher,
        r#"
          cam = Rubevy.find(:Camera2d)[0]
          cam[:Projection] = { Orthographic: [ { scale: 2.5 } ] }
          sleep 0.05                                   # the write lands at the end of the frame
          after = cam[:Projection][:Orthographic][0]
          Rubevy.ask("zoomed", after[:scale], after[:far], after[:scaling_mode].to_s).pop
        "#,
    );
    frames(&mut app, 40);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.num(0), Some(2.5), "the script reads back what it wrote");
    assert_eq!(r.num(1), Some(1000.0), "and the fields it did not name are untouched");
    assert_eq!(r.text(2), Some("WindowSize"));
    assert_eq!(projection_scale(&app, camera), 2.5, "the world agrees");
}

/// The Array is not optional. `apply_enum`'s other branch hands a Hash to `Enum::field_mut(name)`,
/// and a *tuple* variant's fields have no names in bevy_reflect — `"0"` is not one — so a Hash
/// inside the variant reaches nothing and the write is skipped with `no such field`. Only the
/// struct variants of an enum take that spelling. This is the one place where the shape a script
/// must write is not the shape it read back, so the Ruby layer above should hide it.
#[test]
fn a_tuple_variants_field_cannot_be_named() {
    let mut app = app();
    let camera = spawn_camera_2d(&mut app);
    let watcher = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        watcher,
        r#"
          Rubevy.find(:Camera2d)[0][:Projection] = { Orthographic: { "0" => { scale: 4.0 } } }
          Rubevy.ask("wrote").pop
        "#,
    );
    frames(&mut app, 10);

    assert_eq!(app.world().resource::<Seen>().0.len(), 1, "the script got that far");
    assert_eq!(projection_scale(&app, camera), 1.0, "nothing was written");
}

/// Switching variants is refused, and that is the documented rule for a tuple or struct variant
/// (`src/reflect.rs`, `apply_enum`): bevy would have to build the whole new variant out of the
/// Ruby value, and a Hash says nothing about types. The write is logged and skipped; what was
/// there stays there, and the rest of the script keeps running.
#[test]
fn switching_the_projection_to_another_variant_is_refused() {
    let mut app = app();
    let camera = spawn_camera_2d(&mut app);
    let watcher = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        watcher,
        r#"
          cam = Rubevy.find(:Camera2d)[0]
          cam[:Projection] = { Perspective: [ { fov: 1.0 } ] }   # not the current variant
          cam[:Projection] = :Perspective                        # nor is a Symbol enough
          cam[:Projection] = { Orthographic: [ { scale: 3.0 } ] } # this one still lands
          sleep 0.05
          now = cam[:Projection]
          Rubevy.ask("still", now.keys[0].to_s, now[:Orthographic][0][:scale]).pop
        "#,
    );
    frames(&mut app, 40);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.text(0), Some("Orthographic"), "the refused writes changed nothing");
    assert_eq!(r.num(1), Some(3.0));
    assert_eq!(projection_scale(&app, camera), 3.0);
}

/// 3D is the same three components and the same spelling; only the variant's name and the
/// numbers inside it differ. A perspective camera zooms by its field of view, where the
/// orthographic one zooms by `scale`, so the Ruby word "zoom" means two different fields and the
/// layer above has to know which camera it is on.
#[test]
fn a_perspective_camera_is_the_same_write_with_another_variant() {
    let mut app = app();
    let camera = app
        .world_mut()
        .spawn((
            Camera3d::default(),
            Transform::from_xyz(0.0, 0.0, 10.0),
            Projection::Perspective(PerspectiveProjection::default()),
        ))
        .id();
    let watcher = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        watcher,
        r#"
          cam = Rubevy.find(:Camera3d)[0]
          before = cam[:Projection]
          cam[:Projection] = { Perspective: [ { fov: 1.2 } ] }
          sleep 0.05
          after = cam[:Projection][:Perspective][0]
          Rubevy.ask("3d",
                     before.keys[0].to_s,
                     before[:Perspective][0].keys.map { |k| k.to_s }.sort.join(","),
                     after[:fov],
                     after[:near],
                     cam[:Camera3d][:depth_load_op].inspect).pop
        "#,
    );
    frames(&mut app, 40);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.text(0), Some("Perspective"), "the default Projection of a Camera3d");
    assert_eq!(
        r.text(1),
        Some("aspect_ratio,far,fov,near,near_clip_plane"),
        "what a perspective camera has instead of scale"
    );
    assert_eq!(r.num(2), Some(1.2f32 as f64), "the field of view is what zooms it");
    assert!(r.num_or(3, -1.0) > 0.0, "near kept its default");
    assert_eq!(
        r.text(4),
        Some("{Clear: [0.0]}"),
        "a tuple variant one level down reads the same way"
    );
    match app.world().get::<Projection>(camera).expect("still there") {
        Projection::Perspective(p) => assert_eq!(p.fov, 1.2),
        other => panic!("not perspective any more: {other:?}"),
    }
}
