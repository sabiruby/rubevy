//! rubevy: run mruby bytecode inside Bevy, using the [SabiRuby](sabiruby) VM.
//!
//! One VM for the whole app ([`ScriptWorld`]), and one **task** per script
//! (mruby-task): a [`Script`] component becomes a task of the scheduler, and
//! every frame the plugin gives the scheduler a budget of instructions. A task
//! that calls `sleep` costs nothing until its time comes, so hundreds of
//! scripts can sit on entities and wake only when they have something to do.
//! A budget of zero pauses them all, and a paused VM does not age
//! ([`ScriptWorld::budget`]).
//!
//! What a script sees of the host:
//! * `$rubevy` — a Hash refreshed at the head of every frame (`:frame`,
//!   `:delta`, `:time`).
//! * `Rubevy.entity` — the entity this script is attached to, read off the task
//!   the scheduler is running. It is a `Rubevy::Entity` object, which carries
//!   `Entity::to_bits` as a handle the VM never reads through (`Vm::data_new`);
//!   `to_i` gives the number, `==` compares by it.
//! * `Rubevy.log`, `Rubevy.spawn`, `Rubevy.despawn`, `Rubevy.set_position`,
//!   `Rubevy.move_to` — these do not touch the Bevy world from inside the VM
//!   (a native cannot); they put a command on a queue that a system drains
//!   after the frame's scripts have run.
//! * **Components by name**: `entity[:Transform]` answers a Hash of its
//!   fields, `entity[:Transform] = hash` writes back the fields the Hash names,
//!   and `entity.has?`, `entity.components` and `Rubevy.find(:Npc)` say what is
//!   where. It goes through Bevy's reflection, so no type is named in rubevy —
//!   a game's own component joins in by being
//!   `#[derive(Reflect)] #[reflect(Component)]` and registered. A read is
//!   answered inside the frame it was made in — the tick runs the scripts,
//!   answers the reads they stopped on and runs them again — so the value comes
//!   back in the line that asked for it. A write still lands at the end of the
//!   frame, as `Rubevy.spawn` does.
//! * **Events**: `Rubevy.subscribe(:hit)` answers a `Task::Queue` the game
//!   pushes onto ([`ScriptWorld::publish`]), so a script waits for something to
//!   happen exactly as it waits for an answer — in its own task, or in one it
//!   made with `Task.new`.
//! * `puts`/`p` output is forwarded to Bevy's log.
//!
//! Where a game's own systems go in the frame is [`RubevySet`]: an answering
//! system in [`RubevySet::Answer`] makes a `Rubevy.ask` round trip cost one
//! frame.
//!
//! A script may `require` another, which reads from the asset directory
//! ([`RubevyPlugin::with_asset_root`], `assets` by default): `.mrb` always,
//! `.rb` only in a build with the `ruby-source` feature, which brings the
//! reference compiler along.
//!
//! Scripts share one VM, so they share globals and constants. That is the
//! design, not an oversight: a game's scripts are written together. A use that
//! needs isolation — mods, a player's own script — gives that side a **second
//! VM**, which is the plugin added a second time under a name tag:
//!
//! ```no_run
//! # use bevy::prelude::*;
//! # use rubevy::RubevyPlugin;
//! struct Mods;
//! # fn build(app: &mut App) {
//! app.add_plugins(RubevyPlugin::default())                        // the game's VM
//!     .add_plugins(RubevyPlugin::<Mods>::for_vm("assets/mods"));  // the mods' VM
//! # }
//! ```
//!
//! Everything the plugin owns then exists twice: [`ScriptWorld<Mods>`] is that
//! VM ([`ScriptWorld`] is still the first one), [`Script<Mods>`] is a script of
//! it ([`Script::for_vm`]), its systems are ordered by
//! `RubevySet::<Mods>::deliver()` / `tick()` / `answer()`, and its scripts'
//! ends arrive as [`ScriptEnded<Mods>`]. Heap, globals, constants, classes,
//! symbols, GC, the scheduler, subscriptions, the frame budget and `require`'s
//! load path are that VM's own, and handing one VM's [`ScriptTask`] to another
//! is a compile error rather than a quiet mistake. The costs are memory (a VM
//! is about half a megabyte, and a `.mrb` loaded into two VMs is two copies of
//! its irep) and frame time: the budget is per VM, so a frame's worst case is
//! the sum of the VMs' [`ScriptWorld::frame_time`]s. `docs/host-api.md` has
//! the section, and `examples/two_vms.rs` the working app.

mod embed;
mod reflect;
mod source;

pub use crate::embed::{CompileFn, EmbeddedHost};
pub use crate::source::{in_the_authors_lines, Program};

use bevy::asset::{io::Reader, Asset, AssetApp, AssetLoader, LoadContext};
use bevy::diagnostic::FrameCount;
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::tasks::{block_on, poll_once, AsyncComputeTaskPool, Task};

use std::marker::PhantomData;

use sabiruby::convert::{DataRef, FromRuby, This};
use sabiruby::value::ObjId;
use sabiruby::{Value, Vm, VmError};

use crate::reflect::RubyData;

/// The bytes of a RITE binary (`.mrb`).
#[derive(Asset, TypePath, Debug, Clone)]
pub struct MrbAsset {
    pub bytes: Vec<u8>,
}

#[derive(Default, TypePath)]
pub struct MrbLoader;

#[derive(Debug, thiserror::Error)]
pub enum MrbLoadError {
    #[error("could not read .mrb: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a RITE binary: {0}")]
    Rite(String),
}

impl AssetLoader for MrbLoader {
    type Asset = MrbAsset;
    type Settings = ();
    type Error = MrbLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        // Validate early so a broken file fails at load time, not at spawn time.
        sabiruby::rite::parse(&bytes).map_err(|e| MrbLoadError::Rite(e.to_string()))?;
        Ok(MrbAsset { bytes })
    }

    fn extensions(&self) -> &[&str] {
        &["mrb"]
    }
}

/// Attach to an entity to run a script as a task.
///
/// `M` is the name tag of the VM the script belongs to. Left out — `Script`, which is
/// `Script<()>` — it is the app's first VM, which is what an app with one VM ever writes. An app
/// that started a second one with `RubevyPlugin::<Mods>` gives that VM's scripts `Script<Mods>`,
/// and the two are different components: a system of one VM never sees the other's.
///
/// `Debug` and `Clone` are written out below rather than derived, because a derive would ask the
/// name tag itself to be `Debug` and `Clone` — and a name tag is usually an empty `struct Mods;`
/// that implements nothing at all.
#[derive(Component)]
pub struct Script<M = ()> {
    pub source: Handle<MrbAsset>,
    /// 0-255, 0 first (mruby-task's priority).
    pub priority: u8,
    /// Shown in logs and answered by `Task#name`.
    pub name: Option<String>,
    /// The name tag, which is a type and never a value. `fn() -> M` rather than `M` so that the
    /// component is `Send + Sync` whatever the tag is (see the crate's `docs/plans`).
    _m: PhantomData<fn() -> M>,
}

impl Script<()> {
    /// A script of the app's first VM.
    ///
    /// It is spelled without a name tag because Rust does not fall back to a type parameter's
    /// default in an expression: `Script::new(handle)` with a generic `new` would be a type it
    /// cannot infer. The VM with a tag has [`Script::for_vm`].
    pub fn new(source: Handle<MrbAsset>) -> Script<()> {
        Script::for_vm(source)
    }
}

impl<M: 'static> Script<M> {
    /// A script of the VM named `M`: `Script::<Mods>::for_vm(handle)`.
    pub fn for_vm(source: Handle<MrbAsset>) -> Script<M> {
        Script { source, priority: 128, name: None, _m: PhantomData }
    }
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }
}

impl<M> std::fmt::Debug for Script<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Script")
            .field("source", &self.source)
            .field("priority", &self.priority)
            .field("name", &self.name)
            .finish()
    }
}

impl<M> Clone for Script<M> {
    fn clone(&self) -> Self {
        Script {
            source: self.source.clone(),
            priority: self.priority,
            name: self.name.clone(),
            _m: PhantomData,
        }
    }
}

/// The task a [`Script`] became. Added by the plugin once the asset arrives.
///
/// Removing it (or despawning the entity) stops the task: it is terminated in the VM, so a script
/// replaced by a new [`Script`] — a reload, a restart — does not keep running beside the new one.
///
/// The name tag `M` says which VM the task belongs to. It is what keeps an [`ObjId`] of one VM
/// from being handed to another: an `ObjId` is an index into a VM's own heap, so the same number
/// names a different object in the VM next door, and a mistake there would be quiet. With the
/// tag it is a compile error instead.
#[derive(Component)]
#[component(on_remove = stop_removed_task::<M>)]
pub struct ScriptTask<M = ()> {
    task: ObjId,
    _m: PhantomData<fn() -> M>,
}

impl<M> ScriptTask<M> {
    /// The scheduler's id for this script, for the `Vm::task_*` entry points — of **this**
    /// script's VM, which is the one named `M`.
    pub fn task(&self) -> ObjId {
        self.task
    }
}

// as on `Script`: derived, these would ask the name tag to be `Debug` / `Clone` / `Copy` itself
impl<M> std::fmt::Debug for ScriptTask<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptTask").field("task", &self.task).finish()
    }
}

impl<M> Clone for ScriptTask<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Copy for ScriptTask<M> {}

fn stop_removed_task<M: 'static>(
    mut world: bevy::ecs::world::DeferredWorld,
    context: bevy::ecs::lifecycle::HookContext,
) {
    let Some(task) = world.get::<ScriptTask<M>>(context.entity).map(|t| t.task) else { return };
    // a script that ended has been let go of already (`ScriptDone`) — this VM's marker, not the
    // one of another VM that shares the entity
    let ended = world.get::<ScriptDone<M>>(context.entity).is_some();
    let Some(mut scripts) = world.get_resource_mut::<ScriptWorld<M>>() else { return };
    scripts.stop_task(task, !ended);
    // and anything it was listening for: a queue nobody will read is one the game would keep
    // filling (`Rubevy.subscribe`)
    scripts.unsubscribe(context.entity);
}

/// What a host can show of a running script (`ScriptWorld::stats`).
#[derive(Debug, Clone, Default)]
pub struct ScriptStats {
    /// Instructions this script has run since it started. The difference between two frames is
    /// what it spent on that frame.
    pub instructions: u64,
    /// Where it stands in its own source: file and line, while it waits as well as while it
    /// runs. `None` where the program carries no debug info.
    pub location: Option<(String, u32)>,
    /// Every frame it stands in, innermost first. A script parked inside a library method (a
    /// DSL, `sleep`) stands in the library; this is how a panel finds the line of the script's
    /// own file that is waiting.
    pub frames: Vec<(String, u32)>,
    /// Whether the task has run to its end.
    pub finished: bool,
}

/// Marks an entity whose script has ended, so that [`ScriptEnded`] is sent once and the task
/// is let go of once. The [`ScriptTask`] stays, which is what keeps the script from starting
/// again — and where there is no task because the script never started (its `.mrb` would not
/// load), this marker alone keeps it from being started over and over: the system that turns a
/// [`Script`] into a task passes over an entity that carries it.
///
/// It carries the name tag of its VM for the same reason [`ScriptTask`] does. One entity may
/// hold a `Script<A>` and a `Script<B>` at once — two scripts of two VMs on one thing — and an
/// untagged marker would let either of them ending say that *both* had ended: `tick_scripts`
/// would stop looking at the other's task, and `stop_removed_task` would believe its object had
/// been let go of when it had not.
#[derive(Component)]
pub struct ScriptDone<M = ()> {
    _m: PhantomData<fn() -> M>,
}

impl ScriptDone<()> {
    /// The marker of the app's first VM. It was a unit struct before the VMs had name tags, so
    /// a host that used to write `ScriptDone` writes `ScriptDone::new()`.
    pub fn new() -> Self {
        ScriptDone::for_vm()
    }
}

impl Default for ScriptDone<()> {
    fn default() -> Self {
        ScriptDone::new()
    }
}

impl<M: 'static> ScriptDone<M> {
    /// The marker of the VM named `M`: `ScriptDone::<Mods>::for_vm()`.
    pub fn for_vm() -> Self {
        ScriptDone { _m: PhantomData }
    }
}

// as on `Script`: derived, these would ask the name tag to be `Debug` / `Clone` / `Copy` itself
impl<M> std::fmt::Debug for ScriptDone<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ScriptDone")
    }
}

impl<M> Clone for ScriptDone<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Copy for ScriptDone<M> {}

/// Marks an entity whose script could not be **started** — the `.mrb` would not load, or the VM
/// would not spawn the task — as against one that ran and ended. Both carry [`ScriptDone`]; only
/// this one is worth trying again when the asset changes ([`retry_failed_scripts`]), which is
/// the whole reason the two are told apart.
///
/// It is not part of the crate's API. What a game sees of a script that could not start is the
/// [`ScriptEnded`] it was sent, the same as for one that ran and raised.
#[derive(Component)]
struct ScriptStartFailed<M = ()> {
    _m: PhantomData<fn() -> M>,
}

/// **Gives an entity another script**: the running one is stopped and `script` takes its place.
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use rubevy::{replace_script, MrbAsset, Script};
/// fn reload(mut commands: Commands, entity: Entity, compiled: Handle<MrbAsset>) {
///     replace_script(&mut commands, entity, Script::new(compiled).with_name("brain.rb"));
/// }
/// ```
///
/// It is the three lines both sample games wrote by hand whenever an editor applied a change or a
/// file was reloaded: take the [`ScriptTask`] off (whose removal hook terminates the task in the
/// VM and closes what it had subscribed to), take the [`ScriptDone`] marker off if the old script
/// had run to its end, and insert the new [`Script`], which the plugin turns into a task on the
/// next frame. Leaving any of the three out is a bug that does not look like one: without the
/// removal the old task goes on running, still carrying the entity, so the thing has two scripts
/// driving it and only one of them is visible.
///
/// Which VM this is comes from the `script`, so `replace_script(&mut commands, e,
/// Script::<Mods>::for_vm(h))` replaces a script of the VM named `Mods` and leaves the first VM's
/// script on the same entity alone.
///
/// A question the old script had already asked may still be answered once — the [`Request`] was
/// taken before the swap — and no new one is asked (`tests/replace.rs`). To **stop** a script
/// instead of replacing it, remove its [`ScriptTask`] or despawn the entity; there is nothing
/// else to do.
pub fn replace_script<M: 'static>(commands: &mut Commands, entity: Entity, script: Script<M>) {
    commands
        .entity(entity)
        .remove::<ScriptTask<M>>()
        .remove::<ScriptDone<M>>()
        .insert(script);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptStatus {
    Finished,
    Failed,
}

/// Emitted when a script's task runs to its end, with the value it answered
/// (or the exception it did not handle, which mruby-task makes the result).
///
/// **A script that never started ends this way too**: a `.mrb` that will not load is a
/// [`ScriptStatus::Failed`] whose value is what the VM said was wrong with it, sent once for
/// that entity. It is the same ending because it is the same news — this entity has no script
/// running — and a game that already shows an ending shows this one without being changed.
///
/// One message type per VM (`ScriptEnded<Mods>`), so a reader of the first VM's endings is not
/// woken by the second's.
#[derive(Message)]
pub struct ScriptEnded<M = ()> {
    pub entity: Entity,
    pub status: ScriptStatus,
    pub value: String,
    _m: PhantomData<fn() -> M>,
}

impl<M> std::fmt::Debug for ScriptEnded<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptEnded")
            .field("entity", &self.entity)
            .field("status", &self.status)
            .field("value", &self.value)
            .finish()
    }
}

impl<M> Clone for ScriptEnded<M> {
    fn clone(&self) -> Self {
        ScriptEnded {
            entity: self.entity,
            status: self.status,
            value: self.value.clone(),
            _m: PhantomData,
        }
    }
}

/// What a script asked the host to do. A native cannot touch the Bevy world,
/// so it leaves one of these behind and [`drain_commands`] carries it out.
#[derive(Debug, Clone)]
enum HostCommand {
    Log(String),
    Spawn { name: String, x: f32, y: f32, z: f32 },
    Despawn(u64),
    SetPosition { entity: u64, x: f32, y: f32, z: f32 },
    /// `Rubevy.ask`: the script is parked on a queue until the game answers it.
    Ask { entity: u64, kind: String, args: Vec<Arg>, queue: ObjId },
    /// `entity[:Transform] = hash`, through `Rubevy.set_component`: the value was read out of
    /// the VM by the native (a `&mut World` is a system away), and [`apply_component_writes`]
    /// writes it over the component through `ReflectComponent`.
    SetComponent { entity: u64, name: String, value: RubyData },
    /// `Rubevy.set_resource(:Score, hash)`: the same thing for a resource, which belongs to no
    /// entity, so there is nothing to name but the type. [`apply_resource_writes`] makes it.
    SetResource { name: String, value: RubyData },
}

/// A component write waiting for the exclusive system that can make it
/// ([`apply_component_writes`]).
#[derive(Debug, Clone)]
struct ComponentWrite {
    entity: Entity,
    name: String,
    value: RubyData,
}

/// A resource write waiting for [`apply_resource_writes`]. The twin of [`ComponentWrite`]
/// without the entity: Bevy keeps a resource on an entity of its own and nothing outside the
/// ECS names it.
#[derive(Debug, Clone)]
struct ResourceWrite {
    name: String,
    value: RubyData,
}

/// Where a [`RootedValue`] that has been dropped leaves its object until the next sweep
/// ([`ScriptWorld::release_dropped_values`]).
///
/// A `Drop` is not given the `Vm` — it is given nothing at all — so letting go of a value is
/// two steps: dropping the last handle puts the object here, and the next sweep, which has the
/// `&mut Vm`, calls `Vm::gc_unregister` on it. In between the object is still registered, which
/// is the safe side of the mistake: it lives a little longer than it had to, and never a moment
/// less.
type ReleaseQueue = std::sync::Arc<std::sync::Mutex<Vec<ObjId>>>;

/// A Ruby value that an [`Arg`] carries, kept alive for as long as anything holds it.
///
/// A Hash or an Array a script passed to `Rubevy.ask` is *not* copied out of the VM the way a
/// component write is ([`Arg::Value`] explains why): the host is handed the value itself, so
/// it has to be told not to collect it. `Vm::gc_register` is that telling, and the pair to it
/// is what leaks if a game forgets it — so nothing here asks a game to remember. The
/// registration is made once, when the argument is read in the `Rubevy.ask` native, and is
/// owned by an `Arc` that every clone of the [`Request`] shares. When the last of them goes —
/// answered, dropped on the floor, lost with a system that panicked — the `Drop` puts the
/// object on a queue and the next frame's sweep unregisters it.
///
/// So a [`Request`] can stay `Clone` and be kept for as many frames as a game likes, and the
/// only way to keep a value alive for ever is to keep a `Request` alive for ever.
///
/// Read the value with the `Vm` it belongs to — `vm.ary_vals(v.value())`,
/// `vm.hash_entries(v.value())`, or [`Request::arg_as`].
#[derive(Debug, Clone)]
pub struct RootedValue {
    value: Value,
    /// `None` for an immediate (`nil`, `true`, `false`): there is no heap object to register.
    ///
    /// Nothing reads this, which is the whole of its job: the registration lasts exactly as
    /// long as the last clone of this field, and ends in its `Drop`.
    #[allow(dead_code)]
    root: Option<std::sync::Arc<Root>>,
}

