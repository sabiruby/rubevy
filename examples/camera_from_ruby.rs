//! Panning and zooming a camera from Ruby, with nothing in rubevy that knows what a camera is.
//!
//! Bevy's camera is three ordinary reflected components: the marker (`Camera2d`), the `Transform`
//! that places it, and `Projection` — an enum whose current variant holds the numbers that zoom
//! it. A script can reach all three with what `docs/host-api.md` documents under "Components by
//! name", and `tests/camera.rs` is that, spelled out. What is spelled out there is awkward,
//! though: the projection's variant has to be named on every write, a tuple variant takes an
//! Array and not a Hash, and two zooms in one tick come to one, because a write lands at the end
//! of the frame.
//!
//! So rubevy carries an **optional Ruby layer** over it, and the app says whether its scripts get
//! it — one line, `world.load_and_run(rubevy::layers::CAMERA)` at `Startup`. The script below is
//! what the same work looks like through it; `src/layers/camera.rb` is the layer itself, and it
//! is Ruby over `Rubevy.find` and the component writes, with no Rust and no new dependency.
//!
//! Headless: this renders nothing, so it wants no window and no GPU. It registers the three types
//! itself, as `examples/components.rs` registers `Transform` — `MinimalPlugins` registers none of
//! bevy's own. It also compiles its Ruby in process (`sabiruby-compiler`, a dev-dependency)
//! rather than loading a committed `.mrb`, so that the script is here to read beside its output.
//!
//!     cargo run --example camera_from_ruby

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::asset::AssetPlugin;
use bevy::camera::{Camera2d, OrthographicProjection, Projection};
use bevy::log::LogPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, RubevyPlugin, Script, ScriptEnded, ScriptWorld};

const SCRIPT: &str = r#"
  cam = Rubevy::Camera.find
  Rubevy.log "camera: #{cam.inspect}, components #{cam.entity.components.join(', ')}"

  # Panning. The layer keeps z — in 2D that is the drawing order, not a place — and remembers
  # where it put the camera, so two moves in one tick both count (the world would still be
  # answering the value from before the first one, since a write lands at the end of the frame).
  cam.move_to(100.0, 0.0)
  cam.pan(20.0, -40.0)

  # Zooming. `zoom 2` is twice as close on a 2D camera and on a 3D one alike: the layer divides
  # the orthographic `scale` and works the perspective `fov` out through its tangent. Twice,
  # here, in one tick — which is the other thing the layer holds the magnification for.
  cam.zoom 2
  cam.zoom 2
  Rubevy.log "now at #{cam.position.inspect} at #{cam.magnification}x"

  sleep 0                                  # the clock moves on, and the frame's writes land
  there = cam.entity[:Transform][:translation]
  scale = cam.entity[:Projection][:Orthographic][0][:scale]
  Rubevy.log "the world says #{there.inspect} at scale #{scale}"
  Rubevy.log "refused: #{cam.rejected_writes.length}"

  # A question no component can answer — where a point on the window is in the world — is thrown
  # at the game and answered by it. The layer hands the queue back rather than waiting on it.
  where = cam.world_at(640.0, 360.0).pop
  Rubevy.log "the middle of the window is #{where.inspect}"
"#;

fn main() {
    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(16))),
            LogPlugin::default(),
            AssetPlugin::default(),
            RubevyPlugin::default(),
        ))
        // The three types the script names. `DefaultPlugins` in a real game registers bevy's own;
        // `MinimalPlugins` registers nothing and this crate is not built with bevy's
        // `reflect_auto_register`, so an unregistered type would simply read as nil.
        .register_type::<Transform>()
        .register_type::<Camera2d>()
        .register_type::<Projection>()
        .add_systems(Startup, (add_the_camera_layer, spawn))
        .add_systems(Update, (answer_world_at, report_ended))
        .run();
}

/// The whole of taking the layer up. An app that leaves this out has no `Rubevy::Camera`.
fn add_the_camera_layer(mut world: ResMut<ScriptWorld>) {
    world.load_and_run(rubevy::layers::CAMERA).expect("the camera layer runs");
}

fn spawn(mut commands: Commands, mut assets: ResMut<Assets<MrbAsset>>) {
    // The camera's own components are put on by hand rather than by `Camera2d`'s required
    // components, because a required `Camera` wants a render target and this example renders
    // nothing. These three are the ones a script touches.
    commands.spawn((
        Camera2d,
        Transform::from_xyz(0.0, 0.0, 0.0),
        Projection::Orthographic(OrthographicProjection::default_2d()),
    ));

    let opts = sabiruby_compiler::Options { filename: "camera.rb".into(), ..Default::default() };
    let bytes = sabiruby_compiler::compile(SCRIPT.as_bytes(), &opts).expect("the script compiles");
    let script = assets.add(MrbAsset { bytes });
    commands.spawn(Script::new(script).with_name("camera"));
}

/// The answering side of `cam.world_at`, which is the game's: rubevy answers nothing for it, and
/// a script that popped the queue of a question nobody answers would park for ever. Working the
/// real answer out wants the window's size and the camera's projection; this one is the shape of
/// it and not the arithmetic.
fn answer_world_at(
    mut world: ResMut<ScriptWorld>,
    cameras: Query<(&Transform, &Projection), With<Camera2d>>,
) {
    for request in world.take_requests() {
        if request.kind != "camera.world_at" {
            world.answer(&request, Answer::Nil);
            continue;
        }
        let camera = request.entity_arg(0).and_then(|e| cameras.get(e).ok());
        let (x, y) = (request.num_or(1, 0.0), request.num_or(2, 0.0));
        let answer = match camera {
            Some((at, Projection::Orthographic(o))) => Answer::Text(format!(
                "({}, {})",
                at.translation.x + (x as f32 - 640.0) * o.scale,
                at.translation.y - (y as f32 - 360.0) * o.scale
            )),
            _ => Answer::Nil,
        };
        world.answer(&request, answer);
    }
}

fn report_ended(
    mut ended: MessageReader<ScriptEnded>,
    cameras: Query<(&Transform, &Projection), With<Camera2d>>,
    mut exit: MessageWriter<AppExit>,
) {
    for e in ended.read() {
        for (at, projection) in &cameras {
            let scale = match projection {
                Projection::Orthographic(o) => o.scale,
                _ => f32::NAN,
            };
            info!("host: the camera is at {:?} with scale {}", at.translation, scale);
        }
        info!("host: script on {:?} ended: {:?} {}", e.entity, e.status, e.value);
        exit.write(AppExit::Success);
    }
}
