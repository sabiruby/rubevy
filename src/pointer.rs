//! **The mouse, in world units** — a press, a drag, where it let go, and where it is: four
//! messages, for one camera the app names ([`PointerPlugin`]). Behind the `pointer` feature.
//!
//! | message | when |
//! |---|---|
//! | [`WorldGrab`] | a button went down |
//! | [`WorldClick`] | it came up without having travelled more than [`Pointer::click_slop`] |
//! | [`WorldDrop`] | it came up after travelling further — a drag ended |
//! | [`WorldMove`] | the point under the cursor moved, button down or not |
//!
//! A press always begins with a [`WorldGrab`] and always ends as exactly one of [`WorldClick`] or
//! [`WorldDrop`], so a game that picks something up on the grab always gets a word for putting
//! it down.
//!
//! **Why this is Rust and a feature, and not a Ruby layer** (`rubevy::layers`). Where a point on
//! the window is in the world is the camera's placing and its projection worked backwards —
//! `Camera::viewport_to_world_2d` — which no component holds and which a script can only ask the
//! game for (`Rubevy::Camera#world_at` is that question). The messages are Bevy's, for the game's
//! systems; a game that wants its scripts to hear them publishes them
//! ([`ScriptWorld::publish`](crate::ScriptWorld::publish)), which is one line per message and
//! the game's choice of names (`examples/pointer.rs`, with the control layer). The feature brings
//! `bevy_camera` (and `bevy_window` with it) and bevy's `mouse` into the build; without it the
//! crate's dependencies are what they were.
//!
//! **Where the shape comes from.** An embedding wrote this three times before it was made general
//! — a camera the game drives, a desk, a factory floor — and what differed between the copies
//! was the camera, which is the type parameter here. The fourth message, [`WorldMove`], is new:
//! the copies read [`Pointer::at`] every frame instead, which still works.
//!
//! **2D.** The point is `viewport_to_world_2d`'s: x and y on the camera's near plane, which for
//! an orthographic camera is the world point under the cursor. A 3D game wants a ray and a
//! surface to meet it, which is the game's (`Camera::viewport_to_world`).

use std::marker::PhantomData;

use bevy::camera::Camera;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

/// A button went down, at `at` in the world.
#[derive(Message, Debug, Clone, Copy, PartialEq)]
pub struct WorldGrab {
    pub at: Vec2,
    pub button: MouseButton,
    /// The camera the point was worked out through.
    pub camera: Entity,
}

/// A button came up where it went down — within [`Pointer::click_slop`] pixels of it — so a
/// click and not a drag.
#[derive(Message, Debug, Clone, Copy, PartialEq)]
pub struct WorldClick {
    pub at: Vec2,
    pub button: MouseButton,
    pub camera: Entity,
}

/// A button came up after a drag: `at` is where it landed, `from` where it went down.
#[derive(Message, Debug, Clone, Copy, PartialEq)]
pub struct WorldDrop {
    pub at: Vec2,
    pub from: Vec2,
    pub button: MouseButton,
    pub camera: Entity,
}

/// The point under the cursor moved (the cursor did, or the camera did under it). `dragging` is
/// the button of a drag under way, if one is.
#[derive(Message, Debug, Clone, Copy, PartialEq)]
pub struct WorldMove {
    pub at: Vec2,
    pub from: Vec2,
    pub dragging: Option<MouseButton>,
    pub camera: Entity,
}

/// **The default of [`Pointer::click_slop`]**, in logical pixels of the window. It is not
/// measured: it is the value rubevy_games' `games-shell` uses (`CLICK_SLOP`), which that crate
/// says it inherited without a recorded source (`docs/numbers.md`). A game moves it on the
/// resource.
pub const DEFAULT_CLICK_SLOP: f32 = 6.0;

/// **What the pointer is doing**, and where it is in the world, as seen through the camera
/// marked `C`.
#[derive(Resource, Debug)]
pub struct Pointer<C = Camera> {
    /// How far the cursor may travel between down and up, in logical pixels of the window, and
    /// the press still be a click. Once a press has gone further it is a drag for good, even if
    /// it comes back: a thing picked up does not snap back half way.
    pub click_slop: f32,
    /// Where the cursor is in the world, or `None` while it is outside the window (or there is no
    /// camera `C` to see it through).
    pub at: Option<Vec2>,
    /// Which buttons make grabs, clicks and drops. Every button by default; a game that gives
    /// only the left one a meaning says so here.
    pub buttons: Vec<MouseButton>,
    presses: Vec<Press>,
    _camera: PhantomData<fn() -> C>,
}