/// The one registration behind a [`RootedValue`] and all its clones.
#[derive(Debug)]
struct Root {
    id: ObjId,
    release: ReleaseQueue,
}

impl Drop for Root {
    fn drop(&mut self) {
        // `lock` only fails where another thread panicked holding it; the queue is still a
        // queue, and dropping the id here instead would be the leak this whole type is against
        let mut queue = match self.release.lock() {
            Ok(q) => q,
            Err(poisoned) => poisoned.into_inner(),
        };
        queue.push(self.id);
    }
}

impl RootedValue {
    /// Registers `v` with the collector, where it is a heap object, and takes charge of letting
    /// it go again.
    fn new(vm: &mut Vm, v: Value, release: &ReleaseQueue) -> RootedValue {
        let root = match v {
            Value::Obj(id) => {
                vm.gc_register(id);
                Some(std::sync::Arc::new(Root { id, release: release.clone() }))
            }
            _ => None,
        };
        RootedValue { value: v, root }
    }

    /// The value, to read through the `Vm` it came from.
    pub fn value(&self) -> Value {
        self.value
    }
}

/// Two of these are equal when they name the same object — identity, not `==` in Ruby, which
/// would need the `Vm` this does not hold.
impl PartialEq for RootedValue {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

/// An argument of `Rubevy.ask`, after the name of what is being asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Num(f64),
    Text(String),
    /// A `Rubevy::Entity` the script passed on: the object `Rubevy.entity` and
    /// [`Answer::Entity`] hand out, read back through its handle rather than through a number
    /// that has been past a `f64`.
    Entity(Entity),
    /// Anything else: a Hash, an Array, an object of the script's own, `nil`, `true`, `false`.
    ///
    /// The three above are the fast paths — a number, a string and an entity are copied out of
    /// the VM at the call and cost the host nothing afterwards — and this is what everything
    /// else falls to. It is the value itself, kept alive by [`RootedValue`], not a copy: a
    /// nested structure has no flat shape to be copied into, and building one would be a second
    /// and poorer `Answer` (the same argument [`ScriptWorld::answer_value`] makes on the way
    /// back).
    ///
    /// **Nothing inside is flattened.** `Rubevy.ask("path", [1.0, 2.0], {speed: 3})` is two
    /// `Arg::Value`s, and the numbers inside the Array stay Ruby numbers inside a Ruby Array —
    /// `request.num(0)` is `None`, not `1.0`. An argument is one `Arg`, and which one it is
    /// depends on that argument alone, not on what is inside it. Read the contents with
    /// [`Request::arg_as`] or through the `Vm`.
    Value(RootedValue),
}

impl Arg {
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Arg::Num(n) => Some(*n),
            Arg::Text(_) | Arg::Entity(_) | Arg::Value(_) => None,
        }
    }
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Arg::Text(t) => Some(t),
            Arg::Num(_) | Arg::Entity(_) | Arg::Value(_) => None,
        }
    }
    /// The entity, where the script passed a `Rubevy::Entity` object.
    pub fn as_entity(&self) -> Option<Entity> {
        match self {
            Arg::Entity(e) => Some(*e),
            Arg::Num(_) | Arg::Text(_) | Arg::Value(_) => None,
        }
    }
    /// The Ruby value, where the argument was one of the sorts [`Arg::Value`] carries.
    pub fn as_value(&self) -> Option<Value> {
        match self {
            Arg::Value(v) => Some(v.value()),
            Arg::Num(_) | Arg::Text(_) | Arg::Entity(_) => None,
        }
    }
}

/// Something a script asked the game for and is waiting on (`Rubevy.ask`). Take them with
/// [`ScriptWorld::take_requests`] in a system of your own, work out the answer, and give it back
/// with [`ScriptWorld::answer`] — this frame or any later one. The script's task is parked
/// meanwhile, so it costs nothing and the other scripts keep running.
#[derive(Debug, Clone)]
pub struct Request {
    /// The entity whose script asked, where it has one.
    pub entity: Option<Entity>,
    /// The first argument of `Rubevy.ask`, e.g. `"scan"`.
    pub kind: String,
    /// The rest of the arguments: numbers and strings, in the order they were written.
    pub args: Vec<Arg>,
    /// Hand this back to [`ScriptWorld::answer`]; it is the queue the script waits on.
    pub queue: ObjId,
}

/// What a game answers a [`Request`] with.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Nil,
    Bool(bool),
    Num(f64),
    Text(String),
    /// A list of numbers, e.g. a position or what a sensor found.
    List(Vec<f64>),
    /// A list of rows of numbers: a table, e.g. every robot with its team and hp.
    Rows(Vec<Vec<f64>>),
    /// An entity, as the `Rubevy::Entity` object a script can pass back to `Rubevy.despawn`,
    /// `Rubevy.set_position` or `Rubevy.ask`. Inside [`Answer::List`] and [`Answer::Rows`] an
    /// entity is still a number (`Entity::to_bits` through a `f64`), which is what a table of
    /// them wants; this is the way to hand one over without that.
    Entity(Entity),
}

/// What answers one kind of `Rubevy.ask` **inside the tick**, registered with
/// [`ScriptWorld::answer_in_tick`].
///
/// It is handed the world as it stands in [`RubevySet::Tick`] and the [`Request`], and answers
/// with the flat [`Answer`] — which is all it can answer with, because the `Vm` is being run
/// around it and is not the closure's to touch (`ScriptWorld::answer_value`'s way of building a
/// value has no counterpart here).
pub type InTickAnswerer = Box<dyn Fn(&World, &Request) -> Answer + Send + Sync + 'static>;

impl Request {
    /// The `i`th argument as a number, where it is one.
    pub fn num(&self, i: usize) -> Option<f64> {
        self.args.get(i).and_then(Arg::as_num)
    }
    /// The `i`th argument as a string, where it is one.
    pub fn text(&self, i: usize) -> Option<&str> {
        self.args.get(i).and_then(Arg::as_text)
    }
    /// The `i`th argument as a number, or `or` where there is none.
    pub fn num_or(&self, i: usize, or: f64) -> f64 {
        self.num(i).unwrap_or(or)
    }
    /// The `i`th argument as an entity, where the script passed a `Rubevy::Entity`.
    pub fn entity_arg(&self, i: usize) -> Option<Entity> {
        self.args.get(i).and_then(Arg::as_entity)
    }
    /// The `i`th argument as the Ruby value it is, where it is one of the sorts
    /// [`Arg::Value`] carries (a Hash, an Array, an object, `nil`, `true`, `false`).
    ///
    /// The value is alive for as long as this `Request` is — it is registered with the
    /// collector — so a game may keep the request for a frame or ten and read it when the
    /// answer is ready. Read it through the `Vm` in [`ScriptWorld::vm`]:
    ///
    /// ```no_run
    /// # use rubevy::{Request, ScriptWorld};
    /// fn speeds(world: &mut ScriptWorld, request: &Request) -> Vec<(String, f64)> {
    ///     let Some(h) = request.value(0) else { return Vec::new() };
    ///     // a Hash entry by entry, keys and values as they are
    ///     let entries = world.vm.hash_entries(h).unwrap_or_default();
    ///     entries
    ///         .into_iter()
    ///         .filter_map(|(k, v)| {
    ///             let k = String::from_utf8_lossy(&world.vm.as_string(k).ok()?).into_owned();
    ///             Some((k, f64::from_ruby(&mut world.vm, v).ok()?))
    ///         })
    ///         .collect()
    /// }
    /// # use sabiruby::convert::FromRuby;
    /// ```
    pub fn value(&self, i: usize) -> Option<Value> {
        self.args.get(i).and_then(Arg::as_value)
    }
    /// The `i`th argument converted with sabiruby's [`FromRuby`], which is how a host system
    /// takes a `Vec<f64>`, a `Vec<String>`, a `Vec<Vec<f64>>` or anything else that trait
    /// knows:
    ///
    /// ```no_run
    /// # use rubevy::{Request, ScriptWorld};
    /// # fn f(world: &mut ScriptWorld, request: &Request) {
    /// let waypoints: Vec<f64> = request.arg_as(&mut world.vm, 0).unwrap_or_default();
    /// # }
    /// ```
    ///
    /// `None` where there is no such argument, where it is not one [`Arg::Value`] carries (a
    /// number, a string and an entity are already out of the VM — use [`Request::num`],
    /// [`Request::text`] and [`Request::entity_arg`] for those), or where the value would not
    /// convert. That last one is deliberately flattened into "no", the way the neighbours
    /// answer `None` for an argument of another sort; a host that wants to see the `TypeError`
    /// calls `T::from_ruby` on [`Request::value`] itself.
    ///
    /// A Hash has no `FromRuby` in the VM, so it is read with `Vm::hash_entries` — the example
    /// on [`Request::value`].
    pub fn arg_as<T: FromRuby>(&self, vm: &mut Vm, i: usize) -> Option<T> {
        let v = self.value(i)?;
        T::from_ruby(vm, v).ok()
    }
}

/// Marks and names an entity a script spawned (`Rubevy.spawn`).
#[derive(Component, Debug, Clone)]
pub struct SpawnedByScript {
    pub name: String,
}

/// The VM the scripts of one name tag share, and the queue between them and the world.
///
/// `ScriptWorld` — that is, `ScriptWorld<()>` — is the app's first VM, and is what an app with
/// one VM writes. `ScriptWorld<Mods>` is the VM `RubevyPlugin::<Mods>` started: another
/// resource, so another heap, another set of globals, constants and classes, another scheduler
/// and another budget.
#[derive(Resource)]
pub struct ScriptWorld<M = ()> {
    /// The VM itself, for a host that wants to add to it — **at `Startup`**, before any script
    /// has run.
    ///
    /// The resource exists as soon as [`RubevyPlugin`] is added, and the first script does not
    /// start until the first `Update`, so a `Startup` system is the place to install a class,
    /// a module or a native of the game's own:
    ///
    /// ```no_run
    /// # use bevy::prelude::*;
    /// # use rubevy::ScriptWorld;
    /// # use sabiruby::{Value, Vm};
    /// fn install_host_api(mut world: ResMut<ScriptWorld>) {
    ///     let vm = &mut world.vm;
    ///     sabiruby_serde::install_json(vm);          // `JSON.parse` / `JSON.generate`
    ///     let object = vm.core.object;
    ///     vm.define_fn(object, "arena_size", |_vm: &mut Vm| -> f64 { 240.0 });
    /// }
    /// # fn build(app: &mut App) {
    /// app.add_systems(Startup, install_host_api);
    /// # }
    /// ```
    ///
    /// **What not to do with it.** It is the VM the scheduler is running, not a VM of your own.
    ///
    /// * Do not run tasks through it — no `task_run_limits`, `task_run_once` or `Task.run`.
    ///   `tick_scripts` is what gives the scheduler its frame, and a second run inside a system
    ///   would spend a budget nobody set and resume scripts in the middle of somebody else's
    ///   frame.
    ///   `Vm::load_and_run` at `Startup` is fine — that is running a program, not the scheduler.
    /// * Do not keep anything from it. A `Value` or an `ObjId` held past the call is a reference
    ///   the collector does not know about: register it (`Vm::gc_register`) or let it go before
    ///   the system returns. [`Request`] and [`ScriptTask`] are the two things rubevy keeps this
    ///   way, and both are registered.
    /// * Do not touch the Bevy world from a native. A native is handed `&mut Vm` and nothing
    ///   else; what it can do is leave something behind for a system to pick up, which is what
    ///   `Rubevy.ask` and [`ScriptWorld::take_requests`] already are.
    ///
    /// Adding to the VM *after* `Startup` is not forbidden and is sometimes what a game means (a
    /// class that only exists once a level is loaded); what it costs is that a script which had
    /// already run may have seen the VM without it.
    pub vm: Vm,
    /// Instructions the scheduler may spend per frame, over all tasks.
    ///
    /// **Zero pauses the scripts.** The VM checks the budget at the head of its own loop, so a
    /// budget of zero runs not one instruction; and a frame in which no script can run is not
    /// a frame the scripts age by, so the plugin does not move mruby-task's clock on either
    /// (`task_advance_ticks`). Without that, every `sleep` in the VM came due while nothing was
    /// running, and giving the budget back woke the lot of them at once — a hundred paused
    /// frames and a script that asked for `sleep 0.1` was two seconds late, all in one frame.
    /// Now a pause is time the scripts did not live through: a script that had 0.07 s of its
    /// `sleep` left when the pause began has 0.07 s left when it ends.
    ///
    /// It is spelled as a budget of zero rather than as a `pause` flag on purpose. A flag would
    /// be a second way to say the same thing, and two of them can disagree (paused with a budget,
    /// running with none); the games that pause already write `world.budget = 0`. What the game
    /// keeps is its own old budget, to put back.
    ///
    /// The rest of the frame is untouched while paused: `$rubevy` is still refreshed, so a HUD or
    /// a debugger panel reading the VM sees a live frame count, and the host's questions are
    /// still taken and answered — they simply reach a script that is not running. Bevy's own
    /// `Time` is the game's to pause (`Time<Virtual>`); this is about the VM's scheduler.
    pub budget: u64,
    /// Time the scripts may take per frame, on the VM's clock (Bevy's `Instant`). `None`:
    /// instructions only, and the clock is not read at all.
    ///
    /// It bounds the tick and not only the runs of the VM inside it. A tick is a loop — run the
    /// ready tasks, answer what they parked on, run them again — and the clock ends both halves
    /// of it: the running timeslice is cut short when the time is up (`Vm::task_run_limits`),
    /// and the answering stops when the time is up, with the questions it has not reached put
    /// off to the next frame, where they are taken before anything asked in it.
    ///
    /// **What it does not cover**, which is another way of saying by how much a tick can still
    /// pass it:
    ///
    /// * **One answer.** A late tick still makes one answer of rubevy's own kinds and one of the
    ///   game's [`ScriptWorld::answer_in_tick`] kinds, because the run of the VM before them may
    ///   have used the whole of `frame_time` by itself, and a frame that answered nobody would
    ///   leave every task parked on its question for ever. So the overshoot is the cost of the
    ///   dearest single answer: a component or resource read is a couple of microseconds
    ///   (`tests/read_cost.rs` says what it is on your machine), while `Rubevy.find` walks every
    ///   entity in the world and an `answer_in_tick` closure costs whatever the game wrote.
    /// * **The VM's own notice of the deadline.** `Vm::task_run_limits` is handed what is left
    ///   of the frame and comes back when it is gone, but not to the nanosecond: with three
    ///   thousand tasks waiting, a tick whose scripts ask nothing at all still took about 2.9 ms
    ///   under a `frame_time` of 1 ms (measured 2026-09-20, one machine — the worklog named
    ///   below has the run). That is inside the VM's scheduler, where every push and every wake
    ///   walks the waiting tasks, and rubevy cannot shorten it from the outside. At a few
    ///   hundred tasks it is not there.
    /// * **A native that cannot be switched out.** A script inside `sort { }` or a long native
    ///   is not interrupted by this clock; that is what [`ScriptWorld::overrun`] is for, and it
    ///   is the larger number of the two on purpose.
    /// * **A second VM.** Each [`ScriptWorld<M>`] has its own, and nothing adds them together:
    ///   the worst case for a frame is the sum of the VMs' `frame_time`s.
    /// * **Everything in the frame that is not the tick.** Starting the scripts of new entities
    ///   (`Vm::load` and one `task_spawn` an entity), carrying out their commands, the writes at
    ///   the end of the frame, and what the game publishes to them all happen in systems of
    ///   their own.
    ///
    /// **What a script sees when the time runs out.** Its question is answered on the next
    /// frame instead of in the line that asked — the one exception to "a read costs no frame",
    /// and only for the questions left at the tail of a frame that ran out. Nothing is lost,
    /// nothing is asked twice, and a question that is put off is taken before anything the next
    /// frame asks, so it is not put off again. `docs/host-api.md` ("Components by name") is the
    /// long version and `docs/worklog/2026-09-20-frame-time-as-a-limit.md` the measurements.
    pub frame_time: Option<std::time::Duration>,
    /// Past this, a script that cannot be switched out — it is inside a native waiting for a
    /// block, `sort { }` or `Array.new { loop { } }` — gets `Task::Overrun` rather than holding
    /// the frame. `None`: no such limit.
    pub overrun: Option<std::time::Duration>,
    /// How many messages a subscriber's queue holds in this VM before the oldest is dropped —
    /// the default for every `Rubevy.subscribe` that does not ask for one of its own
    /// (`Rubevy.subscribe(:belt, limit: 512)`).
    ///
    /// It is read where a message is published, not where a script subscribes, so changing it
    /// moves every standing subscription that did not name its own — and there is one number,
    /// not a copy of it per subscriber to keep in step. A limit of its own is the subscription's
    /// for as long as it stands.
    ///
    /// **Where the default comes from.** 64, which is [`ScriptWorld::QUEUE_LIMIT`] and what this
    /// was before it could be changed at all. It is **not a measured number**: the record that
    /// chose it (`docs/worklog/2026-09-15-events.md`) gives a qualitative reason only — "enough
    /// for a script that reads its queue every few frames, and small enough that a script that
    /// never reads costs nothing to speak of". What one reader can actually take in a frame, and
    /// what a waiting message costs in memory, were measured on 2026-09-20 and are in
    /// `docs/worklog/2026-09-20-overflow-and-limits.md` beside the sum that a default could be
    /// derived from; whether the default should move from 64 is the app's call, which is what
    /// this field is for.
    ///
    /// Zero is not refused here, and what it means is "the newest and nothing else": room is
    /// made before the message is pushed, so the queue is emptied and the one that arrived is
    /// left. A script that asks for a limit of its own is refused anything below one
    /// (`ArgumentError`), because a script writing `limit: 0` has miscounted rather than asked
    /// for that.
    pub queue_limit: usize,
    /// Ticks not yet handed to the scheduler (frame times shorter than a tick).
    tick_remainder: f32,
    /// What scripts asked the game for and are waiting on (`Rubevy.ask`).
    requests: Vec<Request>,
    /// The questions rubevy answers itself ([`RESERVED_KINDS`]) rather than handing to the
    /// game: a component by name, and the entities that have one. They are kept apart from
    /// [`ScriptWorld::requests`] at the moment they are made, so a game's own answering system
    /// never sees a kind it does not know — and cannot answer one of these by mistake.
    ///
    /// Most of them never reach this vector at all: [`tick_scripts`] takes them off the VM's
    /// command queue and answers them while the frame is still running
    /// ([`answer_reflect_requests`]). What lands here is the leftovers of a frame that ran out
    /// of budget or of time, by either of the two roads a frame runs out on: the questions still
    /// on the VM's queue when the loop ended, which [`drain_commands`] sorts out as it always
    /// did, and the ones [`answer_reflect_requests`] had taken but did not reach before
    /// [`ScriptWorld::frame_time`] was up. Both keep the order they were asked in, and the next
    /// frame's tick answers them before anything that frame asks.
    reflect_requests: Vec<Request>,
    /// The kinds the game answers inside the tick ([`ScriptWorld::answer_in_tick`]), by kind.
    ///
    /// They are the game's side of the same loop the component reads are answered in: a kind in
    /// here is taken off the VM's command queue by [`answer_in_tick_requests`] between two runs
    /// of the VM, never becomes a [`Request`] in [`ScriptWorld::requests`], and so is never seen
    /// by [`ScriptWorld::take_requests`].
    in_tick_answerers: std::collections::HashMap<String, InTickAnswerer>,
    /// Questions for [`ScriptWorld::in_tick_answerers`] left over from a frame that ran out of
    /// budget or of time — on the VM's queue when the loop ended, or taken and not reached
    /// before the frame time was up — the way [`ScriptWorld::reflect_requests`] is for rubevy's
    /// own kinds. The next frame's answer loop takes them first.
    in_tick_requests: Vec<Request>,
    /// The name a script wrote (`"Transform"`, `"my_game::Hp"`) to the `ReflectComponent` it
    /// stands for, kept from one read to the next.
    ///
    /// Bevy's own advice, in the rustdoc of `ReflectComponent` itself: looking a type up in the
    /// registry and then asking the registration for its `ReflectComponent` "can be costly if
    /// done several times per frame", and a `ReflectComponent` is cheap to clone and worth
    /// keeping between frames. Now that a script may read a component several times in one
    /// frame — which is the whole point of the synchronous read — that is exactly this map.
    /// A name nothing is registered under is *not* remembered: a type may be registered later
    /// (a plugin added with a level), and the miss costs one hash.
    reflect_cache: std::collections::HashMap<String, bevy::ecs::reflect::ReflectComponent>,
    /// The name a script wrote (`"Score"`, `"Time<Virtual>"`) to what a resource of that type
    /// takes to reach: its `TypeId`, which is how the world is asked where it keeps that
    /// resource, and the `ReflectComponent` that reads it once the entity is known.
    ///
    /// Kept for the same reason [`ScriptWorld::reflect_cache`] is, and the two are apart because
    /// a name only lands here once the registration has said `#[reflect(Resource)]`
    /// ([`reflect_resource_of`]).
    resource_cache:
        std::collections::HashMap<String, (std::any::TypeId, bevy::ecs::reflect::ReflectComponent)>,
    /// Component writes waiting for [`apply_component_writes`], which is the system that has a
    /// `&mut World` to make them with.
    component_writes: Vec<ComponentWrite>,
    /// Resource writes waiting for [`apply_resource_writes`], the same way.
    resource_writes: Vec<ResourceWrite>,
    /// Answers still being worked out on Bevy's task pool ([`ScriptWorld::answer_with`]), each
    /// with the request it belongs to. [`deliver_answers`] takes them off as they finish.
    answering: Vec<(Request, Task<Answer>)>,
    /// The `Rubevy::Entity` class, for [`ScriptWorld::answer`] to build an [`Answer::Entity`]
    /// with. The natives carry it in their closures.
    entity_class: ObjId,
    /// How many entity objects the collector has taken, counted by the free hook.
    freed_entities: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// The objects of dropped [`Arg::Value`]s, waiting for a sweep with the `Vm`
    /// ([`ScriptWorld::release_dropped_values`]). The `Rubevy.ask` native holds the other end.
    release: ReleaseQueue,
    /// Every program this VM has loaded, by **the bytes of the program itself**, to the irep
    /// `Vm::load` made of it ([`ScriptWorld::irep_of`]).
    ///
    /// `start_scripts` used to hand `Vm::load` the bytes of every [`Script`] it turned into a
    /// task, so a thousand entities running one `.mrb` put a thousand copies of the same
    /// instructions in the VM. `Vm::task_spawn` takes an irep, and an irep is read-only while
    /// it runs, so one copy is enough for all of them.
    ///
    /// **Why the bytes and not the `AssetId`.** A game that compiles a player's Ruby itself adds
    /// a *new* asset every time — `Assets::add` hands out a fresh id — and both sample games do
    /// it once per script *and once per creature*: garden compiles the same species' file again
    /// for every animal born into it (`garden/src/main.rs`, `give_mind`). Keyed by asset id,
    /// those thousand creatures would still be a thousand ireps. Keyed by what the program *is*,
    /// they are one.
    ///
    /// The key is the whole program and not a hash of it, so two different programs can never
    /// meet in it: a hash collision in the map is settled by comparing the bytes, as it is in
    /// any `HashMap`, and the worst a collision can cost is the comparison. The price is one
    /// copy of each distinct program's bytes — **once**, where the irep it saves cost that much
    /// again for every entity (measured 2026-09-20 with three thousand entities of a 294-byte
    /// `.mrb`: 2.21 kB of resident memory an entity when each loaded its own, 1.46 kB when they
    /// shared one).
    ///
    /// Nothing is ever taken out of it, and nothing needs to be: a program that changed is other
    /// bytes, so it misses and is loaded. What that leaves behind is the irep of the version
    /// before it, and **SabiRuby has no way to drop an irep** (0.5.2: nothing in the crate ever
    /// removes from `Vm::ireps`), so the VM would keep it whether this map did or not. Keeping it
    /// is what makes an editor that applies the same text twice — or applies a change and takes
    /// it back — cost nothing the second time.
    programs: std::collections::HashMap<Box<[u8]>, sabiruby::object::IrepId>,
    /// Every program this VM could **not** load, by the same key [`ScriptWorld::programs`] uses,
    /// to what the VM said was wrong with it.
    ///
    /// A `.mrb` that will not load is not a passing condition: it is a truncated download, a file
    /// written by another version's compiler, a game that saved bytes that were never bytecode.
    /// `start_scripts` used to answer one by logging and moving on, which meant it met the same
    /// broken program again on the next frame, and the next — a parse and a line in the log per
    /// entity per frame, for as long as the entity existed. Now the first meeting is the only
    /// one that parses and the only one that speaks; what every later entity of the same program
    /// gets is the message out of here.
    ///
    /// **There is no limit on it and it needs none**, for the reason [`ScriptWorld::programs`]
    /// has none: the key is the program itself, so this can only grow when a game makes a
    /// *distinct* program that will not load, and it then holds one copy of those bytes — less
    /// than the `MrbAsset` they came from costs, and less than what a program that *does* load
    /// costs (an irep the VM never gives back). A cap would be a number with nothing behind it,
    /// and whatever fell out of the cap would go back to being parsed once a frame for ever.
    /// [`ScriptWorld::broken_programs`] is how a game watches it.
    broken: std::collections::HashMap<Box<[u8]>, String>,
    /// Which VM this is, as a type. Nothing reads it; what it does is keep the resources of two
    /// VMs apart, and with them their tasks, requests and components.
    _m: PhantomData<fn() -> M>,
}

