//! Panning and zooming a camera from Ruby, with nothing in rubevy that knows what a camera is.
//!
//! Bevy's camera is three ordinary reflected components: the marker (`Camera2d`), the `Transform`
//! that places it, and `Projection` — an enum whose current variant holds the numbers that zoom
//! it. So the camera is reachable with what `docs/host-api.md` already documents under
//! "Components by name": `Rubevy.find`, `e[:Transform] =`, `e[:Projection] =`.
//!
//! The one thing worth reading twice is the write to `Projection`. It is an enum, and the variant
//! it is in is a *tuple* of one — `Projection::Orthographic(OrthographicProjection { .. })` — so
//! the value is a one-entry Hash naming the variant, holding the Array of the variant's fields,
//! holding a partial Hash for the projection itself:
//!
//! ```text
//! cam[:Projection] = { Orthographic: [ { scale: 2.0 } ] }
//! ```
//!
//! The variant has to be the one the component is already in; switching a tuple variant from Ruby
//! is refused on purpose (`src/reflect.rs`, `apply_enum`). The measurements and the reasoning are
//! in `docs/worklog/2026-09-20-camera-from-ruby.md`, and `tests/camera.rs` holds the same ground
//! as assertions.
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
use rubevy::{MrbAsset, RubevyPlugin, Script, ScriptEnded};

const SCRIPT: &str = r#"
  cam = Rubevy.find(:Camera2d)[0]
  Rubevy.log "camera: #{cam.inspect}, components #{cam.components.join(', ')}"

  # Reading the projection: a one-entry Hash naming the variant it is in.
  shown = cam[:Projection]
  Rubevy.log "projection: #{shown.keys[0]} #{shown[:Orthographic][0][:scale]}x"

  # Panning is an ordinary partial Transform write; the fields not named keep their values.
  tf = cam[:Transform]
  tf[:translation][0] += 120.0
  tf[:translation][1] -= 40.0
  cam[:Transform] = tf

  # Zooming is a write into the current variant of the enum.
  cam[:Projection] = { Orthographic: [ { scale: 2.0 } ] }

  sleep 0.05                       # a frame goes by, and with it both writes

  at = cam[:Transform][:translation]
  Rubevy.log "now at #{at[0]}, #{at[1]} at #{cam[:Projection][:Orthographic][0][:scale]}x"

  # Switching the variant is refused: bevy would have to build the whole new variant out of a
  # Ruby Hash, which says nothing about types. The line below is logged and skipped, and the
  # camera stays orthographic.
  cam[:Projection] = { Perspective: [ { fov: 1.0 } ] }
  sleep 0.05
  Rubevy.log "still #{cam[:Projection].keys[0]}"
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
        .add_systems(Startup, spawn)
        .add_systems(Update, report_ended)
        .run();
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
            info!("host: the camera is at {:?} at {}x", at.translation, scale);
        }
        info!("host: script on {:?} ended: {:?} {}", e.entity, e.status, e.value);
        exit.write(AppExit::Success);
    }
}
