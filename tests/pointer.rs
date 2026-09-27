//! `rubevy::pointer` (the `pointer` feature): the four messages, through a camera the app names.
//!
//! There is no renderer here, so the camera's computed values — which bevy_render's
//! `camera_system` would fill — are written by the test: an orthographic projection of an
//! 800×600 window at scale 1, so a cursor `dx` logical pixels right of the middle is `dx` world
//! units right of the camera (to within the float error of inverting the matrix, which is why
//! the points are rounded before they are compared).
#![cfg(feature = "pointer")]

use bevy::camera::{Camera, ComputedCameraValues, RenderTargetInfo};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rubevy::pointer::{Pointer, PointerPlugin, WorldClick, WorldDrop, WorldGrab, WorldMove};

const W: f32 = 800.0;
const H: f32 = 600.0;

/// The marker of the camera the pointer is to look through; a second camera without it is there
/// to show that the other one is not used.
#[derive(Component)]
struct Board;

#[derive(Resource, Default)]
struct Heard {
    grabs: Vec<WorldGrab>,
    clicks: Vec<WorldClick>,
    drops: Vec<WorldDrop>,
    moves: Vec<WorldMove>,
}

fn hear(
    mut heard: ResMut<Heard>,
    mut g: MessageReader<WorldGrab>,
    mut c: MessageReader<WorldClick>,
    mut d: MessageReader<WorldDrop>,
    mut m: MessageReader<WorldMove>,
) {
    heard.grabs.extend(g.read().copied());
    heard.clicks.extend(c.read().copied());
    heard.drops.extend(d.read().copied());
    heard.moves.extend(m.read().copied());
}

fn camera_at(x: f32, y: f32) -> (Camera, Transform, GlobalTransform) {
    let camera = Camera {
        computed: ComputedCameraValues {
            clip_from_view: Mat4::orthographic_rh(-W / 2.0, W / 2.0, -H / 2.0, H / 2.0, -1000.0, 1000.0),
            target_info: Some(RenderTargetInfo { physical_size: UVec2::new(W as u32, H as u32), scale_factor: 1.0 }),
            ..default()
        },
        ..default()
    };
    let t = Transform::from_xyz(x, y, 0.0);
    (camera, t, GlobalTransform::from(t))
}

fn app() -> (App, Entity) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(PointerPlugin::<Board>::default())
        .init_resource::<Heard>()
        .add_systems(Update, hear);
    let mut window = Window::default();
    window.resolution.set(W, H);
    app.world_mut().spawn((window, PrimaryWindow));
    // the other camera, first in the world and far away: it must not be the one looked through
    app.world_mut().spawn(camera_at(10_000.0, 10_000.0));
    let board = app.world_mut().spawn((camera_at(100.0, 50.0), Board)).id();
    (app, board)
}

/// Puts the cursor `(dx, dy)` logical pixels from the middle of the window (y up, as the world's).
fn cursor(app: &mut App, dx: f32, dy: f32) {
    let mut q = app.world_mut().query_filtered::<&mut Window, With<PrimaryWindow>>();
    let mut w = q.single_mut(app.world_mut()).unwrap();
    w.set_cursor_position(Some(Vec2::new(W / 2.0 + dx, H / 2.0 - dy)));
}

/// One frame with the button pressed or released on it (`ButtonInput` is not cleared by an
/// `InputPlugin` here, so the test clears it itself).
fn frame(app: &mut App, press: Option<bool>) {
    {
        let mut input = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        input.clear();
        match press {
            Some(true) => input.press(MouseButton::Left),
            Some(false) => input.release(MouseButton::Left),
            None => {}
        }
    }
    app.update();
}

#[test]
fn a_press_that_does_not_travel_is_a_click_in_world_units() {
    let (mut app, board) = app();
    cursor(&mut app, 10.0, 20.0);
    frame(&mut app, None);
    assert_eq!(app.world().resource::<Pointer<Board>>().at.map(Vec2::round), Some(Vec2::new(110.0, 70.0)));
    frame(&mut app, Some(true));
    cursor(&mut app, 12.0, 20.0); // inside the slop
    frame(&mut app, None);
    frame(&mut app, Some(false));

    let heard = app.world().resource::<Heard>();
    let grabs: Vec<(Vec2, MouseButton, Entity)> = heard.grabs.iter().map(|g| (g.at.round(), g.button, g.camera)).collect();
    assert_eq!(grabs, vec![(Vec2::new(110.0, 70.0), MouseButton::Left, board)]);
    assert_eq!(heard.clicks.len(), 1);
    assert_eq!(heard.clicks[0].at.round(), Vec2::new(112.0, 70.0));
    assert!(heard.drops.is_empty());
}

#[test]
fn a_press_that_travels_is_a_drop_and_moves_say_so() {
    let (mut app, _) = app();
    cursor(&mut app, 0.0, 0.0);
    frame(&mut app, None);
    frame(&mut app, Some(true));
    cursor(&mut app, 50.0, 0.0);
    frame(&mut app, None);
    cursor(&mut app, 1.0, 0.0); // back inside the slop: still a drag
    frame(&mut app, None);
    frame(&mut app, Some(false));

    let heard = app.world().resource::<Heard>();
    assert!(heard.clicks.is_empty());
    assert_eq!(heard.drops.len(), 1);
    assert_eq!((heard.drops[0].from.round(), heard.drops[0].at.round()), (Vec2::new(100.0, 50.0), Vec2::new(101.0, 50.0)));
    let moves: Vec<(Vec2, Option<MouseButton>)> = heard.moves.iter().map(|m| (m.at.round(), m.dragging)).collect();
    assert_eq!(
        moves,
        vec![(Vec2::new(150.0, 50.0), Some(MouseButton::Left)), (Vec2::new(101.0, 50.0), Some(MouseButton::Left))]
    );
}

#[test]
fn the_slop_and_the_buttons_are_the_games() {
    let (mut app, _) = app();
    {
        let mut p = app.world_mut().resource_mut::<Pointer<Board>>();
        p.click_slop = 100.0;
    }
    cursor(&mut app, 0.0, 0.0);
    frame(&mut app, None);
    frame(&mut app, Some(true));
    cursor(&mut app, 50.0, 0.0);
    frame(&mut app, None);
    frame(&mut app, Some(false));
    assert_eq!(app.world().resource::<Heard>().clicks.len(), 1, "50 px is inside a slop of 100");

    app.world_mut().resource_mut::<Pointer<Board>>().buttons = vec![MouseButton::Right];
    frame(&mut app, Some(true));
    frame(&mut app, Some(false));
    let heard = app.world().resource::<Heard>();
    assert_eq!((heard.grabs.len(), heard.clicks.len()), (1, 1), "the left button is not listened to any more");
}