impl<M: 'static> ScriptWorld<M> {
    fn new() -> Result<ScriptWorld<M>, String> {
        let mut vm = Vm::with_mrblib().map_err(|e| format!("mrblib: {e:?}"))?;
        // the clock is Bevy's, not the instruction count; what the instruction count still does
        // is end a timeslice, which is what keeps one script from eating a frame
        vm.task_external_clock(true);
        // the scheduler collects at its idle points instead of the allocation path, so a script
        // that allocates never pauses a frame for the collector (`docs/gc.md` of the VM)
        if let Err(e) = enable_scheduler_gc(&mut vm) {
            return Err(format!("GC.scheduler_driven: {e}"));
        }
        // the natives' side of the bridge lives in the VM, not in a static, so two `App`s in
        // one process each have their own (`Vm::set_host_state`)
        vm.set_host_state(HostState::default());
        let release: ReleaseQueue = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let entity_class = install_host_api(&mut vm, release.clone());
        // the Ruby half of the host API (`src/prelude.rb`), which is Ruby because each method
        // in it waits for an answer and a native cannot be parked
        if let Err(e) = vm.load_and_run(PRELUDE) {
            let message = vm.describe_error(&e);
            return Err(format!("prelude: {message}"));
        }
        // An entity object owns nothing on the host's side — its handle *is* `Entity::to_bits`,
        // and the ECS is what says whether that entity still exists — so there is nothing to
        // release here. The hook is registered all the same: it is the place a kind of Data that
        // does own something (a handle into a slab) would give it back, and counting keeps the
        // path exercised (`ScriptWorld::freed_entities`).
        let freed_entities = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let counter = freed_entities.clone();
        vm.set_on_free(Box::new(move |tag, _handle| {
            if tag == ENTITY_TAG {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }));
        // a clock for the time limits (`frame_time`, `overrun`). Timeslices stay counted in
        // instructions, so what a script does is the same on every machine; the clock only keeps a
        // frame from being lost to one that the count cannot stop
        vm.task_set_clock(Some(clock_ns));
        Ok(ScriptWorld {
            vm,
            budget: 200_000,
            frame_time: Some(std::time::Duration::from_millis(8)),
            overrun: Some(std::time::Duration::from_millis(50)),
            queue_limit: QUEUE_LIMIT,
            tick_remainder: 0.0,
            requests: Vec::new(),
            reflect_requests: Vec::new(),
            in_tick_answerers: std::collections::HashMap::new(),
            in_tick_requests: Vec::new(),
            reflect_cache: std::collections::HashMap::new(),
            resource_cache: std::collections::HashMap::new(),
            component_writes: Vec::new(),
            resource_writes: Vec::new(),
            answering: Vec::new(),
            entity_class,
            freed_entities,
            release,
            programs: std::collections::HashMap::new(),
            broken: std::collections::HashMap::new(),
            _m: PhantomData,
        })
    }

    /// **Gives the VM another [`Host`](sabiruby::Host) and tells it where `require` looks**,
    /// which are one act and not two.
    ///
    /// ```no_run
    /// # use bevy::prelude::*;
    /// # use rubevy::{EmbeddedHost, ScriptWorld};
    /// # static RUBY_FILES: &[(&str, &str)] = &[];
    /// fn embed_the_scripts(mut world: ResMut<ScriptWorld>) {
    ///     world.require_from(EmbeddedHost::new(RUBY_FILES), &["ruby"]);
    /// }
    /// # fn build(app: &mut App) {
    /// app.add_systems(Startup, embed_the_scripts);
    /// # }
    /// ```
    ///
    /// [`RubevyPlugin`] puts a host that reads the asset directory on the VM and
    /// `["{asset_root}/scripts", "{asset_root}"]` on its load path, and a game that replaces the
    /// host has to replace the
    /// load path too: the paths a host can answer are the host's own, and the plugin's are the
    /// asset directory's. `vm.set_host(..)` on its own leaves the VM asking the new host for
    /// `assets/scripts/helper.rb` — a path a table of embedded files does not hold — and the
    /// script gets a `LoadError` naming a file the game can see is right there. Nothing in the
    /// types says so, and because the host that gets replaced is usually the browser's, it is a
    /// thing that happens **in a browser only**, where the log is a console nobody is reading.
    ///
    /// So this is the pair under one name. The two calls it makes are still there
    /// (`world.vm.set_host`, `world.vm.set_load_path`) and still do what they did — a game that
    /// wants only one of them writes that one — and this is the way to write both.
    ///
    /// **Call it at `Startup`**, for the reason everything on [`ScriptWorld::vm`] is called at
    /// `Startup`: a script that has already run has already done its `require`s.
    ///
    /// `load_path` replaces the load path, it is not added to it, and the entries are what the
    /// VM joins with the name a script wrote (`"ruby"` + `"helper"` + `".rb"`). An empty slice
    /// leaves `require` with nowhere to look but the name as written.
    ///
    /// The host is also what `eval` compiles through
    /// ([`Host::compile`](sabiruby::Host::compile)), so this is where a browser's compiler
    /// arrives as well ([`EmbeddedHost::compile_with`]).
    pub fn require_from(
        &mut self,
        // `Send + Sync` are `Host`'s own supertraits, so the bound is `Host` and nothing else
        host: impl sabiruby::Host + 'static,
        load_path: &[&str],
    ) {
        self.vm.set_host(Box::new(host));
        self.vm.set_load_path(load_path);
    }

    /// The irep of the program in `bytes`, loading it the first time this VM sees it.
    ///
    /// This is what keeps a thousand entities running one `.mrb` to one copy of it
    /// ([`ScriptWorld::programs`] says why the bytes are the key). A caller gets the same
    /// `IrepId` back for the same program however many times it asks, and `Vm::task_spawn` may
    /// be given that id once per task: an irep is read-only while it runs — the VM keeps no
    /// inline cache, no counter and no debug state in one (SabiRuby 0.5.2, `VmIrep` is the
    /// instructions, the pool, the symbols, the line table and nothing besides; the one place
    /// the VM writes to an irep is the throwaway one a `Binding` wraps a scope in,
    /// `ext_binding.rs`) — and a string literal is copied out of the pool each time it is
    /// reached, so two tasks of one program cannot reach each other through it.
    ///
    /// **A program that will not load is remembered too** ([`ScriptWorld::broken`]), and the
    /// `Err` is what the VM said about it. `name` is only for the log: a broken program is said
    /// to be broken **once**, here, where it is met for the first time — every entity that comes
    /// to the same bytes afterwards is handed the same message without a parse and without a
    /// second line in the log.
    fn irep_of(&mut self, bytes: &[u8], name: &str) -> Result<sabiruby::object::IrepId, String> {
        if let Some(&irep) = self.programs.get(bytes) {
            return Ok(irep);
        }
        if let Some(message) = self.broken.get(bytes) {
            return Err(message.clone());
        }
        match self.vm.load(bytes) {
            Ok(irep) => {
                self.programs.insert(bytes.into(), irep);
                Ok(irep)
            }
            Err(e) => {
                let message = self.vm.describe_error(&e);
                error!("rubevy: {name} failed to load: {message}");
                self.broken.insert(bytes.into(), message.clone());
                Err(message)
            }
        }
    }

    /// How many distinct programs this VM has loaded for its scripts.
    ///
    /// One per program and not one per script: the scripts of one `.mrb` share its irep. It is
    /// also the number of ireps that the starting of scripts has left in the VM, which is the
    /// number to watch where a game applies a player's edits over and over — the VM has no way
    /// to give an irep back, so each **new** text is one more, for the life of the app.
    pub fn loaded_programs(&self) -> usize {
        self.programs.len()
    }

    /// How many distinct programs this VM has tried to load and could not.
    ///
    /// The pair to [`ScriptWorld::loaded_programs`], and the same kind of number: one per
    /// program, however many entities met it. It is worth watching for the same reason — this
    /// VM keeps the bytes of each of them so that it never parses one twice — and in a game
    /// whose `.mrb` files are files on a disk, it is zero or it is a bug in the build.
    pub fn broken_programs(&self) -> usize {
        self.broken.len()
    }

    /// What a script has spent and where it is, for a HUD or a debugger panel. The task comes
    /// from the entity's [`ScriptTask`].
    pub fn stats(&self, script: &ScriptTask<M>) -> ScriptStats {
        ScriptStats {
            instructions: self.vm.task_instructions(script.task),
            location: self.vm.task_location(script.task),
            frames: self.vm.task_frames(script.task),
            finished: self.vm.task_finished(script.task),
        }
    }

    /// Terminates a task that is still running and, where `release`, lets the collector have it.
    fn stop_task(&mut self, task: ObjId, release: bool) {
        if !self.vm.task_finished(task) {
            let terminate = self.vm.intern("terminate");
            if let Err(e) = self.vm.funcall(Value::Obj(task), terminate, &[], Value::Nil) {
                let message = self.vm.describe_error(&e);
                error!("rubevy: could not stop a script: {message}");
            }
        }
        if release {
            self.vm.gc_unregister(task);
        }
    }

    /// The requests scripts made since the last call (`Rubevy.ask`), for a system of the game to
    /// answer. A request stays valid until it is answered: keep the ones you cannot answer yet
    /// and hand them back to [`ScriptWorld::answer`] on a later frame.
    ///
    /// Two sorts of question are never in here: the four rubevy answers itself
    /// (`RESERVED_KINDS`) and the kinds the game registered with
    /// [`ScriptWorld::answer_in_tick`]. Both are answered inside the tick, and both are sorted
    /// out where the question is made, so a system that answers here sees only what is left for
    /// it.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// Makes `f` the answer to every `Rubevy.ask(kind, …)`, **inside the tick**.
    ///
    /// It is the game's way into the loop that already answers `e[:Transform]` in the line that
    /// asked for it: `tick_scripts` runs the ready tasks, answers what they parked on, and runs
    /// them again. So the script waits no frame at all, which is what a question asked of the
    /// world every frame for every creature needs — "the nearest plant", "what is within 4
    /// metres" — where a `Rubevy.find` and a read per candidate would spend the frame's budget
    /// on the walking.
    ///
    /// ```no_run
    /// # use bevy::prelude::*;
    /// # use rubevy::{Answer, ScriptWorld};
    /// # #[derive(Component)] struct Plant;
    /// fn install_answers(mut scripts: ResMut<ScriptWorld>) {
    ///     scripts.answer_in_tick("nearest", Box::new(|world: &World, request| {
    ///         let Some(from) = request.entity_arg(0).and_then(|e| world.get::<Transform>(e))
    ///         else { return Answer::Nil };
    ///         let mut best: Option<(Entity, f32)> = None;
    ///         for e in world.iter_entities() {
    ///             let (Some(_), Some(t)) = (e.get::<Plant>(), e.get::<Transform>()) else { continue };
    ///             let d = t.translation.distance(from.translation);
    ///             if best.is_none_or(|(_, b)| d < b) { best = Some((e.id(), d)); }
    ///         }
    ///         best.map_or(Answer::Nil, |(e, _)| Answer::Entity(e))
    ///     }));
    /// }
    /// # fn build(app: &mut App) { app.add_systems(Startup, install_answers); }
    /// ```
    ///
    /// **What the closure is handed.** The world as it stands in [`RubevySet::Tick`] — the same
    /// world a component read sees, so a system of the game that must have moved things first
    /// belongs in `.before(RubevySet::Tick)` — and the [`Request`]. Ordinary Bevy reads work on
    /// it (`world.get::<T>`, `world.iter_entities`, a resource of the game's own,
    /// `AppTypeRegistry`); what is **not** in that world is [`ScriptWorld<M>`] itself. The tick
    /// has taken it out for the length of the loop, so `world.get_resource::<ScriptWorld<M>>()`
    /// is `None` inside the closure, and with it the `Vm` — which is why the answer is the flat
    /// [`Answer`] and not a value built in the VM. An [`Arg::Value`] argument can be seen for
    /// what it is ([`Request::value`]) but not read into: reading one needs the `Vm`.
    ///
    /// **Do not change the world in it.** The closure is handed `&World` and not `&mut World`,
    /// so this is the compiler's rule and not a request; what a game means by it — spawning,
    /// despawning, writing a component — is what `Rubevy.spawn` and `e[:Hp] =` already do at the
    /// end of the frame.
    ///
    /// **When not to use it.** An answer that depends on what the *rest* of the frame works out
    /// belongs in a system in [`RubevySet::Answer`] (one frame, and it sees everything);
    /// an answer that is work rather than a lookup belongs in [`ScriptWorld::answer_with`] (a
    /// future, answered on the frame it finishes). This one runs in the middle of the scripts'
    /// own time, so what it costs comes out of their frame.
    ///
    /// A kind registered here never reaches [`ScriptWorld::take_requests`]. Registering the same
    /// kind twice keeps the later closure and warns; the kinds rubevy answers itself
    /// (`RESERVED_KINDS`) cannot be taken over this way, because they are taken off the queue
    /// first.
    pub fn answer_in_tick(&mut self, kind: impl Into<String>, f: InTickAnswerer) {
        let kind = kind.into();
        if self.in_tick_answerers.insert(kind.clone(), f).is_some() {
            warn!("rubevy: {kind} had an in-tick answerer already; the later one answers it now");
        }
    }

    /// Lets the collector have the values of [`Request`]s that have been dropped
    /// ([`Arg::Value`]), and answers how many. **The plugin does this at the head of every
    /// frame, before the scripts run**; a game has nothing to call.
    ///
    /// It is here because the two halves of letting a value go happen in different places: the
    /// `Drop` of the last handle knows *what* to release and has no `Vm`, and this has the `Vm`
    /// and does not know what until it looks. Between the two the value is still registered, so
    /// nothing is ever collected while a request still names it — a value simply outlives its
    /// request by up to one frame. A host driving `ScriptWorld` without [`RubevyPlugin`]'s
    /// systems calls this itself, or the registrations pile up.
    pub fn release_dropped_values(&mut self) -> usize {
        let ids = {
            let mut queue = match self.release.lock() {
                Ok(q) => q,
                Err(poisoned) => poisoned.into_inner(),
            };
            std::mem::take(&mut *queue)
        };
        for id in &ids {
            self.vm.gc_unregister(*id);
        }
        ids.len()
    }

    /// How many `Rubevy::Entity` objects the collector has taken since the VM started, as the
    /// free hook ([`Vm::set_on_free`]) counted them. An entity object owns nothing on the
    /// host's side, so the hook has nothing to release; this is what shows it runs.
    pub fn freed_entities(&self) -> u64 {
        self.freed_entities.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Answers a request: the script's `Rubevy.ask` returns this value and its task becomes
    /// ready again. The queue is let go of here, so answer each request once.
    pub fn answer(&mut self, request: &Request, answer: Answer) {
        let value = answer_value(&mut self.vm, answer, self.entity_class);
        self.push_answer(request, value);
    }

    /// Answers a request with a value built inside the VM.
    ///
    /// [`Answer`] is flat — numbers, a string, a list of numbers, a table of them — because
    /// that is what a game asking about the world usually has. A component read by name is not
    /// flat (`{translation: [x, y, z], rotation: [x, y, z, w]}`), and nesting [`Answer`] to
    /// carry it would be a second, poorer copy of what the VM can already build. So the host is
    /// handed the `Vm` and builds the value with `hash_new` / `hash_set` / `ary_new` /
    /// `str_new`, or with sabiruby's `IntoRuby`:
    ///
    /// ```no_run
    /// # use rubevy::{Request, ScriptWorld};
    /// # use sabiruby::Value;
    /// fn answer_inventory(world: &mut ScriptWorld, request: &Request) {
    ///     world.answer_value(request, |vm| {
    ///         let h = vm.hash_new();
    ///         let k = Value::Sym(vm.intern("gold"));
    ///         let _ = vm.hash_set(h, k, Value::Int(12));
    ///         h
    ///     });
    /// }
    /// ```
    ///
    /// The closure runs at once, inside this call, and the same rule holds as for
    /// [`ScriptWorld::answer`]: answer each request once. Nothing in the closure may park a
    /// task — it is host code, not a script.
    pub fn answer_value(&mut self, request: &Request, build: impl FnOnce(&mut Vm) -> Value) {
        let value = build(&mut self.vm);
        self.push_answer(request, value);
    }

    fn push_answer(&mut self, request: &Request, value: Value) {
        if let Err(e) = self.vm.task_queue_push(request.queue, value) {
            let message = self.vm.describe_error(&e);
            error!("rubevy: could not answer {}: {message}", request.kind);
        }
        self.vm.gc_unregister(request.queue);
    }

    /// Answers a request with what a future works out: the future goes to Bevy's
    /// [`AsyncComputeTaskPool`], and the plugin answers the request on the frame it finishes.
    ///
    /// The script sees nothing of this. It is parked on its queue from the `Rubevy.ask` until
    /// the answer arrives, exactly as it is when a system of the game keeps the request and
    /// answers it three frames later; the other scripts keep running meanwhile.
    ///
    /// ```no_run
    /// # use rubevy::{Answer, ScriptWorld};
    /// # use bevy::prelude::*;
    /// fn answer_paths(mut world: ResMut<ScriptWorld>) {
    ///     for request in world.take_requests() {
    ///         let to = (request.num_or(0, 0.0), request.num_or(1, 0.0));
    ///         world.answer_with(request, async move { Answer::List(vec![to.0, to.1]) });
    ///     }
    /// }
    /// ```
    ///
    /// Take the [`Request`] from [`ScriptWorld::take_requests`] and hand it over as it is; it is
    /// answered once, here, so do not answer it again yourself.
    ///
    /// **What the future runs on.** Bevy's task pools are threads only in a build with bevy's
    /// `multi_threaded` feature. Without it they are a single-threaded fallback on the main
    /// thread: a future that only computes is driven to its end inside this call, which holds
    /// the frame, while a future waiting on something else (a channel, an IO completion, a
    /// waker of your own) still parks and is picked up later. An app that wants the work off
    /// the main thread enables `multi_threaded` on its own `bevy` dependency.
    ///
    /// # Panics
    ///
    /// If Bevy's task pools have not been set up — `TaskPoolPlugin`, which both `MinimalPlugins`
    /// and `DefaultPlugins` add.
    pub fn answer_with(
        &mut self,
        request: Request,
        answer: impl Future<Output = Answer> + Send + 'static,
    ) {
        let task = AsyncComputeTaskPool::get().spawn(answer);
        self.answering.push((request, task));
    }

    /// How many answers are being worked out on the task pool right now
    /// ([`ScriptWorld::answer_with`]), for a HUD or a test.
    pub fn answering(&self) -> usize {
        self.answering.len()
    }

    /// Sends a message to the scripts that asked for it (`Rubevy.subscribe`).
    ///
    /// `entity` is who it is about: `Some(e)` reaches only the scripts on that entity, `None`
    /// every script that subscribed to the name. Nothing is queued for a name nobody
    /// subscribed to, so a game may publish freely.
    ///
    /// A Bevy event reaches Ruby by a game writing the one line that turns it into this — an
    /// observer, or an ordinary system reading its messages:
    ///
    /// ```no_run
    /// # use bevy::prelude::*;
    /// # use rubevy::{Answer, ScriptWorld};
    /// # #[derive(EntityEvent)]
    /// # struct Hit { entity: Entity, damage: f32 }
    /// # fn build(app: &mut App) {
    /// app.add_observer(|on: On<Hit>, mut scripts: ResMut<ScriptWorld>| {
    ///     scripts.publish(Some(on.entity), "hit", Answer::Num(on.damage as f64));
    /// });
    /// # }
    /// ```
    ///
    /// rubevy does not tie Bevy's event types to Ruby by itself: which events a script may see,
    /// and what each one carries, is the game's to say.
    ///
    /// **A queue that nobody reads.** A script parked on something else — or one that is
    /// simply slower than the game — is not made to keep up. Each queue holds
    /// [`ScriptWorld::queue_limit`] messages, or whatever that subscription asked for
    /// (`Rubevy.subscribe(:belt, limit: 512)`); past that the oldest is dropped so that the
    /// newest is there. A script that wakes late gets the last few things that happened, not the
    /// first few. What was dropped is counted rather than lost in silence:
    /// [`ScriptWorld::dropped`] for the VM, `Rubevy::Subscription#dropped` for the one script
    /// whose stream has the hole in it.
    pub fn publish(&mut self, entity: Option<Entity>, name: &str, payload: Answer) {
        let entity_class = self.entity_class;
        self.publish_value(entity, name, move |vm| answer_value(vm, payload.clone(), entity_class));
    }

    /// [`ScriptWorld::publish`] with the message built inside the VM, for a payload that is not
    /// flat — a Hash, a list with an entity in it. The closure runs once per subscriber.
    pub fn publish_value(
        &mut self,
        entity: Option<Entity>,
        name: &str,
        mut build: impl FnMut(&mut Vm) -> Value,
    ) {
        // the VM's default, resolved here and not where the script subscribed, so that an app
        // moving `queue_limit` moves every subscription that did not ask for one of its own
        let default_limit = self.queue_limit;
        // One lookup: what it costs to publish a name is what that name's subscribers cost, and
        // a name nobody subscribed to costs the lookup alone — not a walk of every subscription
        // in the VM, which is what this used to be.
        //
        // The limit rides along as a `u32` so that the pair is eight bytes and the list a
        // publisher builds stays the size it was: a thousand subscribers of one name is a
        // thousand of these on every message, and `usize` would have made each one sixteen bytes
        // for a number that cannot reach four billion (a queue of four billion messages is not
        // one any machine holds, so the saturating cast can never bind in practice).
        let queues: Vec<(ObjId, u32)> = match self.vm.host_state::<HostState>() {
            Some(state) => match state.subscriptions.get(name) {
                Some(subs) => subs
                    .iter()
                    .filter(|s| entity.is_none() || Some(s.entity) == entity)
                    .map(|s| (s.queue, s.limit.unwrap_or(default_limit).min(u32::MAX as usize) as u32))
                    .collect(),
                None => return,
            },
            None => return,
        };
        let mut dropped: u64 = 0;
        for (queue, limit) in queues {
            dropped += self.make_room(queue, limit as usize);
            let value = build(&mut self.vm);
            if let Err(e) = self.vm.task_queue_push(queue, value) {
                let message = self.vm.describe_error(&e);
                error!("rubevy: could not publish {name}: {message}");
            }
        }
        // once for the whole message rather than once per subscriber: reaching the host state is
        // a downcast, and a name a thousand scripts listen for would have paid for it a thousand
        // times over for one number
        if dropped > 0
            && let Some(state) = self.vm.host_state_mut::<HostState>()
        {
            state.dropped = state.dropped.saturating_add(dropped);
        }
    }

    /// Drops the oldest messages until there is room for one more, and counts what it dropped.
    ///
    /// This runs on every published message, so it asks the VM rather than the script's Ruby:
    /// `Vm::task_queue_len` and `Vm::task_queue_try_pop` read the queue's own Array, where
    /// `size` and `__pop_try(true)` each put a call on the stack to do the same thing.
    ///
    /// What it drops is counted twice over, because the two readers are different people: the
    /// VM's own total ([`ScriptWorld::dropped`]) is for the game, which wants to know that its
    /// scripts are falling behind at all; the count on the queue object (`@rubevy_dropped`, read
    /// back by `Rubevy::Subscription#dropped` in `src/prelude.rb`) is for the script, which
    /// wants to know that **its** stream has a hole in it — "I am 40 belt movements behind" is a
    /// thing a script can act on, and before this it could not be seen at all. The counter is on
    /// the queue rather than in the host state because the queue is what the script holds, and
    /// because it goes away exactly when the subscription does.
    ///
    /// It answers with what it dropped so that [`ScriptWorld::publish_value`] can add the VM's
    /// own total once for the whole message instead of once for every subscriber.
    ///
    /// **What the counting costs.** Nothing at all while nothing is dropped — the early return
    /// is the common case. A message that *is* dropped pays the two instance-variable calls,
    /// which look their name up as a string each time, and that came to about 12 ns a message,
    /// or 10–14% of what publishing into a full queue costs (measured 2026-09-20, before and
    /// after, on a quiet machine: `docs/worklog/2026-09-20-overflow-and-limits.md`). A game
    /// that is keeping up pays none of it.
    fn make_room(&mut self, queue: ObjId, limit: usize) -> u64 {
        let mut dropped: u64 = 0;
        // an `Err` is a queue this VM cannot read, which ends the loop as a full one does
        while let Ok(n) = self.vm.task_queue_len(queue) {
            if n < limit {
                break;
            }
            // `None` is an empty queue, which cannot happen while `n >= limit` and `limit >= 1`;
            // stopping on it is what keeps this loop finite whatever the queue turns out to be,
            // and it is also what ends it when `limit` is zero
            match self.vm.task_queue_try_pop(queue) {
                Ok(Some(_)) => dropped += 1,
                _ => break,
            }
        }
        if dropped == 0 {
            return 0;
        }
        let had = match self.vm.ivar_get(queue, DROPPED_IVAR) {
            Value::Int(n) if n >= 0 => n as u64,
            _ => 0,
        };
        // saturating because the ivar is read back as a Ruby Integer: a count that ran past
        // `i64::MAX` would come back negative, and a wrong number is worse than a stuck one
        let total = had.saturating_add(dropped).min(i64::MAX as u64);
        self.vm.ivar_set(queue, DROPPED_IVAR, Value::Int(total as i64));
        dropped
    }

    /// How many published messages this VM has dropped, over all its subscriptions, since it
    /// started.
    ///
    /// A queue that is full drops its oldest to make room ([`ScriptWorld::queue_limit`]), and
    /// before this that happened in silence — no log, no counter, nothing a game could put on a
    /// HUD. A number that is climbing says either that the scripts are behind or that the limit
    /// is too small for what the game publishes; which of the two it is, is what
    /// `Rubevy::Subscription#dropped` says on the script's side, subscription by subscription.
    ///
    /// It counts messages, not queues: one publish to a name a thousand full queues listen for
    /// is a thousand. It never goes down, and a subscription ending does not take its share out.
    pub fn dropped(&self) -> u64 {
        self.vm.host_state::<HostState>().map(|s| s.dropped).unwrap_or(0)
    }

    /// The `Rubevy::Entity` class, for a host that builds a value of its own inside
    /// [`ScriptWorld::answer_value`] or [`ScriptWorld::publish_value`]: read it before the call
    /// and hand it to [`entity_object`] inside. [`Answer::Entity`] is the way where the whole
    /// answer is one entity; this is for an entity inside a list or a Hash.
    pub fn entity_class(&self) -> ObjId {
        self.entity_class
    }

    /// How many subscriptions are standing, for a HUD or a test.
    ///
    /// Counted over the names rather than kept in a field of its own, so there is one place a
    /// subscription can be added or let go of and no second number to keep in step. That is a
    /// walk of the names a VM's scripts listen for — not of the subscribers, of which there may
    /// be a thousand to a name.
    pub fn subscriptions(&self) -> usize {
        self.vm
            .host_state::<HostState>()
            .map(|s| s.subscriptions.values().map(Vec::len).sum())
            .unwrap_or(0)
    }

    /// Lets go of what an entity's script subscribed to. Called where a script's task ends and
    /// where its [`ScriptTask`] is removed: a queue nobody will ever read is a queue the game
    /// would keep filling.
    ///
    /// Each queue is **closed** before it is let go of, which is what ends the tasks waiting on
    /// it. A script's own task is terminated with its [`ScriptTask`], but a task it made with
    /// `Task.new` is not: parked on a `pop` for a message that will never come, it would stand
    /// there — holding its context, its stack and everything the block closed over — for as
    /// long as the VM lives, and its `ensure` would never run. Closing wakes it, and
    /// `Rubevy::Subscription#pop` (`src/prelude.rb`) raises `Rubevy::Unsubscribed` in it, so it
    /// unwinds through its `ensure` and ends.
    fn unsubscribe(&mut self, entity: Entity) {
        let Some(state) = self.vm.host_state_mut::<HostState>() else { return };
        let mut dropped: Vec<(u64, ObjId)> = Vec::new();
        // every name, because one script may listen for several; a name left with no subscribers
        // goes out of the map, so publishing to it is a miss again
        state.subscriptions.retain(|_name, subs| {
            subs.retain(|s| {
                if s.entity == entity {
                    dropped.push((s.seq, s.queue));
                    false
                } else {
                    true
                }
            });
            !subs.is_empty()
        });
        // in the order the script subscribed, which is the order the tasks parked on these queues
        // wake in (see `Subscription::seq`)
        dropped.sort_unstable_by_key(|(seq, _queue)| *seq);
        let close = self.vm.intern("close");
        for (_seq, queue) in dropped {
            if let Err(e) = self.vm.funcall(Value::Obj(queue), close, &[], Value::Nil) {
                let message = self.vm.describe_error(&e);
                error!("rubevy: could not close a subscription: {message}");
            }
            self.vm.gc_unregister(queue);
        }
    }
}

/// The **default** for how many messages a subscriber's queue holds before the oldest is
/// dropped — what [`ScriptWorld::queue_limit`] starts at.
///
/// A queue with no limit is a leak with a slow fuse: a script that subscribes and then waits on
/// something else would hold every message the game ever sent. 64 is enough for a script that
/// reads its queue every few frames, and small enough that a script that never reads costs
/// nothing to speak of — which is the whole of the reason it is 64
/// (`docs/worklog/2026-09-15-events.md`; nothing was measured to arrive at it).
///
/// The same starting number for every VM, so it is one constant and not one per name tag. What
/// each VM actually holds is its own field, and what one subscription holds may be its own again
/// (`Rubevy.subscribe(:belt, limit: 512)`).
const QUEUE_LIMIT: usize = 64;

impl ScriptWorld<()> {
    /// The default queue limit above, under the name a host reads it by. It is on
    /// the first VM's type rather than on every one of them because a constant of a tagged type
    /// would have to be written `ScriptWorld::<()>::QUEUE_LIMIT` by the apps that have one VM —
    /// and the number every VM starts at is the same anyway.
    ///
    /// **This is the default, not the limit.** What a VM drops messages by is the field
    /// [`ScriptWorld::queue_limit`], which an app may set to anything and a subscription may
    /// override for itself; a host that compares against this constant is comparing against the
    /// number nobody has changed.
    pub const QUEUE_LIMIT: usize = QUEUE_LIMIT;
}

/// The Ruby value for an [`Answer`], which is what both [`ScriptWorld::answer`] and
/// [`ScriptWorld::publish`] send.
fn answer_value(vm: &mut Vm, answer: Answer, entity_class: ObjId) -> Value {
    match answer {
        Answer::Nil => Value::Nil,
        Answer::Bool(b) => Value::bool(b),
        Answer::Num(n) => Value::Float(n),
        Answer::Text(t) => vm.str_new(t.as_bytes()),
        Answer::List(ns) => {
            let items: Vec<Value> = ns.into_iter().map(Value::Float).collect();
            vm.ary_new(items)
        }
        Answer::Entity(e) => vm.data_new(entity_class, ENTITY_TAG, e.to_bits()),
        Answer::Rows(rows) => {
            let items: Vec<Value> = rows
                .into_iter()
                .map(|row| {
                    let cells: Vec<Value> = row.into_iter().map(Value::Float).collect();
                    vm.ary_new(cells)
                })
                .collect();
            vm.ary_new(items)
        }
    }
}

/// Nanoseconds since the first call, on Bevy's `Instant` (which is also there in a browser).
fn clock_ns() -> u64 {
    static ORIGIN: std::sync::OnceLock<bevy::platform::time::Instant> = std::sync::OnceLock::new();
    ORIGIN.get_or_init(bevy::platform::time::Instant::now).elapsed().as_nanos() as u64
}

fn enable_scheduler_gc(vm: &mut Vm) -> Result<(), String> {
    let gc = vm.intern("GC");
    let Some(gc) = vm.const_get(vm.core.object, gc) else { return Ok(()) };
    let m = vm.intern("scheduler_driven=");
    vm.funcall(gc, m, &[Value::True], Value::Nil)
        .map(|_| ())
        .map_err(|e| vm.describe_error(&e))
}

/// What this plugin keeps inside the VM (`Vm::set_host_state`), for the natives to reach
/// through the `&mut Vm` they are given. One of these per VM, so two `App`s in one process
/// have a queue each — which is why the tests may run in parallel.
#[derive(Default)]
struct HostState {
    /// The queue a native writes and [`drain_commands`] reads.
    commands: Vec<HostCommand>,
    /// What the scripts are listening for (`Rubevy.subscribe`), **by name**: for each name, the
    /// subscribers of that name in the order they asked.
    ///
    /// It was one flat `Vec` walked by every `publish`, which made publishing cost the number of
    /// standing subscriptions whether anybody was listening for that name or not — 228 ns a
    /// message at a thousand subscriptions, for a name nobody had subscribed to, and 9.6 ns once
    /// it was keyed by name (`docs/worklog/2026-09-20-subscription-index.md`, after the event
    /// table of `docs/worklog/2026-09-20-factory-survey.md`). A game that publishes freely, as
    /// [`ScriptWorld::publish`]'s rustdoc invites it to, was paying for every other subscriber in
    /// the world. Keyed by name it is one lookup, and a name nobody listens for is a miss.
    ///
    /// A name whose last subscriber goes is taken out of the map, so this does not fill up with
    /// the names of scripts that have ended.
    subscriptions: std::collections::HashMap<String, Vec<Subscription>>,
    /// How many subscriptions have ever been made in this VM, which is where each one's
    /// [`Subscription::seq`] comes from. A counter, not a limit: nothing is refused when it
    /// grows, and a `u64` of them is more than a running game can ask for.
    subscriptions_made: u64,
    /// How many published messages this VM has dropped for want of room, over every
    /// subscription it has ever had ([`ScriptWorld::dropped`]).
    ///
    /// It is here rather than on [`ScriptWorld`] because the one place that drops a message is
    /// [`ScriptWorld::make_room`], which is already holding the `Vm` to reach the queue — and
    /// because a subscription's own count lives on its queue object, so the two counters are
    /// written in the same breath.
    dropped: u64,
}

/// One `Rubevy.subscribe(:hit)`: the queue it answered with, and whose script it belongs to.
///
/// It lives in the VM's host state beside the command queue, because the native that makes it
/// has the `Vm` and nothing else. [`ScriptWorld::publish`] reads it back through the same
/// `&mut Vm`. The name it listens for is the key it is filed under, not a field.
#[derive(Debug, Clone)]
struct Subscription {
    /// The entity the subscribing script is attached to. A message sent to an entity reaches
    /// only the subscriptions of that entity's scripts.
    ///
    /// Not an `Option`: the one place a subscription is made is the `Rubevy.subscribe` native,
    /// and it refuses a task with no entity (`ArgumentError`) — a subscription with no entity
    /// would never be let go of, since letting go of one is what `unsubscribe(entity)` does.
    /// The `Option` on the publishing side is a different thing and stays: `None` there means
    /// "everyone", not "nobody's".
    entity: Entity,
    queue: ObjId,
    /// The limit this subscription asked for (`Rubevy.subscribe(:belt, limit: 512)`), or `None`
    /// to follow the VM's [`ScriptWorld::queue_limit`].
    ///
    /// `None` rather than a copy of the VM's number taken when the script subscribed, so that
    /// there is one place the default lives and an app that changes it mid-game changes what
    /// every ordinary subscription holds.
    limit: Option<usize>,
    /// Which `Rubevy.subscribe` in this VM this was, counting from the first.
    ///
    /// Within one name the order the subscribers were asked in is the order they sit in, so
    /// publishing needs no number. [`ScriptWorld::unsubscribe`] does: it closes the queues of
    /// one entity across all the names it subscribed to, and the map's names come out in
    /// whatever order the hasher gives — a different one in every process. Closing a queue wakes
    /// what is parked on it, so that order is the order those tasks wake in. Sorting the few
    /// queues of one entity by this puts them back in the order the script asked for them,
    /// which is what a flat `Vec` gave for free.
    seq: u64,
}

fn push_command(vm: &mut Vm, c: HostCommand) {
    if let Some(state) = vm.host_state_mut::<HostState>() {
        state.commands.push(c);
    }
}

fn take_commands(vm: &mut Vm) -> Vec<HostCommand> {
    match vm.host_state_mut::<HostState>() {
        Some(state) => std::mem::take(&mut state.commands),
        None => Vec::new(),
    }
}

/// Where the VM reads a `require` from: the asset directory, `scripts` under
/// it first. A file the OS cannot see is not visible to a script either — a
/// packed or remote asset source is not served here, and a browser, which has
/// no filesystem at all, is served nothing (`tests/no_wasm_unsupported.rs`).
struct FileHost;

/// Ruby source to a RITE binary with the reference compiler, which is only in a build with the
/// `ruby-source` feature.
///
/// Both of the crate's own hosts answer `compile` with this when they were given nothing else —
/// [`FileHost`], and an [`EmbeddedHost`] without a [`compile_with`](EmbeddedHost::compile_with) —
/// so that a build without the feature says the same thing whichever of them is installed.
#[cfg(feature = "ruby-source")]
pub(crate) fn reference_compile(
    src: &[u8],
    opts: &sabiruby::EvalOptions,
) -> Result<Vec<u8>, String> {
    let o = sabiruby_compiler::Options {
        filename: opts.filename.into(), debug_info: opts.debug_info, ..Default::default()
    };
    sabiruby_compiler::compile_eval(src, &o, opts.line, opts.scopes).map_err(|e| {
        match e.diagnostics.iter().find(|d| d.kind.is_error()) {
            Some(d) => d.message.clone(),
            None => String::from("compile error"),
        }
    })
}

/// The same in a build without the feature: there is no compiler to ask.
#[cfg(not(feature = "ruby-source"))]
pub(crate) fn reference_compile(
    _src: &[u8],
    _opts: &sabiruby::EvalOptions,
) -> Result<Vec<u8>, String> {
    Err(String::from("rubevy was built without the `ruby-source` feature: require a .mrb, or `eval` is unavailable"))
}

impl sabiruby::Host for FileHost {
    fn compile(&mut self, src: &[u8], opts: &sabiruby::EvalOptions) -> Result<Vec<u8>, String> {
        reference_compile(src, opts)
    }
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(path).ok() // wasm: native-only — a browser has no filesystem to read from
    }
    fn file_exists(&mut self, path: &str) -> bool {
        std::path::Path::new(path).is_file()
    }
}

