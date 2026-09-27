//! The mouse in world units (`rubevy::pointer`, the `pointer` feature) heard by a script written
//! as handlers (`rubevy::layers::CONTROL`).
//!
//! The game adds `PointerPlugin::<Board>` — `Board` is the marker on the camera the board is seen
//! through — and passes the messages it wants on to its scripts, one line each: which messages a
//! script hears, and under what names, is the game's to say. The script is `on(:clicked) { … }`
//! and `on(:dropped) { … }`, each a task that may wait: the click handler asks the game a question
//! and waits for the answer inside the block.
//!
//! Headless: there is no window system here, so a stand-in for the hand writes the window's cursor
//! and the button state a frame at a time — a click, then a drag — and the camera's computed
//! values are written in as bevy_render would write them for an 800×600 window.
//!
//!     cargo run --example pointer --features pointer

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::asset::AssetPlugin;
use bevy::camera::{Camera, ComputedCameraValues, RenderTargetInfo};
use bevy::diagnostic::FrameCount;
use bevy::log::LogPlugin;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rubevy::pointer::{PointerPlugin, WorldClick, WorldDrop};
use rubevy::{Answer, MrbAsset, RubevyPlugin, Script, ScriptWorld};

const SCRIPT: &str = r#"
  on(:clicked) do |x, y|
    # a handler is a task: it can wait for the game, here inside the block
    what = Rubevy.ask("board.what_is_at", x, y).pop
    @clicks = (@clicks || 0) + 1
    Rubevy.log "click #{@clicks} at #{x.round}, #{y.round}: #{what}"
  end

  on(:dropped) do |from_x, from_y, x, y|
    Rubevy.log "dragged from #{from_x.round}, #{from_y.round} to #{x.round}, #{y.round}"
    Rubevy.entity[:Transform] = { translation: [x, y, 0.0] }   # and the piece goes there
  end

  Rubevy::Control.run
"#;

const W: f32 = 800.0;
const H: f32 = 600.0;

#[derive(Component)]
struct Board;

fn main() {
    App::new()
        .add_plugins((
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_millis(16))),
            LogPlugin::default(),
            AssetPlugin::default(),
            RubevyPlugin::default(),
            PointerPlugin::<Board>::default(),
        ))
        .register_type::<Transform>()
        .add_systems(Startup, (add_the_control_layer, spawn))
        .add_systems(PreUpdate, the_hand.before(bevy::input::InputSystems))
        .add_systems(Update, (pass_the_pointer_on, answer))
        .run();
}

fn add_the_control_layer(mut world: ResMut<ScriptWorld>) {
    world.load_and_run(rubevy::layers::CONTROL).expect("the layer runs");
}

fn spawn(mut commands: Commands, mut scripts: ResMut<Assets<MrbAsset>>) {
    let opts = sabiruby_compiler::Options { filename: "board.rb".into(), ..Default::default() };
    let bytes = sabiruby_compiler::compile(SCRIPT.as_bytes(), &opts).expect("compiles");
    commands.spawn((Script::new(scripts.add(MrbAsset { bytes })), Transform::default()));

    let mut window = Window::default();
    window.resolution.set(W, H);
    commands.spawn((window, PrimaryWindow));
    let camera = Camera {
        computed: ComputedCameraValues {
            clip_from_view: Mat4::orthographic_rh(-W / 2.0, W / 2.0, -H / 2.0, H / 2.0, -1000.0, 1000.0),
            target_info: Some(RenderTargetInfo { physical_size: UVec2::new(W as u32, H as u32), scale_factor: 1.0 }),
            ..default()
        },
        ..default()
    };
    commands.spawn((camera, Transform::default(), GlobalTransform::default(), Board));
}

/// The whole of the bridge from the pointer to the scripts: one line per message.
fn pass_the_pointer_on(
    mut clicks: MessageReader<WorldClick>,
    mut drops: MessageReader<WorldDrop>,
    mut scripts: ResMut<ScriptWorld>,
) {
    for c in clicks.read() {
        scripts.publish(None, "clicked", Answer::List(vec![c.at.x as f64, c.at.y as f64]));
    }
    for d in drops.read() {
        let list = vec![d.from.x as f64, d.from.y as f64, d.at.x as f64, d.at.y as f64];
        scripts.publish(None, "dropped", Answer::List(list));
    }
}

fn answer(mut scripts: ResMut<ScriptWorld>) {
    for r in scripts.take_requests() {
        if r.kind == "board.what_is_at" {
            let x = r.num_or(0, 0.0);
            let what = if x < 0.0 { "the left half" } else { "the right half" };
            scripts.answer(&r, Answer::Text(what.into()));
        }
    }
}

/// A stand-in for the hand: where the cursor is and what the button does, frame by frame — a
/// click at (-100, 50), then a drag from (0, 0) to (200, -80) — and then the example ends.
fn the_hand(
    frame: Res<FrameCount>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut input: ResMut<ButtonInput<MouseButton>>,
    pieces: Query<&Transform, With<Script>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Ok(mut window) = windows.single_mut() else { return };
    let at = |x: f32, y: f32| Some(Vec2::new(W / 2.0 + x, H / 2.0 - y));
    input.clear();
    match frame.0 {
        5 => window.set_cursor_position(at(-100.0, 50.0)),
        6 => input.press(MouseButton::Left),
        7 => input.release(MouseButton::Left),
        10 => window.set_cursor_position(at(0.0, 0.0)),
        11 => input.press(MouseButton::Left),
        12 => window.set_cursor_position(at(120.0, -40.0)),
        13 => window.set_cursor_position(at(200.0, -80.0)),
        14 => input.release(MouseButton::Left),
        20 => {
            for t in &pieces {
                info!("host: the piece is at {:?}", t.translation.round());
            }
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