impl<C> Default for Pointer<C> {
    fn default() -> Self {
        Pointer {
            click_slop: DEFAULT_CLICK_SLOP,
            at: None,
            buttons: vec![
                MouseButton::Left,
                MouseButton::Right,
                MouseButton::Middle,
                MouseButton::Back,
                MouseButton::Forward,
            ],
            presses: Vec::new(),
            _camera: PhantomData,
        }
    }
}

impl<C> Pointer<C> {
    /// The button of the drag under way, if a press has travelled past the slop.
    pub fn dragging(&self) -> Option<MouseButton> {
        self.presses.iter().find(|p| p.dragging).map(|p| p.button)
    }
}

#[derive(Debug, Clone, Copy)]
struct Press {
    button: MouseButton,
    /// Where it went down in the world, for [`WorldDrop::from`].
    in_the_world: Vec2,
    /// Where it went down on the window, which is what the slop is measured in.
    on_the_window: Vec2,
    dragging: bool,
}

/// Adds [`Pointer<C>`] and the four messages, read through the camera that has the component `C`
/// — `PointerPlugin::<Camera>` (the default) for the first active camera there is,
/// `PointerPlugin::<MyBoardCamera>` for the one the game marked. The system runs in
/// `PreUpdate`, after bevy's input, so the messages are there for `Update`.
pub struct PointerPlugin<C = Camera>(PhantomData<fn() -> C>);

impl<C> Default for PointerPlugin<C> {
    fn default() -> Self {
        PointerPlugin(PhantomData)
    }
}

impl<C: Component> Plugin for PointerPlugin<C> {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pointer<C>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .add_message::<WorldGrab>()
            .add_message::<WorldClick>()
            .add_message::<WorldDrop>()
            .add_message::<WorldMove>()
            .add_systems(PreUpdate, watch_the_pointer::<C>.after(bevy::input::InputSystems));
    }
}

/// **The one system**: the cursor in world units, and the four messages.
///
/// It reads the window's cursor position rather than `CursorMoved`, so that a press with no
/// movement at all still knows where it is, and a test can put the cursor where it likes by
/// writing the window's own field.
#[allow(clippy::too_many_arguments)]
fn watch_the_pointer<C: Component>(
    mut pointer: ResMut<Pointer<C>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(Entity, &Camera, &GlobalTransform), With<C>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut grabbed: MessageWriter<WorldGrab>,
    mut clicked: MessageWriter<WorldClick>,
    mut dropped: MessageWriter<WorldDrop>,
    mut moved: MessageWriter<WorldMove>,
) {
    let cursor = windows.single().ok().and_then(Window::cursor_position);
    let camera = cameras.iter().find(|(_, c, _)| c.is_active);
    let seen = match (cursor, camera) {
        (Some(cursor), Some((entity, camera, placed))) => {
            camera.viewport_to_world_2d(placed, cursor).ok().map(|at| (cursor, at, entity))
        }
        _ => None,
    };
    let Some((cursor, at, camera)) = seen else {
        // outside the window: a press under way stays under way, and ends where the cursor comes
        // back in (or does not end, if the button comes up outside — bevy then sees no release
        // either, on most platforms)
        pointer.at = None;
        return;
    };
    let slop = pointer.click_slop;
    let listened = pointer.buttons.clone();
    // the order is the reader's: a grab before the move it starts, a move that already says it is
    // a drag, and the drop or click last
    for &button in &listened {
        if buttons.just_pressed(button) {
            pointer.presses.retain(|p| p.button != button);
            pointer.presses.push(Press { button, in_the_world: at, on_the_window: cursor, dragging: false });
            grabbed.write(WorldGrab { at, button, camera });
        }
    }
    for press in &mut pointer.presses {
        // **a press becomes a drag once and stays one**
        if !press.dragging && cursor.distance(press.on_the_window) > slop {
            press.dragging = true;
        }
    }
    let before = pointer.at.replace(at);
    if let Some(from) = before
        && from != at
    {
        moved.write(WorldMove { at, from, dragging: pointer.dragging(), camera });
    }
    for &button in &listened {
        if !buttons.just_released(button) {
            continue;
        }
        let Some(i) = pointer.presses.iter().position(|p| p.button == button) else { continue };
        let press = pointer.presses.remove(i);
        if press.dragging {
            dropped.write(WorldDrop { at, from: press.in_the_world, button, camera });
        } else {
            clicked.write(WorldClick { at, button, camera });
        }
    }
}