/// Where a host's systems go in the frame, so that a script's question is answered on the frame
/// it was asked and the script wakes on the next one.
///
/// The three sets run in this order inside `Update`:
///
/// | set | what is in it | what it is for |
/// |---|---|---|
/// | [`RubevySet::Deliver`] | `start_scripts`, `deliver_answers`, the release sweep | what arrived between the frames reaches the VM before a script runs |
/// | [`RubevySet::Tick`] | `tick_scripts` (exclusive), `drain_commands`, `apply_component_writes`, `apply_resource_writes` | the scripts run — and the reads they make, with the kinds the host answers through [`ScriptWorld::answer_in_tick`], are answered while they run — and what they asked the *host* for otherwise becomes a [`Request`] |
/// | [`RubevySet::Answer`] | **the host's answering systems** | the questions this frame asked are answered before the frame ends |
///
/// **Put the system that calls [`ScriptWorld::take_requests`] in [`RubevySet::Answer`]**:
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use rubevy::{Answer, RubevyPlugin, RubevySet, ScriptWorld};
/// # fn answer_requests(mut world: ResMut<ScriptWorld>) {
/// #     for request in world.take_requests() { world.answer(&request, Answer::Nil); }
/// # }
/// # fn build(app: &mut App) {
/// app.add_plugins(RubevyPlugin::default())
///     .add_systems(Update, answer_requests.in_set(RubevySet::Answer));
/// # }
/// ```
///
/// **What it is worth.** A round trip then takes **one frame**: the `Rubevy.ask` happens in
/// `Tick` and leaves a command behind, `drain_commands` (still `Tick`) turns it into a
/// [`Request`], the host answers it in `Answer`, and the script wakes in the next frame's
/// `Tick`. That one frame is checked in `tests/scheduling.rs`.
///
/// **What it costs not to use it.** A system that is in no set is ordered against nothing, so
/// where Bevy runs it is up to the executor and the rest of the app. Almost everywhere is fine
/// — before the sets, after them, in `PreUpdate`, in `PostUpdate` — because anything that
/// answers before the *next* `Tick` is soon enough. The one place that is not is **between
/// `tick_scripts` and `drain_commands`**: there the system misses the question this frame asked
/// (it becomes a [`Request`] a moment later) and answers the previous frame's too late for this
/// frame's scripts, so every round trip costs **two** frames. That is what rubevy_games measured
/// with an unordered answering system: 2.0 frames for every question the game answered against
/// 1.0 for every question rubevy answered itself
/// (`docs/worklog/2026-09-16-showpieces-d2-d3.md` there). The cost was not a rule of the bridge,
/// it was where the system happened to land — and the point of the set is that `Deliver` and
/// `Answer` are outside that window by construction. (Do not put an answering system *inside*
/// [`RubevySet::Tick`]: that is the one set that contains the window.)
///
/// The questions rubevy answers itself (a component by name) are not in this at all any more.
/// They cost no frame: `tick_scripts` answers them between two runs of the VM, so the script
/// has the value in the line it asked for it, and `Answer` is the host's set alone. A host that
/// wants a question of its own answered there too — a spatial one, asked every frame — registers
/// it with [`ScriptWorld::answer_in_tick`] instead of answering it in a system; the trade is
/// that the closure sees the world as it stands in `Tick` and not the finished frame.
///
/// **One set of three per VM.** `RubevySet::Deliver` is the first VM's, `RubevySet::<Mods>` the
/// sets of the VM `RubevyPlugin::<Mods>` started; they are different sets, so the two VMs' frames
/// are ordered against each other only where an app says so.
///
/// It is a struct with three constants rather than an enum with three variants, which is what it
/// was while there was one VM. The reason is Rust's and not Bevy's: a name tag with a default
/// (`RubevySet<M = ()>`) is filled in where a *type* is written, and not where a *value* is — so
/// `RubevySet::Answer` as an enum variant would have been a type the compiler could not infer,
/// and every app with one VM would have had to write `RubevySet::<()>::Answer`. As a constant of
/// `impl RubevySet<()>` it is found the way `Script::new` is, and the tagged VM's sets are
/// [`RubevySet::deliver`], [`RubevySet::tick`] and [`RubevySet::answer`].
#[derive(SystemSet)]
pub struct RubevySet<M = ()> {
    which: SetKind,
    _m: PhantomData<fn() -> M>,
}

/// Which of the three [`RubevySet`]s this is. Private: what a host names is the constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SetKind {
    Deliver,
    Tick,
    Answer,
}

// The names an app with one VM writes. `non_upper_case_globals` is allowed because these stand
// where the enum's variants stood: `RubevySet::Deliver` is what every app already says.
#[allow(non_upper_case_globals)]
impl RubevySet<()> {
    /// Before the scripts run: what finished between the frames reaches the VM.
    pub const Deliver: RubevySet<()> = RubevySet::deliver();
    /// The scripts run, and their questions become [`Request`]s.
    pub const Tick: RubevySet<()> = RubevySet::tick();
    /// The questions are answered — where a host's own answering systems go.
    pub const Answer: RubevySet<()> = RubevySet::answer();
}

impl<M: 'static> RubevySet<M> {
    /// [`RubevySet::Deliver`] of the VM named `M`: `RubevySet::<Mods>::deliver()`.
    pub const fn deliver() -> RubevySet<M> {
        RubevySet { which: SetKind::Deliver, _m: PhantomData }
    }
    /// [`RubevySet::Tick`] of the VM named `M`.
    pub const fn tick() -> RubevySet<M> {
        RubevySet { which: SetKind::Tick, _m: PhantomData }
    }
    /// [`RubevySet::Answer`] of the VM named `M`.
    pub const fn answer() -> RubevySet<M> {
        RubevySet { which: SetKind::Answer, _m: PhantomData }
    }
}

// `SystemSet` asks for these six, and a derive would ask the name tag for them in turn (§6.1 of
// `docs/plans/multi-vm-plan.md`): written out, a tag is an empty struct that implements nothing.
// Two sets are the same set when they are the same one of the three *and* carry the same tag —
// the tag is in the type, so it is Rust that keeps `RubevySet<Mods>::Tick` apart from
// `RubevySet::Tick`, not this `eq`. Bevy asks through `DynEq` / `DynHash`, which downcast to
// `Self` before comparing and mix the `TypeId` into the hash (`bevy_ecs/src/label.rs:29,51`), so
// ignoring the tag here costs nothing: the two VMs' chains are configured independently.
impl<M> std::fmt::Debug for RubevySet<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RubevySet::{:?}", self.which)
    }
}

impl<M> Clone for RubevySet<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Copy for RubevySet<M> {}

impl<M> PartialEq for RubevySet<M> {
    fn eq(&self, other: &Self) -> bool {
        self.which == other.which
    }
}

impl<M> Eq for RubevySet<M> {}

impl<M> std::hash::Hash for RubevySet<M> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.which.hash(state);
    }
}

/// Adds the VM, the `.mrb` asset loader and the systems, in the three sets of [`RubevySet`].
///
/// `RubevyPlugin::default()` is the app's first VM. A second one is the same plugin under
/// another name tag, with its own asset root:
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use rubevy::RubevyPlugin;
/// struct Mods;
/// # fn build(app: &mut App) {
/// app.add_plugins(RubevyPlugin::default())
///     .add_plugins(RubevyPlugin::<Mods>::for_vm("assets/mods"));
/// # }
/// ```
pub struct RubevyPlugin<M = ()> {
    /// Where a `require` reads from (Bevy's asset directory).
    pub asset_root: String,
    _m: PhantomData<fn() -> M>,
}

/// The first VM. It is `RubevyPlugin<()>` and not a plugin of every name tag, because
/// `RubevyPlugin::default()` — which every app that has one VM writes — has nothing to infer a
/// tag from.
impl Default for RubevyPlugin<()> {
    fn default() -> Self {
        RubevyPlugin { asset_root: String::from("assets"), _m: PhantomData }
    }
}

impl RubevyPlugin<()> {
    /// The app's first VM, reading its `require`s from `root`:
    /// `RubevyPlugin::with_asset_root("assets")`.
    ///
    /// It is spelled without a name tag for the same reason [`Script::new`] is: a default type
    /// parameter is filled in where a *type* is written and not where a value is, so a generic
    /// `with_asset_root` would leave the plain call — which is the one every app with one VM
    /// writes — with nothing to infer the tag from. The VM with a tag has [`RubevyPlugin::for_vm`].
    pub fn with_asset_root(root: impl Into<String>) -> Self {
        RubevyPlugin::<()>::for_vm(root)
    }
}

impl<M: 'static> RubevyPlugin<M> {
    /// The VM named `M`, reading its `require`s from `root`:
    /// `RubevyPlugin::<Mods>::for_vm("assets/mods")`.
    ///
    /// The name is the pair to [`Script::for_vm`]: everything a tagged VM is reached by is
    /// `for_vm`, and the untagged spellings — `RubevyPlugin::default()`,
    /// `RubevyPlugin::with_asset_root(..)`, `Script::new(..)` — stay what they were.
    pub fn for_vm(root: impl Into<String>) -> Self {
        RubevyPlugin { asset_root: root.into(), _m: PhantomData }
    }
}

impl<M: 'static> Plugin for RubevyPlugin<M> {
    fn build(&self, app: &mut App) {
        let mut world = match ScriptWorld::<M>::new() {
            Ok(w) => w,
            Err(e) => panic!("rubevy: could not start the VM: {e}"),
        };
        world.vm.set_host(Box::new(FileHost));
        let root = &self.asset_root;
        world.vm.set_load_path(&[&format!("{root}/scripts"), root]);
        // `MrbAsset` and its loader belong to no VM in particular, so the second plugin must
        // not register them again: `init_asset` builds a *fresh* `Assets<MrbAsset>` and inserts
        // it, which would drop the handles the first VM's scripts are already holding, and
        // `init_asset_loader` would leave two loaders claiming `.mrb`.
        // `app.is_plugin_added::<RubevyPlugin<M>>()` cannot see this — `RubevyPlugin<Mods>` is a
        // different type from `RubevyPlugin<()>` — so the test is on the thing itself.
        if !app.world().contains_resource::<Assets<MrbAsset>>() {
            app.init_asset::<MrbAsset>().init_asset_loader::<MrbLoader>();
        }
        // `ScriptEnded<M>` is per-VM, on the other hand: another name tag is another message
        // type, and it does want its own.
        app.add_message::<ScriptEnded<M>>()
            .insert_resource(world)
            .configure_sets(
                Update,
                (RubevySet::<M>::deliver(), RubevySet::<M>::tick(), RubevySet::<M>::answer())
                    .chain(),
            )
            .add_systems(
                Update,
                // `retry_failed_scripts` is before `start_scripts` so that an asset changed on
                // the frame before is tried again on this one and not the next
                (
                    retry_failed_scripts::<M>,
                    start_scripts::<M>,
                    deliver_answers::<M>,
                    release_values::<M>,
                )
                    .chain()
                    .in_set(RubevySet::<M>::deliver()),
            )
            .add_systems(
                Update,
                // `tick_scripts` is exclusive now (it answers the reads of the scripts it is
                // running, which takes the world), so the whole set is a point the frame passes
                // through one system at a time — two VMs tick one after the other
                (
                    tick_scripts::<M>,
                    drain_commands::<M>,
                    apply_component_writes::<M>,
                    apply_resource_writes::<M>,
                )
                    .chain()
                    .in_set(RubevySet::<M>::tick()),
            );
        // Nothing of rubevy's is in `RubevySet::Answer` any more: the questions rubevy answers
        // itself are answered inside the tick, and the set is the game's alone.
    }
}

/// The instance variable each task carries its entity in (`Vm::ivar_set`), read back by the
/// natives through [`Vm::task_running`]. A script can see it — it is an ordinary `@ivar` — but
/// the name is not one a script would write by accident.
const ENTITY_IVAR: &str = "@rubevy_entity";

/// The instance variable a subscription's queue carries its dropped count in, written by
/// [`ScriptWorld::make_room`] and read back by `Rubevy::Subscription#dropped` (`src/prelude.rb`).
///
/// An ordinary `@ivar` on the queue object, as [`ENTITY_IVAR`] is on the task: the collector
/// reaches it through the object that holds it, it goes when the subscription goes, and a script
/// may read it directly if it would rather not call the method.
const DROPPED_IVAR: &str = "@rubevy_dropped";

/// The [`Script`]s of this VM that are not running and have not ended: what [`start_scripts`]
/// looks at each frame.
///
/// It is a `type` for the same reason [`RunningTasks`] is — a query of three things filtered by
/// two is a mouthful in a signature — and the three and the two are: the entity, its script, and
/// whether it is one that failed to start before ([`ScriptStartFailed`], which is taken off when
/// it does start); no task, and not ended.
type PendingScripts<'w, 's, M> = Query<
    'w,
    's,
    (Entity, &'static Script<M>, Has<ScriptStartFailed<M>>),
    (Without<ScriptTask<M>>, Without<ScriptDone<M>>),
>;

/// Turns every [`Script`] whose asset has arrived into a task.
///
/// A script that **cannot** be turned into one — the bytes will not load, or the VM will not
/// spawn the task — ends here instead: a [`ScriptEnded`] of [`ScriptStatus::Failed`] carrying
/// what went wrong, and a [`ScriptDone`] so that this is said once ([`give_up`]).
fn start_scripts<M: 'static>(
    mut commands: Commands,
    assets: Res<Assets<MrbAsset>>,
    mut world: ResMut<ScriptWorld<M>>,
    mut ended: MessageWriter<ScriptEnded<M>>,
    pending: PendingScripts<M>,
) {
    for (entity, script, had_failed) in &pending {
        let Some(asset) = assets.get(&script.source) else { continue };
        let name = script.name.clone().unwrap_or_else(|| format!("{entity}"));
        // the program is loaded once however many entities run it (`ScriptWorld::irep_of`);
        // what is per-entity is the task, below
        let irep = match world.irep_of(&asset.bytes, &name) {
            Ok(i) => i,
            Err(message) => {
                give_up(&mut commands, &mut ended, entity, message);
                continue;
            }
        };
        let vm = &mut world.vm;
        match vm.task_spawn(irep, script.priority, Some(&name)) {
            Ok(task) => {
                // the entity holds the task, so the collector must not take it
                vm.gc_register(task);
                // the task carries its entity, which is what `Rubevy.entity` answers
                vm.ivar_set(task, ENTITY_IVAR, Value::Int(entity.to_bits() as i64));
                commands.entity(entity).insert(ScriptTask::<M> { task, _m: PhantomData });
                // it started this time: whatever went wrong before is over, and the mark that
                // said so must not be left on the entity for a later asset change to act on
                if had_failed {
                    commands.entity(entity).remove::<ScriptStartFailed<M>>();
                }
            }
            Err(e) => {
                // unlike a program that will not load, this is the VM's answer to *this* task
                // (no room for another context, say), so it is one entity's news and is said
                // per entity
                let message = vm.describe_error(&e);
                error!("rubevy: {name} failed to start: {message}");
                give_up(&mut commands, &mut ended, entity, message);
            }
        }
    }
}

/// Ends a script that never started: the entity is told once, and marked so that
/// [`start_scripts`] does not come back to it every frame.
///
/// The ending is the one every other script gets — a [`ScriptEnded`] with
/// [`ScriptStatus::Failed`] and, as its value, what the VM said. A game that shows the ending of
/// a script shows this one with nothing added, which is the point of using the ending it already
/// had rather than a message of its own.
///
/// [`ScriptStartFailed`] is what makes it a *pause* rather than a full stop: an asset that
/// changes takes both marks off again ([`retry_failed_scripts`]).
fn give_up<M: 'static>(
    commands: &mut Commands,
    ended: &mut MessageWriter<ScriptEnded<M>>,
    entity: Entity,
    message: String,
) {
    ended.write(ScriptEnded { entity, status: ScriptStatus::Failed, value: message, _m: PhantomData });
    commands
        .entity(entity)
        .insert((ScriptDone::<M>::for_vm(), ScriptStartFailed::<M> { _m: PhantomData }));
}

/// Takes the mark off a script that could not start, when the asset it names is changed.
///
/// A `.mrb` that would not load is worth trying again the moment it is another `.mrb`, and that
/// is what an editor's Apply, a `cargo` rebuild picked up by the asset server's hot reload, or a
/// download that finished properly the second time all are. Two ways an asset can change reach
/// this: [`AssetEvent::Modified`], which is the same handle with other bytes, and
/// [`replace_script`], which is another handle altogether and takes the [`ScriptDone`] off
/// itself.
///
/// It reads the events of **every** `MrbAsset`, not only this VM's, because that is the only
/// form they come in; what is per-VM is the query, so the failed scripts of the VM named `M` are
/// the only ones it can touch.
fn retry_failed_scripts<M: 'static>(
    mut commands: Commands,
    mut changed: MessageReader<AssetEvent<MrbAsset>>,
    failed: Query<(Entity, &Script<M>), With<ScriptStartFailed<M>>>,
) {
    let changed: Vec<AssetId<MrbAsset>> = changed
        .read()
        .filter_map(|e| match e {
            AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    if changed.is_empty() {
        return;
    }
    // a `Vec` and not a set: what is in it is the assets that changed in one frame, and what it
    // is searched for is one lookup per script that is already known to be broken
    for (entity, script) in &failed {
        if changed.contains(&script.source.id()) {
            commands.entity(entity).remove::<ScriptStartFailed<M>>().remove::<ScriptDone<M>>();
        }
    }
}

/// Lets the collector have the values of the [`Request`]s that were dropped since the last
/// frame ([`ScriptWorld::release_dropped_values`]).
///
/// It is in [`RubevySet::Deliver`], which is the last moment with the `Vm` before a script runs:
/// a host answering in [`RubevySet::Answer`] drops its requests at the end of the frame, and the
/// values they held stop being registered at the head of the next one, so an object can be
/// collected on the first frame it is no longer needed.
fn release_values<M: 'static>(mut world: ResMut<ScriptWorld<M>>) {
    world.release_dropped_values();
}

/// The scripts of this VM that have not ended, kept between frames.
///
/// An exclusive system is handed the world and nothing else, so the one query [`tick_scripts`]
/// still wants is spelled as a `SystemState`, which an exclusive system may take beside the
/// world. That keeps the archetype matching from frame to frame the way the `Query` parameter
/// did; `World::query_filtered` would build it again every frame.
type RunningTasks<M> =
    SystemState<Query<'static, 'static, (Entity, &'static ScriptTask<M>), Without<ScriptDone<M>>>>;

/// Moves the scheduler's clock on by the frame time, runs the ready tasks for up to the frame's
/// budget, and answers the component reads they make **while they are still running**.
///
/// It is an exclusive system (`&mut World`) for the sake of that last part. A read —
/// `entity[:Transform]`, `has?`, `components`, `Rubevy.find` — parks the task that made it on a
/// queue, and `Vm::task_run_limits` comes back as soon as no task can run. That moment is the
/// one place in the frame where both halves of the answer are in the same pair of hands: the
/// world the value is in, and the VM the value has to be built in. So a frame is a loop rather
/// than a single run:
///
/// > run the tasks → answer the reads they parked on → run them again
///
/// and it ends when a round answers nothing (nobody would wake), or when the budget or the
/// frame time is spent. The script gets the value in the line it asked for it, and what it sees
/// is the world **as it stands in [`RubevySet::Tick`]** — which is what makes a game's own
/// `.before(RubevySet::Tick)` mean something.
///
/// **Writes are not part of this.** They still go through [`apply_component_writes`] at the end
/// of the frame, so a read that follows a write in the same tick reads the *old* value. The
/// promise a write makes is the one `Commands` makes, and the games are built on it.
///
/// **Why there is no `unsafe` in it.** Nothing of the world crosses into the VM. `resource_scope`
/// takes [`ScriptWorld<M>`] out of the world for the length of the loop, the natives are handed
/// `&mut Vm` and nothing else exactly as they were, and the `&World` never leaves this function:
/// the two references are two arguments of [`answer_reflect_requests`], held apart by the
/// borrow checker like any others. For the same reason nothing structural happens inside the
/// loop — while `ScriptWorld<M>` is out of the world the `on_remove` hook of [`ScriptTask`]
/// cannot find it and would lose a task quietly — so the sweep of finished scripts is after the
/// closure, and `Rubevy.spawn` / `Rubevy.despawn` stay with [`drain_commands`].
fn tick_scripts<M: 'static>(world: &mut World, tasks: &mut RunningTasks<M>) {
    let (delta, elapsed) = {
        let time = world.resource::<Time>();
        (time.delta_secs(), time.elapsed_secs())
    };
    let frame_no = world.resource::<FrameCount>().0;
    // the tasks to sweep at the end of the frame, read before `ScriptWorld` leaves the world:
    // a query cannot be run inside `resource_scope`'s closure without the world being borrowed
    // twice, and the sweep wants the VM anyway
    // (a read-only `Query` has nothing to validate, so the `Err` arm is unreachable; it is
    // spelled rather than unwrapped because a panic here would take the frame with it)
    let running: Vec<(Entity, ObjId)> = tasks
        .get(world)
        .map(|q| q.iter().map(|(entity, st)| (entity, st.task)).collect())
        .unwrap_or_default();

    world.resource_scope(|world: &mut World, mut scripts: Mut<ScriptWorld<M>>| {
        let scripts = &mut *scripts;
        // frame time in ticks, keeping what did not make a whole one for next frame — unless the
        // scripts are paused, in which case this frame does not count as time for them (see the
        // rustdoc of `ScriptWorld::budget`)
        if scripts.budget > 0 {
            let unit_ms = scripts.vm.task_tick_unit_ms() as f32;
            scripts.tick_remainder += delta * 1000.0 / unit_ms;
            let whole = scripts.tick_remainder.floor().max(0.0);
            scripts.tick_remainder -= whole;
            if whole >= 1.0 {
                scripts.vm.task_advance_ticks(whole as u32);
            }
        }

        set_frame_state(&mut scripts.vm, frame_no, delta, elapsed);

        // The answer loop. There is no count of rounds in it and no limit of its own: a round
        // happens only when the round before it answered somebody, a question costs the
        // instructions the script spent asking it, and the budget and the frame time are
        // checked at the head of every round — so the two numbers the frame already had are
        // what end it. Since 2026-09-20 the frame time is checked *inside* the answering as
        // well: a round that parked a thousand tasks has a thousand answers to make, and making
        // all of them whatever the clock said is how a `frame_time` of 1 ms became a tick of
        // 3.58 ms (`docs/worklog/2026-09-20-factory-survey.md`).
        //
        // **The clock here is [`clock_ns`], the one the VM itself was given** — Bevy's `Instant`,
        // which is `web-time` in a browser. It is not `std::time::Instant`: that one *panics* on
        // `wasm32-unknown-unknown` ("time not implemented on this platform"), which is a panic in
        // the middle of the first frame and takes the whole page with it. The loop's clock and the
        // VM's deadline clock being one source is also what makes `time_ns` below mean what it
        // says.
        let started_ns = clock_ns();
        // the one moment the frame's scripts must be done by, on that same clock: the head of
        // every round is measured against it, and so is every answer made in the round
        // (`answer_reflect_requests`), which is what makes `frame_time` a limit on the tick and
        // not only on the runs of the VM inside it
        let deadline_ns = scripts.frame_time.map(|t| started_ns.saturating_add(t.as_nanos() as u64));
        let mut spent = 0u64;
        loop {
            let left = scripts.budget.saturating_sub(spent);
            if left == 0 {
                break;
            }
            let time_left = deadline_ns.map(|deadline| deadline.saturating_sub(clock_ns()));
            if time_left == Some(0) {
                break;
            }
            let limits = sabiruby::RunLimits {
                instructions: Some(left),
                time_ns: time_left,
                overrun_ns: scripts.overrun.map(|d| d.as_nanos() as u64),
                ..Default::default()
            };
            match scripts.vm.task_run_limits(limits) {
                Ok(instructions) => spent += instructions,
                Err(e) => {
                    // the scheduler itself failed, which a task's own exception never does
                    let message = scripts.vm.describe_error(&e);
                    error!("rubevy: the scheduler raised: {message}");
                    break;
                }
            }
            // the tasks have stopped: either every one of them is waiting for something, or the
            // frame is spent. Answer what this system can answer — rubevy's own kinds first,
            // then the game's in-tick answerers — and if that woke anybody, give them what is
            // left of the frame.
            let answered = answer_reflect_requests(&*world, scripts, deadline_ns)
                + answer_in_tick_requests(&*world, scripts, deadline_ns);
            if answered == 0 {
                break;
            }
        }
        flush_output(&mut scripts.vm);
    });

    // The scripts that ran to their end. It is outside the closure because `ScriptWorld<M>` is
    // back in the world here: inserting `ScriptDone` is a structural change, and a structural
    // change made while the resource was out of the world is one the component hooks cannot
    // follow.
    let mut finished: Vec<ScriptEnded<M>> = Vec::new();
    {
        let mut scripts = world.resource_mut::<ScriptWorld<M>>();
        let scripts = &mut *scripts;
        for (entity, task) in running {
            if !scripts.vm.task_finished(task) {
                continue;
            }
            let value = scripts.vm.task_value(task);
            let status =
                if scripts.vm.is_exception(value) { ScriptStatus::Failed } else { ScriptStatus::Finished };
            let text = scripts.vm.inspect_str(value).unwrap_or_else(|_| String::from("?"));
            finished.push(ScriptEnded { entity, status, value: text, _m: PhantomData });
            scripts.vm.gc_unregister(task);
            // A script that runs to its end keeps its `ScriptTask` — that is what stops it
            // starting again — so the `on_remove` hook is not reached here. What it subscribed
            // to is let go of all the same: the script will never read those queues.
            scripts.unsubscribe(entity);
        }
    }
    for message in finished {
        let entity = message.entity;
        world.write_message(message);
        if let Ok(mut e) = world.get_entity_mut(entity) {
            e.insert(ScriptDone::<M>::for_vm());
        }
    }
}

/// `$rubevy`, refreshed at the head of every frame.
fn set_frame_state(vm: &mut Vm, frame: u32, delta: f32, elapsed: f32) {
    let h = vm.hash_new();
    for (key, value) in [
        ("frame", Value::Int(frame as i64)),
        ("delta", Value::Float(delta as f64)),
        ("time", Value::Float(elapsed as f64)),
    ] {
        let k = Value::Sym(vm.intern(key));
        let _ = vm.hash_set(h, k, value);
    }
    vm.global_set("$rubevy", h);
}

fn flush_output(vm: &mut Vm) {
    let out = vm.take_output();
    if out.is_empty() {
        return;
    }
    for line in String::from_utf8_lossy(&out).lines() {
        info!("[script] {line}");
    }
}

/// Answers the requests whose futures have finished ([`ScriptWorld::answer_with`]).
///
/// It runs at the head of the frame, before [`tick_scripts`], rather than beside
/// [`drain_commands`] at the end of it: a future finishes at whatever moment its thread is done,
/// and most of a frame's wall clock is outside this schedule (the wait for the display). An
/// answer that arrived in that gap is given to the VM before the scripts run, so the script that
/// asked wakes on this frame instead of the next one. Nothing is lost the other way: a future
/// that finishes later in this frame is picked up at the head of the next, which is where the
/// script would have woken anyway.
fn deliver_answers<M: 'static>(mut world: ResMut<ScriptWorld<M>>) {
    if world.answering.is_empty() {
        return;
    }
    let world = &mut *world;
    let mut i = 0;
    while i < world.answering.len() {
        // `poll_once` on the pool's handle: the future itself is driven by the pool, and this
        // only asks whether its answer is there yet (bevy's own way of reading a `Task`)
        match block_on(poll_once(&mut world.answering[i].1)) {
            Some(answer) => {
                let (request, _finished) = world.answering.remove(i);
                world.answer(&request, answer);
            }
            None => i += 1,
        }
    }
}

/// Carries out what the scripts asked for this frame.
fn drain_commands<M: 'static>(
    mut commands: Commands,
    mut transforms: Query<&mut Transform>,
    mut world: ResMut<ScriptWorld<M>>,
) {
    for c in take_commands(&mut world.vm) {
        match c {
            HostCommand::Ask { entity, kind, args, queue } => {
                // parked scripts wait here until a system of the game answers
                // (`ScriptWorld::take_requests` / `answer`) — except the kinds that are
                // answered inside the tick, which are sorted out here rather than left for a
                // game's system to skip over: the handful rubevy answers itself, and the ones
                // the game registered with `answer_in_tick`. What lands in either of those two
                // is the leftovers of a frame that ran out of budget or of time; the next
                // frame's answer loop takes them first.
                let request = Request { entity: entity_from_bits(entity), kind, args, queue };
                if RESERVED_KINDS.contains(&request.kind.as_str()) {
                    world.reflect_requests.push(request);
                } else if world.in_tick_answerers.contains_key(&request.kind) {
                    world.in_tick_requests.push(request);
                } else {
                    world.requests.push(request);
                }
            }
            HostCommand::SetComponent { entity, name, value } => {
                if let Some(entity) = entity_from_bits(entity) {
                    world.component_writes.push(ComponentWrite { entity, name, value });
                }
            }
            HostCommand::SetResource { name, value } => {
                world.resource_writes.push(ResourceWrite { name, value });
            }
            HostCommand::Log(text) => info!("[script] {text}"),
            HostCommand::Spawn { name, x, y, z } => {
                commands.spawn((SpawnedByScript { name }, Transform::from_xyz(x, y, z)));
            }
            HostCommand::Despawn(bits) => {
                if let Some(e) = entity_from_bits(bits) {
                    commands.entity(e).despawn();
                }
            }
            HostCommand::SetPosition { entity, x, y, z } => {
                if let Some(e) = entity_from_bits(entity) {
                    if let Ok(mut t) = transforms.get_mut(e) {
                        t.translation = Vec3::new(x, y, z);
                    }
                }
            }
        }
    }
}

fn entity_from_bits(bits: u64) -> Option<Entity> {
    Entity::try_from_bits(bits)
}

// ------------------------------------------------------------------ components by name

/// The `Rubevy.ask` kinds rubevy answers itself, in [`answer_reflect_requests`]. A game never sees
/// them in [`ScriptWorld::take_requests`], and a game that wants these names for itself has to
/// pick others — [`ScriptWorld::answer_in_tick`] included, since those are taken off the
/// queue before the game's own in-tick answerers are.
///
/// They are what `src/prelude.rb` sends: `Rubevy::Entity#[]`, `#has?`, `#components`,
/// `Rubevy.find` and `Rubevy.resource`.
const RESERVED_KINDS: [&str; 5] =
    ["component.get", "component.has", "components", "entities.with", "resource.get"];

/// Takes the questions rubevy answers itself off the VM's command queue, leaving every other
/// command where it is.
///
/// The `Rubevy.ask` native cannot tell the two apart — it is handed `&mut Vm` and puts a
/// [`HostCommand::Ask`] on the queue whatever the kind — and the sorting has always been
/// [`drain_commands`]'s. The tick needs the reserved four *before* `drain_commands` runs, so it
/// takes those out here and lets the rest lie: a game's questions keep the order they were asked
/// in, and the commands that need a `Commands` or a `Query` (`Rubevy.spawn`, `Rubevy.despawn`,
/// `Rubevy.log`, `Rubevy.move_to`) are still carried out where they always were.
fn take_reflect_asks(vm: &mut Vm) -> Vec<Request> {
    take_asks(vm, |kind| RESERVED_KINDS.contains(&kind))
}

/// The `Rubevy.ask`s whose kind `wanted` says yes to, taken off the VM's command queue in the
/// order they were asked in, with every other command left where it is.
///
/// Two callers, and both of them are the answer loop: [`take_reflect_asks`] for the kinds rubevy
/// answers itself and [`answer_in_tick_requests`] for the kinds the game registered with
/// [`ScriptWorld::answer_in_tick`].
fn take_asks(vm: &mut Vm, wanted: impl Fn(&str) -> bool) -> Vec<Request> {
    let Some(state) = vm.host_state_mut::<HostState>() else { return Vec::new() };
    let mut taken = Vec::new();
    let mut i = 0;
    while i < state.commands.len() {
        let take = match &state.commands[i] {
            HostCommand::Ask { kind, .. } => wanted(kind.as_str()),
            _ => false,
        };
        if !take {
            i += 1;
            continue;
        }
        if let HostCommand::Ask { entity, kind, args, queue } = state.commands.remove(i) {
            taken.push(Request { entity: entity_from_bits(entity), kind, args, queue });
        }
    }
    taken
}

/// The `ReflectComponent` a name stands for, through [`ScriptWorld::reflect_cache`].
///
/// The cache is what makes a read that happens a dozen times a frame cheap: without it every one
/// of them is a lookup by short path, a lookup by `TypeId` and a downcast, which is what bevy's
/// own rustdoc on `ReflectComponent` says to keep between frames instead. A `ReflectComponent`
/// is a handful of function pointers, so the clone is a copy.
fn reflect_component_of<M: 'static>(
    scripts: &mut ScriptWorld<M>,
    registry: &bevy::reflect::TypeRegistry,
    name: &str,
) -> Option<bevy::ecs::reflect::ReflectComponent> {
    if let Some(rc) = scripts.reflect_cache.get(name) {
        return Some(rc.clone());
    }
    // the component's short name (`"Transform"`), or its whole path where two types share the
    // short one (`"my_game::Hp"`)
    let rc = registry
        .get_with_short_type_path(name)
        .or_else(|| registry.get_with_type_path(name))?
        .data::<bevy::ecs::reflect::ReflectComponent>()?
        .clone();
    scripts.reflect_cache.insert(name.to_string(), rc.clone());
    Some(rc)
}

/// What a resource's name stands for, through [`ScriptWorld::resource_cache`]: the `TypeId` the
/// world keeps the resource under and the `ReflectComponent` that reads or writes it.
///
/// **Why a `ReflectComponent` for a resource.** In Bevy 0.19 a resource *is* a component, on an
/// entity of its own ([`World::resource_entities`]), and `ReflectResource` is a marker carrying
/// no functions at all — its own rustdoc says the `ReflectComponent` of the same type "is meant
/// to be used instead", and `#[reflect(Resource)]` registers one. So what `ReflectResource` is
/// good for here is the one thing it says: that this type calls itself a resource. Asking for it
/// is what keeps `Rubevy.resource(:Transform)` nil rather than answering out of whatever entity
/// happened to hold one.
///
/// The name is resolved exactly as a component's is — the short type path first
/// (`"Score"`, `"Time<Virtual>"`), the whole path where two types share the short one — so a
/// script names a resource the way it names a component.
fn reflect_resource_of<M: 'static>(
    scripts: &mut ScriptWorld<M>,
    registry: &bevy::reflect::TypeRegistry,
    name: &str,
) -> Option<(std::any::TypeId, bevy::ecs::reflect::ReflectComponent)> {
    if let Some(found) = scripts.resource_cache.get(name) {
        return Some(found.clone());
    }
    let found = resource_data(registry, name)?;
    scripts.resource_cache.insert(name.to_string(), found.clone());
    Some(found)
}

/// The registry half of [`reflect_resource_of`], without the cache: the two systems that write
/// (`&mut World`, no `ScriptWorld` to borrow) use this one.
fn resource_data(
    registry: &bevy::reflect::TypeRegistry,
    name: &str,
) -> Option<(std::any::TypeId, bevy::ecs::reflect::ReflectComponent)> {
    let registration =
        registry.get_with_short_type_path(name).or_else(|| registry.get_with_type_path(name))?;
    registration.data::<bevy::ecs::reflect::ReflectResource>()?;
    let rc = registration.data::<bevy::ecs::reflect::ReflectComponent>()?.clone();
    Some((registration.type_id(), rc))
}

/// The entity Bevy keeps a resource of that type on, or `None` where the world has no such
/// resource — which is what a script asking for one nothing inserted gets.
fn resource_entity(world: &World, type_id: std::any::TypeId) -> Option<Entity> {
    let id = world.components().get_valid_id(type_id)?;
    world.resource_entities().get(id)
}

/// Answers the questions about components from the world the tick is holding, and says how many
/// it answered. Called by [`tick_scripts`] between two runs of the VM — this is what makes a
/// read return inside the same tick.
///
/// It was a system of its own (`answer_components`, in [`RubevySet::Answer`]) while a read cost
/// a frame: the `Rubevy.ask` left a command behind, [`drain_commands`] turned it into a request,
/// the system answered it at the end of the frame and the script woke in the next one. The same
/// reflection is done here, only sooner, and the argument is `&World` and not `&mut World`
/// because reading is all of it.
///
/// It looks in two places, in this order:
/// * [`ScriptWorld::reflect_requests`] — the leftovers of a frame that ended with questions
///   still unanswered (the budget or the frame time ran out), sorted there by
///   [`drain_commands`] or left there by this function.
/// * the VM's command queue, where the questions of the round that has just stopped are
///   ([`take_reflect_asks`]).
///
/// **`deadline_ns` is where the frame time bites.** The clock ([`clock_ns`], the VM's own) is
/// looked at after every answer, and once it is past the deadline what is still unanswered goes
/// back to [`ScriptWorld::reflect_requests`] — in the order it was asked in, in front of
/// everything the next frame will ask, so a question that is put off is put off once.
/// **One answer is always made**, however late the tick already is. That is not politeness, it
/// is what keeps the loop moving: the run of the VM that precedes this call may itself have used
/// the whole of `frame_time`, and a frame that answered nobody would leave every task parked on
/// a question for ever. So the granularity of the limit is one answer — a tick can overrun
/// `frame_time` by the cost of the single most expensive answer it makes, and the expensive one
/// is `entities.with` (`Rubevy.find`), which walks every entity in the world.
///
/// `deadline_ns` is `None` when the app set no [`ScriptWorld::frame_time`]: then the clock is
/// never read here and every question of the round is answered, exactly as before this existed.
///
/// What is reachable here is what is registered: a type with `#[derive(Reflect)]`,
/// `#[reflect(Component)]` and `app.register_type::<T>()`. Bevy registers its own
/// (`Transform`, `Visibility`, `Name`, …) in the plugins that own them — `TransformPlugin` for
/// `Transform`, which `MinimalPlugins` does not add. An unregistered type is not an error: the
/// component reads as `nil`, `has?` as `false`, and it is not in `components`.
fn answer_reflect_requests<M: 'static>(
    world: &World,
    scripts: &mut ScriptWorld<M>,
    deadline_ns: Option<u64>,
) -> usize {
    let mut asked = std::mem::take(&mut scripts.reflect_requests);
    asked.append(&mut take_reflect_asks(&mut scripts.vm));
    if asked.is_empty() {
        return 0;
    }
    let Some(registry) = world.get_resource::<AppTypeRegistry>().cloned() else {
        warn!("rubevy: no AppTypeRegistry, so no component is reachable by name");
        let answered = asked.len();
        for request in asked {
            scripts.answer(&request, Answer::Nil);
        }
        return answered;
    };
    let registry = registry.read();
    let entity_class = scripts.entity_class;
    let mut answered = 0usize;
    {
        let mut asked = asked.into_iter();
        while let Some(request) = asked.next() {
            match request.kind.as_str() {
                "component.get" => {
                    let rc = request
                        .text(1)
                        .and_then(|name| reflect_component_of(scripts, &registry, name));
                    let value = rc
                        .as_ref()
                        .zip(request.entity_arg(0).and_then(|e| world.get_entity(e).ok()))
                        .and_then(|(rc, entity)| rc.reflect(entity));
                    match value {
                        Some(value) => scripts.answer_value(&request, |vm| {
                            reflect::reflect_to_ruby(
                                vm,
                                value.as_partial_reflect(),
                                entity_class,
                                ENTITY_TAG,
                            )
                        }),
                        None => scripts.answer(&request, Answer::Nil),
                    }
                }
                // `Rubevy.resource(:Score)` — the same round trip as a component read, answered
                // out of the same `&World` in the same gap between two runs of the VM, so it
                // costs no frame either. The name is the only argument: a resource belongs to no
                // entity.
                "resource.get" => {
                    let found = request
                        .text(0)
                        .and_then(|name| reflect_resource_of(scripts, &registry, name));
                    let holder = found
                        .as_ref()
                        .and_then(|(type_id, _)| resource_entity(world, *type_id))
                        .and_then(|e| world.get_entity(e).ok());
                    let value =
                        found.as_ref().zip(holder).and_then(|((_, rc), e)| rc.reflect(e));
                    match value {
                        Some(value) => scripts.answer_value(&request, |vm| {
                            reflect::reflect_to_ruby(
                                vm,
                                value.as_partial_reflect(),
                                entity_class,
                                ENTITY_TAG,
                            )
                        }),
                        None => scripts.answer(&request, Answer::Nil),
                    }
                }
                "component.has" => {
                    let rc = request
                        .text(1)
                        .and_then(|name| reflect_component_of(scripts, &registry, name));
                    let has = rc
                        .as_ref()
                        .zip(request.entity_arg(0).and_then(|e| world.get_entity(e).ok()))
                        .is_some_and(|(rc, entity)| rc.contains(entity));
                    scripts.answer(&request, Answer::Bool(has));
                }
                "components" => {
                    let mut names: Vec<String> = Vec::new();
                    if let Some(entity) = request.entity_arg(0).and_then(|e| world.get_entity(e).ok())
                    {
                        for registration in registry.iter() {
                            let Some(rc) =
                                registration.data::<bevy::ecs::reflect::ReflectComponent>()
                            else {
                                continue;
                            };
                            if rc.contains(entity) {
                                names.push(
                                    registration.type_info().type_path_table().short_path().to_string(),
                                );
                            }
                        }
                    }
                    // the registry is a map, so its order is not the same twice; a script that
                    // prints this wants it to read the same every run
                    names.sort();
                    scripts.answer_value(&request, |vm| {
                        let items: Vec<Value> =
                            names.iter().map(|n| vm.str_new(n.as_bytes())).collect();
                        vm.ary_new(items)
                    });
                }
                "entities.with" => {
                    let mut found: Vec<Entity> = Vec::new();
                    if let Some(rc) =
                        request.text(0).and_then(|name| reflect_component_of(scripts, &registry, name))
                    {
                        // every entity in the world: this is the lookup the rustdoc of
                        // `Rubevy.find` warns is not for every frame
                        for entity in world.iter_entities() {
                            if rc.contains(entity) {
                                found.push(entity.id());
                            }
                        }
                    }
                    scripts.answer_value(&request, |vm| {
                        let items: Vec<Value> = found
                            .iter()
                            .map(|e| vm.data_new(entity_class, ENTITY_TAG, e.to_bits()))
                            .collect();
                        vm.ary_new(items)
                    });
                }
                _ => scripts.answer(&request, Answer::Nil),
            }
            answered += 1;
            if out_of_time(deadline_ns) {
                // the rest of this round's questions go to the next frame, which takes them
                // before anything asked in it
                scripts.reflect_requests.extend(asked);
                break;
            }
        }
    }
    answered
}

/// Whether the tick has used up the frame time it was given, on the VM's own clock.
///
/// `None` is an app that set no [`ScriptWorld::frame_time`]: no deadline, and — which is the
/// point of the `is_some_and` rather than a comparison against `u64::MAX` — no call to the clock
/// at all, so nothing is charged to a game that does not use this.
fn out_of_time(deadline_ns: Option<u64>) -> bool {
    deadline_ns.is_some_and(|deadline| clock_ns() >= deadline)
}

/// Answers the questions the game registered an in-tick answerer for
/// ([`ScriptWorld::answer_in_tick`]), and says how many it answered. Called by [`tick_scripts`]
/// right after [`answer_reflect_requests`], in the same gap between two runs of the VM.
///
/// It looks in the same two places and in the same order as that one: the leftovers of the
/// previous frame ([`ScriptWorld::in_tick_requests`]) first, then what this round of the VM put
/// on the command queue, each in the order it was asked in. `deadline_ns` works exactly as it
/// does there — the clock after every answer, the rest of the round put off to the next frame,
/// and always at least one answer made — with one thing worth saying twice: **the closure is the
/// game's**, so the one answer a late tick still makes costs whatever that closure costs.
///
/// The two calls are counted apart on purpose: a tick that is already late makes one answer of
/// each kind, not one in all, so a game's in-tick questions are not starved by rubevy's own
/// (nor the other way round).
///
/// The map of answerers is moved out of `scripts` for the length of the call. That is not a
/// trick, it is the borrow: a closure is called with the world while the `Vm` beside it is being
/// answered into, and both live in `ScriptWorld`. Nothing can register an answerer meanwhile —
/// the closures are handed `&World` and a [`Request`], and `ScriptWorld<M>` is not in that world
/// while the tick holds it — so the map that goes back is the map that came out.
fn answer_in_tick_requests<M: 'static>(
    world: &World,
    scripts: &mut ScriptWorld<M>,
    deadline_ns: Option<u64>,
) -> usize {
    if scripts.in_tick_answerers.is_empty() {
        // nothing was registered, so nothing was sorted into `in_tick_requests` either
        return 0;
    }
    let answerers = std::mem::take(&mut scripts.in_tick_answerers);
    let mut asked = std::mem::take(&mut scripts.in_tick_requests);
    asked.append(&mut take_asks(&mut scripts.vm, |kind| answerers.contains_key(kind)));
    let mut answered = 0usize;
    let mut asked = asked.into_iter();
    while let Some(request) = asked.next() {
        // a question is only ever sorted into this road while its kind has an answerer, and an
        // answerer is never taken away, so the `None` arm is unreachable; it answers rather than
        // leaving a task parked for ever, which is what a lost answer would be
        let answer = match answerers.get(&request.kind) {
            Some(answerer) => answerer(world, &request),
            None => Answer::Nil,
        };
        scripts.answer(&request, answer);
        answered += 1;
        if out_of_time(deadline_ns) {
            scripts.in_tick_requests.extend(asked);
            break;
        }
    }
    scripts.in_tick_answerers = answerers;
    answered
}

/// Writes what the scripts put on components this frame (`entity[:Transform] = hash`).
///
/// It runs after [`drain_commands`], at the end of the frame, which is the promise `Commands`
/// makes and the one `Rubevy.spawn` already made: a script's write is seen by the next frame,
/// not in the middle of this one.
fn apply_component_writes<M: 'static>(world: &mut World) {
    if world.resource::<ScriptWorld<M>>().component_writes.is_empty() {
        return;
    }
    let writes = std::mem::take(&mut world.resource_mut::<ScriptWorld<M>>().component_writes);
    let Some(registry) = world.get_resource::<AppTypeRegistry>().cloned() else { return };
    let registry = registry.read();
    for write in writes {
        let Some(rc) = registry
            .get_with_short_type_path(&write.name)
            .or_else(|| registry.get_with_type_path(&write.name))
            .and_then(|r| r.data::<bevy::ecs::reflect::ReflectComponent>())
        else {
            warn!("rubevy: no registered component named {}", write.name);
            continue;
        };
        let Ok(mut entity) = world.get_entity_mut(write.entity) else { continue };
        let Some(mut value) = rc.reflect_mut(&mut entity) else {
            warn!("rubevy: {} has no {}", write.entity, write.name);
            continue;
        };
        if let Err(e) = reflect::apply_ruby(value.as_partial_reflect_mut(), &write.value) {
            warn!("rubevy: {} was not written whole: {e}", write.name);
        }
    }
}

/// Writes what the scripts put on resources this frame (`Rubevy.set_resource(:Score, hash)`).
///
/// The twin of [`apply_component_writes`], running right after it and making the same promise:
/// a script's write lands at the end of the frame, so a read after it in the same tick still
/// answers the old value. It is a system of its own rather than another loop inside that one
/// because the two find their target in different places — a component on the entity the script
/// named, a resource on the entity Bevy keeps it on ([`resource_entity`]) — and because a game
/// that reads the schedule should see the two named apart.
fn apply_resource_writes<M: 'static>(world: &mut World) {
    if world.resource::<ScriptWorld<M>>().resource_writes.is_empty() {
        return;
    }
    let writes = std::mem::take(&mut world.resource_mut::<ScriptWorld<M>>().resource_writes);
    let Some(registry) = world.get_resource::<AppTypeRegistry>().cloned() else { return };
    let registry = registry.read();
    for write in writes {
        let Some((type_id, rc)) = resource_data(&registry, &write.name) else {
            warn!("rubevy: no registered resource named {}", write.name);
            continue;
        };
        let Some(holder) = resource_entity(world, type_id) else {
            warn!("rubevy: there is no {} in this world", write.name);
            continue;
        };
        let Ok(mut holder) = world.get_entity_mut(holder) else { continue };
        let Some(mut value) = rc.reflect_mut(&mut holder) else {
            warn!("rubevy: {} is not where the world said it was", write.name);
            continue;
        };
        if let Err(e) = reflect::apply_ruby(value.as_partial_reflect_mut(), &write.value) {
            warn!("rubevy: {} was not written whole: {e}", write.name);
        }
    }
}

// ------------------------------------------------------------------ the Ruby side

/// The Ruby half of the host API, run when the VM starts: `Rubevy::Entity#[]` and the rest
/// (`src/prelude.rb`, compiled by `tools/compile_scripts.sh`).
const PRELUDE: &[u8] = include_bytes!("prelude.mrb");

/// What a `Rubevy::Entity` object is, in the `tag` of [`Vm::data_new`]. A host that gives its
/// scripts Data objects of its own picks other numbers.
pub const ENTITY_TAG: u32 = 1;

/// Defines `Rubevy::Entity`, the class of the object a script holds an entity in, and answers it.
///
/// It is a Data object (`Vm::data_new`): the VM carries `Entity::to_bits` and never reads
/// through it. `==`, `eql?` and `hash` go by that handle, so two objects naming the same entity
/// are equal and are one key in a Hash, while `dup` and `clone` refuse — which is the point of
/// the kind. What is defined here is `to_i` (the bits, for a script or a game that wants the
/// number) and `inspect`/`to_s`, so that `p entity` says which entity it is.
fn define_entity_class(vm: &mut Vm, module: ObjId) -> ObjId {
    // There is no `define_class_under` on the VM, so the class is made and named the way Ruby
    // does it: `Class.new(Object)` and `Rubevy.const_set(:Entity, it)`, which also gives the
    // anonymous class its name and its outer module (`Rubevy::Entity`).
    let new = vm.intern("new");
    let object = Value::Obj(vm.core.object);
    let class = Value::Obj(vm.core.class);
    let entity = vm.funcall(class, new, &[object], Value::Nil).expect("Class.new");
    let const_set = vm.intern("const_set");
    let name = Value::Sym(vm.intern("Entity"));
    vm.funcall(Value::Obj(module), const_set, &[name, entity], Value::Nil).expect("Rubevy::Entity");
    let entity = entity.obj().expect("a class is an object");
    vm.define_fn(entity, "to_i", |this: This<DataRef>| this.handle as i64);
    vm.define_fn(entity, "inspect", |this: This<DataRef>| entity_inspect(this.handle));
    vm.define_fn(entity, "to_s", |this: This<DataRef>| entity_inspect(this.handle));
    entity
}

fn entity_inspect(handle: u64) -> String {
    match entity_from_bits(handle) {
        Some(e) => format!("#<Rubevy::Entity {e}>"),
        None => format!("#<Rubevy::Entity bits={handle}>"),
    }
}

/// The entity an argument names: either a `Rubevy::Entity` object, or the Integer of
/// `Entity::to_bits` that the host API took before there was one (`Rubevy.despawn 12884901888`),
/// which still works.
fn entity_arg(vm: &mut Vm, v: Option<&Value>) -> Result<u64, VmError> {
    match v {
        Some(v) => match vm.data_of(*v) {
            Some((ENTITY_TAG, handle)) => Ok(handle),
            _ => Ok(vm.expect_int(*v, "entity")? as u64),
        },
        None => Ok(0),
    }
}

/// Defines the `Rubevy` module and its methods, and answers the `Rubevy::Entity` class.
///
/// The methods are closures ([`Vm::define_closure`]) rather than bare function pointers: they
/// carry the entity class, which is what lets `Rubevy.entity` and `Rubevy.ask` build and read
/// entity objects. What they may not carry is the game — a native gets `&mut Vm` and nothing
/// else — so everything that touches the world is left on the queue in the VM's host state
/// ([`HostState`]) for [`drain_commands`].
///
/// `release` is the other end of [`ScriptWorld::release_dropped_values`]: `Rubevy.ask` gives it
/// to every [`RootedValue`] it makes, so that dropping one reaches the sweep.
fn install_host_api(vm: &mut Vm, release: ReleaseQueue) -> ObjId {
    let m = vm.define_module("Rubevy");
    let entity_class = define_entity_class(vm, m);
    let sc = vm.singleton_class(Value::Obj(m)).expect("Rubevy singleton");
    vm.define_closure(sc, "log", |vm, _s, a, _b| {
        let text = match a.first() {
            Some(v) => String::from_utf8_lossy(&vm.as_string(*v)?).into_owned(),
            None => String::new(),
        };
        push_command(vm, HostCommand::Log(text));
        Ok(Value::Nil)
    });
    vm.define_closure(sc, "spawn", |vm, _s, a, _b| {
        let name = match a.first() {
            Some(v) => String::from_utf8_lossy(&vm.as_string(*v)?).into_owned(),
            None => String::new(),
        };
        let (x, y, z) = (num(vm, a.get(1)), num(vm, a.get(2)), num(vm, a.get(3)));
        push_command(vm, HostCommand::Spawn { name, x, y, z });
        Ok(Value::Nil)
    });
    vm.define_closure(sc, "despawn", |vm, _s, a, _b| {
        let bits = entity_arg(vm, a.first())?;
        push_command(vm, HostCommand::Despawn(bits));
        Ok(Value::Nil)
    });
    // the entity this script is attached to, as a `Rubevy::Entity`. A fresh object each call:
    // two of them for the same entity are `==` and hash alike, so a script cannot tell, and the
    // ones it drops are what the free hook sees
    vm.define_closure(sc, "entity", move |vm, _s, _a, _b| {
        Ok(match current_entity(vm) {
            Value::Int(bits) => vm.data_new(entity_class, ENTITY_TAG, bits as u64),
            _ => Value::Nil,
        })
    });
    vm.define_closure(sc, "move_to", |vm, _s, a, _b| {
        // the entity the script is attached to, which is the common case
        let Value::Int(bits) = current_entity(vm) else { return Ok(Value::Nil) };
        let (x, y, z) = (num(vm, a.first()), num(vm, a.get(1)), num(vm, a.get(2)));
        push_command(vm, HostCommand::SetPosition { entity: bits as u64, x, y, z });
        Ok(Value::Nil)
    });
    // `Rubevy.ask("scan", 40)` — the game answers it, this frame or a later one, and the script
    // waits on the queue meanwhile (its task is parked, so it costs nothing)
    vm.define_closure(sc, "ask", move |vm, _s, a, _b| {
        let kind = match a.first() {
            Some(v) => String::from_utf8_lossy(&vm.as_string(*v)?).into_owned(),
            None => return Err(vm.raise_arg("ask needs what to ask for")),
        };
        let mut args: Vec<Arg> = Vec::with_capacity(a.len().saturating_sub(1));
        for v in &a[1..] {
            // an entity the script was given keeps its identity across the boundary; everything
            // else is a number or a string, as before
            let as_entity = match vm.data_of(*v) {
                Some((ENTITY_TAG, handle)) => entity_from_bits(handle),
                _ => None,
            };
            if let Some(e) = as_entity {
                args.push(Arg::Entity(e));
                continue;
            }
            // a String, and only a String: the fast path copies the bytes out here, so the host
            // has them without the VM
            if let Some(bytes) = vm.str_bytes(*v) {
                args.push(Arg::Text(String::from_utf8_lossy(bytes).into_owned()));
                continue;
            }
            args.push(match v {
                Value::Int(i) => Arg::Num(*i as f64),
                Value::Float(f) => Arg::Num(*f),
                Value::Sym(s) => Arg::Text(vm.sym_name(*s).to_string()),
                // a Hash, an Array, an object of the script's own, `nil`: carried as the value
                // it is, rather than through its `to_s` — which is what this used to do, and
                // which threw away everything a structure was for
                other => Arg::Value(RootedValue::new(vm, *other, &release)),
            });
        }
        let queue = vm.task_queue_new()?;
        vm.gc_register(queue);
        let entity = match current_entity(vm) { Value::Int(bits) => bits as u64, _ => u64::MAX };
        push_command(vm, HostCommand::Ask { entity, kind, args, queue });
        Ok(Value::Obj(queue))
    });
    // `entity[:Transform] = hash` goes through here (`src/prelude.rb`). It is a command rather
    // than a question: the script does not wait for it, and the write lands with the frame's
    // other commands. The Hash is read out of the VM now, while there is a `&mut Vm`;
    // `apply_component_writes` has the world but no VM.
    vm.define_closure(sc, "set_component", |vm, _s, a, _b| {
        let bits = entity_arg(vm, a.first())?;
        let name = match a.get(1) {
            Some(v) => String::from_utf8_lossy(&vm.as_string(*v)?).into_owned(),
            None => return Err(vm.raise_arg("set_component needs the name of a component")),
        };
        let value = reflect::read_ruby(vm, a.get(2).copied().unwrap_or(Value::Nil), ENTITY_TAG)?;
        push_command(vm, HostCommand::SetComponent { entity: bits, name, value });
        Ok(Value::Nil)
    });
    // `Rubevy.set_resource(:Score, {points: 10})`, the resource half of the same thing. It is a
    // native and not Ruby for the same reason `set_component` is: a write does not wait for an
    // answer, so nothing here has to be parked. (The *read* is in `src/prelude.rb`, because that
    // one does.) A Symbol is taken as its name, as `subscribe` takes one.
    vm.define_closure(sc, "set_resource", |vm, _s, a, _b| {
        let name = match a.first() {
            Some(Value::Sym(s)) => vm.sym_name(*s),
            Some(v) => String::from_utf8_lossy(&vm.as_string(*v)?).into_owned(),
            None => return Err(vm.raise_arg("set_resource needs the name of a resource")),
        };
        let value = reflect::read_ruby(vm, a.get(1).copied().unwrap_or(Value::Nil), ENTITY_TAG)?;
        push_command(vm, HostCommand::SetResource { name, value });
        Ok(Value::Nil)
    });
    // `hits = Rubevy.subscribe(:hit)` — a `Task::Queue` the game pushes messages onto
    // (`ScriptWorld::publish`). It is the same kind of queue `Rubevy.ask` answers on, so a
    // script waits on it the same way, in this task or in one of its own.
    //
    // `Rubevy.subscribe(:belt, limit: 512)` says how many messages this one queue holds before
    // the oldest is dropped, where the VM's own `ScriptWorld::queue_limit` is not what this
    // stream wants. Keywords reach a native as a trailing Hash (SabiRuby's `native_call_args`),
    // so that is what is read here.
    vm.define_closure(sc, "subscribe", move |vm, _s, a, _b| {
        let name = match a.first() {
            Some(Value::Sym(s)) => vm.sym_name(*s),
            Some(v) => String::from_utf8_lossy(&vm.as_string(*v)?).into_owned(),
            None => return Err(vm.raise_arg("subscribe needs what to listen for")),
        };
        let limit = subscribe_limit(vm, a.get(1).copied())?;
        // A subscription belongs to an entity's script: that is who a message addressed to an
        // entity reaches, and it is what says when to let the queue go. A task a script made
        // with `Task.new` has the entity of the task that made it (`src/prelude.rb`), so it may
        // subscribe for itself; what is left here is a task with no entity at all — one the
        // host spawned outside a `Script`, or one whose `@rubevy_entity` was cleared.
        let entity = match current_entity(vm) {
            Value::Int(bits) => entity_from_bits(bits as u64),
            _ => None,
        };
        let Some(entity) = entity else {
            return Err(vm.raise_arg("subscribe from a task that has an entity (@rubevy_entity)"));
        };
        let queue = vm.task_queue_new()?;
        vm.gc_register(queue);
        // This queue ends: `ScriptWorld::unsubscribe` closes it, and a closed `Task::Queue`
        // answers `pop` with nil for ever. `Rubevy::Subscription` (`src/prelude.rb`) is what
        // turns that nil into `Rubevy::Unsubscribed`, so a task waiting here unwinds — runs its
        // `ensure` and ends — rather than spinning on a queue nothing will fill again. It is
        // extended into this one object, so an `Rubevy.ask` queue keeps the gem's own meaning.
        let subscription = vm.intern("Subscription");
        if let Some(module) = vm.const_get(m, subscription) {
            let extend = vm.intern("extend");
            vm.funcall(Value::Obj(queue), extend, &[module], Value::Nil)?;
        }
        match vm.host_state_mut::<HostState>() {
            Some(state) => {
                let seq = state.subscriptions_made;
                state.subscriptions_made += 1;
                // filed under the name, at the end of what is already listening for it: within
                // one name, the order here is the order `publish` delivers in
                state
                    .subscriptions
                    .entry(name)
                    .or_default()
                    .push(Subscription { entity, queue, seq, limit });
            }
            // no host state is no plugin; let go of the queue rather than leave it rooted
            None => vm.gc_unregister(queue),
        }
        Ok(Value::Obj(queue))
    });
    vm.define_closure(sc, "set_position", |vm, _s, a, _b| {
        let bits = entity_arg(vm, a.first())?;
        let (x, y, z) = (num(vm, a.get(1)), num(vm, a.get(2)), num(vm, a.get(3)));
        push_command(vm, HostCommand::SetPosition { entity: bits, x, y, z });
        Ok(Value::Nil)
    });
    entity_class
}

/// The `limit:` of a `Rubevy.subscribe`, or `None` where the script did not ask for one and the
/// VM's [`ScriptWorld::queue_limit`] is what it gets.
///
/// `arg` is whatever followed the name. A native is handed its caller's keywords as a trailing
/// Hash, so `Rubevy.subscribe(:belt, limit: 512)` and `Rubevy.subscribe(:belt, {limit: 512})`
/// arrive here as the same thing — and so does a positional Hash, which is why anything that is
/// not a Hash is refused rather than ignored.
///
/// Everything refused is an `ArgumentError` raised from the script's own frame, so a script that
/// rescues it reads the line it wrote in its `backtrace` — where the program was compiled with a
/// line table (`mrbc -g`, `sabiruby_compiler::Options::debug_info`; without one a backtrace has
/// nothing to say, which is not this call's doing). A limit below one is refused because
/// the queue would then hold only the newest message; a keyword that is not `limit:` is refused
/// because Ruby refuses one, and because a silently ignored `limt:` is a subscription that
/// quietly keeps the default.
fn subscribe_limit(vm: &mut Vm, arg: Option<Value>) -> Result<Option<usize>, VmError> {
    let Some(arg) = arg else { return Ok(None) };
    let Some(entries) = vm.hash_entries(arg) else {
        return Err(vm.raise_arg("subscribe takes the name to listen for and keywords (limit:)"));
    };
    let mut limit = None;
    for (key, value) in entries {
        let name = match key {
            Value::Sym(s) => vm.sym_name(s),
            other => match vm.str_bytes(other) {
                Some(b) => String::from_utf8_lossy(b).into_owned(),
                None => String::from("?"),
            },
        };
        if name != "limit" {
            return Err(vm.raise_arg(&format!("subscribe: unknown keyword: {name}")));
        }
        match value {
            // one and up: a queue that holds nothing would drop every message but the newest,
            // which is not what a script asking for a small queue means
            Value::Int(n) if n >= 1 => limit = Some(n as usize),
            _ => {
                let shown = vm.inspect_str(value).unwrap_or_else(|_| String::from("?"));
                return Err(vm.raise_arg(&format!(
                    "subscribe: limit: takes a whole number of messages, one or more, not {shown}"
                )));
            }
        }
    }
    Ok(limit)
}

/// The entity of the task the scheduler is running, as `Entity::to_bits`.
fn current_entity(vm: &Vm) -> Value {
    match vm.task_running() {
        Some(task) => vm.ivar_get(task, ENTITY_IVAR),
        None => Value::Nil,
    }
}

fn num(vm: &Vm, v: Option<&Value>) -> f32 {
    let _ = vm;
    match v {
        Some(Value::Int(i)) => *i as f32,
        Some(Value::Float(f)) => *f as f32,
        _ => 0.0,
    }
}

/// So the type is named in the public API even where nothing else uses it.
pub type ScriptVmError = VmError;

/// The `Rubevy::Entity` object for an entity, built inside a closure that has the `Vm`
/// ([`ScriptWorld::answer_value`], [`ScriptWorld::publish_value`]).
///
/// `class` is [`ScriptWorld::entity_class`], read before the closure runs:
///
/// ```no_run
/// # use bevy::prelude::*;
/// # use rubevy::{entity_object, ScriptWorld};
/// # use sabiruby::Value;
/// fn hit(world: &mut ScriptWorld, who: Entity, hurt: Entity) {
///     let class = world.entity_class();
///     world.publish_value(Some(hurt), "hit", move |vm| {
///         let who = entity_object(vm, class, who);
///         vm.ary_new(vec![who, Value::Float(2.5)])
///     });
/// }
/// ```
pub fn entity_object(vm: &mut Vm, class: ObjId, entity: Entity) -> Value {
    vm.data_new(class, ENTITY_TAG, entity.to_bits())
}

/// The entity ids the plugin hands to Ruby are `Entity::to_bits`; this is the
/// way back, for a host that wants to read what a script asked about.
pub fn entity_of(bits: u64) -> Option<Entity> {
    entity_from_bits(bits)
}

/// What a script asked for this frame and has not had carried out yet, for tests: the queue is
/// drained by [`drain_commands`], so this is only useful before that system runs.
#[doc(hidden)]
pub fn pending_command_count<M: 'static>(world: &ScriptWorld<M>) -> usize {
    world.vm.host_state::<HostState>().map(|s| s.commands.len()).unwrap_or(0)
}
