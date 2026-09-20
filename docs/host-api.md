# What a script can say to the game, and the game to a script

The Ruby side of rubevy is small on purpose: a script is a task in a VM (`docs/outlook.ja.md`) —
one VM for the whole app unless the app asks for more ("Two VMs in one app", below) — and
everything it does to the world goes through `Rubevy`. This file is the current surface and the
rules behind it.

## From the script to the world (one way)

A native cannot touch Bevy's `World` — it gets `&mut Vm` and nothing else — so these leave a
command behind and the `drain_commands` system carries it out later in the same frame.

| Ruby | What happens |
|---|---|
| `Rubevy.log "text"` | `info!` through Bevy's log |
| `Rubevy.spawn "name", x, y, z` | an entity with `SpawnedByScript { name }` and a `Transform` |
| `Rubevy.despawn entity` | that entity is despawned |
| `Rubevy.entity` | the entity this script is attached to, as a `Rubevy::Entity`, or nil |
| `Rubevy.move_to x, y, z` | the script's own entity is moved |
| `Rubevy.set_position entity, x, y, z` | any entity is moved |
| `entity[:Transform] = hash` | the fields the Hash names are written over that component |
| `Rubevy.set_resource :Score, hash` | the fields the Hash names are written over that resource |

## What an entity is, on the Ruby side

`Rubevy.entity` — and any [`Answer::Entity`] the game sends back — is a `Rubevy::Entity`: an
object whose *handle* is `Entity::to_bits`, which the VM carries and never reads through
(SabiRuby's `Vm::data_new`). What a script can do with one:

```ruby
e = Rubevy.entity
e.to_i                     # 4294967294 — Entity::to_bits, exactly
e == Rubevy.entity         # true: two objects for one entity are equal, and are one Hash key
e.inspect                  # "#<Rubevy::Entity 1v0>" (Bevy's own name for it)
Rubevy.despawn e           # and it can be handed back
Rubevy.ask("look_at", e)   # → Arg::Entity in the Request, read with `entity_arg(0)`
```

It is not an Integer, which is the point: `Rubevy.despawn 3` used to name somebody, and an
entity that had been past an `f64` (`Answer::Num`) lost its low bits once a generation went
past 2^21. `dup` and `clone` raise, so a handle is not copied behind the host's back.
`Rubevy.despawn` and `Rubevy.set_position` still take the Integer form as well, which is what a
script gets from an `Answer::Num`, `List` or `Rows` — a table of entities is still a table of
numbers, and `entity_of(bits)` is the way back on the Rust side.

## From the script to the game and back (`Rubevy.ask`)

`Rubevy.ask` is the shape everything asynchronous takes: a sensor reading, a path request, a file
the game has to load, an answer another system computes. It returns a `Task::Queue`; `pop` parks
the script's task until the game pushes an answer.

```ruby
found = Rubevy.ask("scan", 40.0).pop     # parked here; the other scripts keep running
Rubevy.move_to found[0], found[1], 0.0 if found
```

The game answers in a system of its own, **put in `RubevySet::Answer`** (the next section says
what that buys):

```rust
app.add_plugins(RubevyPlugin::default())
    .add_systems(Update, answer_requests.in_set(RubevySet::Answer));

fn answer_requests(mut world: ResMut<ScriptWorld>, /* whatever the answer needs */) {
    for request in world.take_requests() {          // Request { entity, kind, args, queue }
        let answer = Answer::List(vec![3.0, 3.5]);  // Nil / Bool / Num / Text / List / Rows / Entity
        world.answer(&request, answer);             // this frame, or keep it for a later one
    }
}
```

`examples/sensor.rs` does exactly that, answering two frames after the question to show that a
request may outlive the frame it was made on.

**Why a queue rather than a callback or a Future.** The VM already has a scheduler (mruby-task)
whose tasks can wait: on a deadline (`sleep`), on another task (`join`), or on a queue. An
asynchronous host call is just one more thing to wait on, so it needs no second scheduler, no
`await` keyword, and nothing in the script that looks asynchronous. A parked task costs nothing —
it is not polled, not woken per frame, and the instruction budget goes to the scripts that can
actually run.

**Rules.**

* Answer each request once. `ScriptWorld::answer` lets go of the queue (`gc_unregister`) as it
  answers, so a second answer to the same request is a mistake.
* A request you cannot answer yet is yours to keep; nothing expires. If the script should not wait
  forever, it can say so on the Ruby side: `Rubevy.ask(…).pop(timeout_ms: 500)` answers nil when
  the deadline passes.
* `kind` is a string; the arguments after it are numbers, strings, entities, or any other Ruby
  value ([`Arg`]), and `Request::num(i)` / `Request::text(i)` / `Request::entity_arg(i)` /
  `Request::value(i)` read them — each answers `None` for an argument of another sort. An answer
  is nil, a bool, a number, a string, an entity, a list of numbers, or a table of them
  ([`Answer::Rows`] — every robot with its team and hp, say), or anything the host builds in the
  VM itself (`answer_value`). The question's side is described below.
* A system is one of three places an answer can come from, and the usual one. The other two are
  a future (`answer_with`, for work that does not fit in a frame) and a closure called inside the
  tick (`answer_in_tick`, for a question asked every frame that the script cannot go on without
  — it costs no frame at all). Both have a section of their own below.

## Where the game's systems go in the frame (`RubevySet`)

`RubevyPlugin` puts its own systems into three public sets, in this order inside `Update`:

| set | what is in it | what it is for |
|---|---|---|
| `RubevySet::Deliver` | `start_scripts`, `deliver_answers`, the release sweep | what arrived between the frames reaches the VM before a script runs |
| `RubevySet::Tick` | `tick_scripts` (exclusive), `drain_commands`, `apply_component_writes` | the scripts run — and the component reads they make, with the kinds the game registered with `answer_in_tick`, are answered while they are still running — and what they asked the *game* for otherwise becomes a `Request` |
| `RubevySet::Answer` | **the game's answering systems** | the questions this frame asked are answered before the frame ends |

```rust
app.add_systems(Update, answer_requests.in_set(RubevySet::Answer));
```

`tick_scripts` is an **exclusive** system (`&mut World`). That is what lets it answer a script's
component reads while the scripts are still running ("A read costs no frame", below), and it has
one consequence worth knowing: the `Tick` of one VM and the `Tick` of another cannot overlap, so
with a second VM the two ticks are **serial** even though everything else about the two is
separate ("Two VMs in one app").

**What it buys: a round trip costs one frame.** This is about the questions the *game* answers;
the four rubevy answers itself cost no frame at all. The `Rubevy.ask` happens in `Tick` and
leaves a command behind, `drain_commands` (still `Tick`) turns it into a `Request`, the game
answers it in `Answer`, and the script wakes in the next frame's `Tick`. `tests/scheduling.rs` measures this
from the script's own side — six `Rubevy.ask(…).pop` round trips, each one exactly one
`$rubevy[:frame]` apart.

**What it costs not to use it.** A system ordered against nothing lands wherever Bevy's executor
puts it, which almost everywhere is fine: before the sets, after them, in `PreUpdate`, in
`PostUpdate` — anything that answers before the *next* `Tick` is soon enough. The one window that
is not fine is **between `tick_scripts` and `drain_commands`**: a system there misses the question
this frame asked (it becomes a `Request` a moment later) and answers the previous frame's too late
for this frame's scripts, so every round trip costs **two** frames. SabiRuby Battle was in exactly
that window before the sets existed and measured 2.0 frames for every question the game answered,
against 1.0 for every question rubevy answered itself (rubevy_games,
`docs/worklog/2026-09-16-showpieces-d2-d3.md`). The point of the set is not that two frames was a
rule — it is that where the system landed was luck, and now it is not.

Both `Deliver` and `Answer` are outside that window by construction. `Answer` is the one to use:
it is the only placement where a system sees the question **on the frame it was asked**, so an
answer may depend on the rest of that frame (positions after `move_robots`, events of this frame,
whatever the game has just worked out). A system in `Deliver` also costs one frame, but it is
always answering the previous frame's questions. Do not put an answering system *inside*
`RubevySet::Tick` — that is the one set that contains the window.

The sets also give a game the other orderings it may want: `.before(RubevySet::Tick)` for a system
that must have moved the world before the scripts look at it, `.after(RubevySet::Answer)` for one
that runs on what the scripts did this frame. `.before(RubevySet::Tick)` is the one that has
grown teeth: the world it leaves behind is the world a component read sees and the world an
`answer_in_tick` closure is handed ("Answering inside the tick").

**A question the game answers in no frame at all.** A kind registered with
`ScriptWorld::answer_in_tick` is not answered in `Answer` but inside `Tick`, by a closure the
loop calls between two runs of the VM. It is the road for the questions asked every frame by
every script — a spatial lookup — and the trade is that such a closure sees the world as it
stands in `Tick` rather than the finished frame. The section "Answering inside the tick" says
when it is worth it and when a system is still the right place.

## What a question may carry (`Arg`)

Three sorts of argument are copied out of the VM at the call and cost the host nothing
afterwards:

| Ruby | `Arg` | read with |
|---|---|---|
| Integer, Float | `Arg::Num(f64)` | `request.num(i)`, `request.num_or(i, d)` |
| String, Symbol | `Arg::Text(String)` | `request.text(i)` |
| `Rubevy::Entity` | `Arg::Entity(Entity)` | `request.entity_arg(i)` |

Everything else — a Hash, an Array, an object of the script's own, `nil`, `true`, `false` — is
`Arg::Value`, which is the Ruby value **itself**, not a copy of it:

```ruby
Rubevy.ask("path", { from: [1.0, 2.0], to: [8.0, 3.0], avoid: ["lava"] }).pop
```

```rust
for request in world.take_requests() {
    // sabiruby's `FromRuby`, for anything that trait knows: Vec<f64>, Vec<String>, f64, …
    let waypoints: Vec<f64> = request.arg_as(&mut world.vm, 0).unwrap_or_default();

    // a Hash has no `FromRuby` in the VM, so it is read entry by entry
    let h = request.value(0).expect("a value");
    for (k, v) in world.vm.hash_entries(h).unwrap_or_default() {
        let name = world.vm.as_string(k).unwrap_or_default();
        // … `f64::from_ruby(&mut world.vm, v)`, `vm.ary_vals(v)`, `vm.hash_entries(v)`
    }
}
```

**Nothing inside is flattened.** An argument is one `Arg`, and which one it is depends on that
argument alone: `Rubevy.ask("path", [1.0, 2.0], 3)` is an `Arg::Value` and an `Arg::Num`, and the
numbers inside the Array stay Ruby numbers inside a Ruby Array (`request.num(0)` is `None`). The
alternative — walking the structure and turning each leaf into an `Arg` — would lose the shape,
which is the only thing the structure was for.

**Why the value rather than a copy.** A component write (`entity[:Transform] = hash`) *is* copied
out, because the write happens a system later, with a `&mut World` and no VM in sight. A question
is the other way round: the host has the `ScriptWorld`, so it has the `Vm`, and copying a nested
value out would mean a second and poorer `Answer` — the same argument `answer_value` makes on the
way back. Before this, a Hash arrived as `Arg::Text` of its `to_s`, which threw away everything a
structure was for.

**How it is kept alive, and let go of.** The value is a value nothing in the VM points at any
more — the script has parked on its queue, the frame that built the literal may have returned —
so it lives only because `Vm::gc_register` says so. That registration is made once, in the
`Rubevy.ask` native, and is owned by an `Arc` inside the `Request`. When the last `Request` naming
it is dropped — answered and let go, dropped unanswered, lost with a system that panicked — the
`Drop` puts the object on a queue and the plugin unregisters it at the head of the next frame
(`ScriptWorld::release_dropped_values`, which a host driving `ScriptWorld` without the plugin's
systems calls itself).

So a game has nothing to remember. There is no `release(request)` to forget, `Request` stays
`Clone` and may be kept for as many frames as the answer takes, and the only way to hold a value
for ever is to hold a `Request` for ever. The cost of the two-step release is that a value
outlives its request by up to one frame, which is the safe side of the mistake.

`tests/ask_value.rs` checks all of it against a real collection: a Hash and an Array asked for
inside a method (so the script itself holds nothing), a `GC.start` in the script and a
`Vm::gc_collect` between `take_requests` and the answer, and the live count falling by the size
of the structure once the request is dropped. Without the registration the same test says
`access to freed object`.

## Answering with an object of the game's own (`answer_value` + `RubyClass`)

`Answer` is flat — numbers, a string, a list, a table — and `ScriptWorld::answer_value(request,
|vm| …)` is the way past it: the host is handed the `&mut Vm` and builds whatever it likes. That
includes a **`Data` object over a Rust value**, which is what sabiruby's `#[derive(RubyClass)]` /
`#[ruby_methods]` make:

```rust
#[derive(RubyClass)]
struct Genome { speed: f64, colour: String }

#[ruby_methods]
impl Genome {
    fn speed(&self) -> f64 { self.speed }
    fn colour(&self) -> String { self.colour.clone() }
    fn mutate(&mut self, by: f64) { self.speed += by; }
}

// once, at Startup: the class and its methods
fn install(mut world: ResMut<ScriptWorld>) { Genome::register(&mut world.vm).unwrap(); }

// and then any answer may be one
world.answer_value(&request, |vm| Genome { speed: 2.5, colour: "blue".into() }.into_ruby(vm));
```

```ruby
g = Rubevy.ask("genome").pop
g.colour            # "blue"
g.mutate(0.5)       # changes the Rust value in place
g.speed             # 3.0
```

Nothing had to be added to rubevy for this, and nothing is added by it: the value lives in a
`HostStore` the **VM** carries, found by its Rust `TypeId`, and the VM drops it when the Ruby
object naming it is collected. That is a different place from the two host mechanisms rubevy
itself uses — `Vm::set_host_state` (one value, rubevy's command queue) and `Vm::set_on_free`
(rubevy's own hook, counting `Rubevy::Entity`s) — so a game's classes and rubevy's do not
compete for either. `tests/host_data.rs` checks exactly that: the answer arrives as the game's
class with the game's methods, rubevy's free hook does not fire for it, and a `gc_collect` after
the script has ended takes the `Genome` out of the store.

`sabiruby = { features = ["macros"] }` is what brings the two macros in. It is a dev-dependency
here, for that test: rubevy defines no Ruby class of its own beyond `Rubevy::Entity`, which is
`Vm::data_new` by hand.

**The rules of `answer_value` still hold.** The closure runs inside the call, answer each request
once, and nothing in the closure may park a task — it is host code, not a script.

## Making the answer later (`answer_with`)

Where the answer is work rather than a lookup — a path search, a file, a query, anything that
does not fit in a frame — hand the request and a future to `ScriptWorld::answer_with` instead
of answering on the spot:

```rust
fn answer_paths(mut world: ResMut<ScriptWorld>) {
    for request in world.take_requests() {
        let to = (request.num_or(0, 0.0), request.num_or(1, 0.0));
        world.answer_with(request, async move { Answer::Rows(search(to).await) });
    }
}
```

The future goes to Bevy's `AsyncComputeTaskPool`, and a system of the plugin answers the request
on the frame it finishes. The request is the one `take_requests` gave you, handed over as it is;
it is answered once, there, so do not answer it again yourself. `ScriptWorld::answering()` says
how many are still out.

Nothing changes for the script: it is parked on its queue from the `Rubevy.ask` until the answer
arrives, exactly as when a system keeps the request for three frames, and the other scripts keep
running. `examples/async.rs` asks on frame 0 and is answered on frame 6, with the ticker beside
it running on 3 and 5.

The plugin picks the finished futures up at the head of the frame, before the scripts run, rather
than at the end beside `drain_commands`: a future finishes whenever its thread is done, and most
of a frame's wall clock is outside the schedule, so an answer that arrived in that gap wakes its
script on this frame instead of the next one.

**Threads are the app's choice, not this plugin's.** Bevy's task pools are threads only in a
build with bevy's `multi_threaded` feature. Without it they are a single-threaded fallback on the
main thread, where a future that only computes is driven to its end inside `answer_with` — the
frame waits for it — while a future waiting on something else (a channel, an IO completion, a
waker of your own) still parks and is picked up later. An app that wants the work off the main
thread puts `multi_threaded` in the features of its own `bevy` dependency; this crate's
`dev-dependencies` do that for the example and the tests. Either way the app needs Bevy's
`TaskPoolPlugin`, which `MinimalPlugins` and `DefaultPlugins` both add — `answer_with` panics
without it, because the pool it spawns on does not exist.

## Answering inside the tick (`answer_in_tick`)

A component read costs no frame because the tick answers it while the scripts are still running
("A read costs no frame", below). `ScriptWorld::answer_in_tick` is the game's way into that same
loop: a kind of `Rubevy.ask` answered by a closure rather than by a system, called between two
runs of the VM with the world as it stands in `RubevySet::Tick`.

```rust
fn install_answers(mut scripts: ResMut<ScriptWorld>) {          // a Startup system
    scripts.answer_in_tick("nearest", Box::new(|world: &World, request: &Request| {
        let Some(from) = request.entity_arg(0).and_then(|e| world.get::<Transform>(e))
        else { return Answer::Nil };
        let mut best: Option<(Entity, f32)> = None;
        for entity in world.iter_entities() {
            let (Some(_), Some(at)) = (entity.get::<Plant>(), entity.get::<Transform>())
            else { continue };
            let d = at.translation.distance(from.translation);
            if best.is_none_or(|(_, so_far)| d < so_far) { best = Some((entity.id(), d)); }
        }
        best.map_or(Answer::Nil, |(entity, _)| Answer::Entity(entity))
    }));
}
```

```ruby
plant = Rubevy.ask("nearest", Rubevy.entity).pop   # back in this line, on this frame
at    = plant[:Transform][:translation]            # and so is the read that follows it
```

`examples/nearest.rs` runs exactly that, with an ordinary answering system beside it for the
contrast — the script prints how long it waited for each, and it is `0` for `nearest` and `1` for
the question the system answers.

**When it is worth it.** When the question is asked every frame, for every creature, and the
script cannot go on without the answer. The world script a garden of creatures wants asks "what
is within four metres" and "who is nearest" once per creature per frame; answered by a system
that is one frame of lag per decision, and answered from Ruby with `Rubevy.find` plus a read per
candidate it is the frame's whole instruction budget (a read is ~75 instructions, and 24
creatures against 90 plants is 2,160 of them). A closure that walks the same world in Rust
answers all of it inside the tick.

**When not to.** Two cases, and both of them are the other two roads:

* **The answer depends on the rest of the frame** — what the physics worked out, what another
  system decided, an event of this frame. The closure runs in the middle of `Tick`, so it cannot
  see any of that; a system in `RubevySet::Answer` can, and costs one frame.
* **The answer is work rather than a lookup** — a path search, a file, anything that does not fit
  in the frame. That is `answer_with` and a future.

And never for changing the world: the closure is handed `&World`, so the compiler says so. What a
game means by it is `Rubevy.spawn` and `e[:Hp] =`, which land at the end of the frame as they
always did.

**What the closure is handed, and what it is not.** `&World` and the `Request`. Ordinary Bevy
reads work on that world — `world.get::<T>(entity)`, `world.iter_entities()`, a resource of the
game's own, `AppTypeRegistry` — and the game's own types need no reflection here, because the
game knows them. What is *not* in that world is `ScriptWorld<M>` itself: the tick has taken it
out for the length of its loop, so `world.get_resource::<ScriptWorld<M>>()` is `None` and there
is no `Vm` to reach. That is why the answer is the flat `Answer` (`Nil` / `Bool` / `Num` /
`Text` / `List` / `Rows` / `Entity`) and there is no `answer_value` here: building a value inside
the VM while the VM is being run around you is a different thing, and it can be added the day
something needs it. An argument that is a Hash or an Array can be seen for what it is
(`Arg::Value`, `request.value(0)`) but not read into, for the same reason — reading one needs the
`Vm`.

**The rules.**

* A registered kind never reaches `take_requests`, exactly as rubevy's own four do not. A game's
  answering system sees only what is left for it.
* The four rubevy answers itself (`component.get`, `component.has`, `components`,
  `entities.with`) cannot be taken over this way: they are taken off the queue first.
* Registering the same kind twice keeps the later closure and warns.
* The questions are answered in the order they were asked in, and the loop ends the way it always
  did — when a round answers nobody, or when the frame's budget or its `frame_time` is spent. A
  question left over from a spent frame is answered first on the next one.
* **The frame time ends the answering too, and the closure is on the inside of it.** The tick
  looks at the clock after every answer, so a round of a hundred questions to a closure that
  takes a hundred microseconds each stops where `frame_time` says and the rest are answered on
  the next frame, in order ("A read costs no frame, except at the tail of a frame that ran out
  of time", below). Two things follow for a game's closure. It is charged to the frame it runs
  in, so a closure that takes longer than `frame_time` by itself makes a tick that passes
  `frame_time` by that much — a tick always makes at least one answer, or a run of the VM that
  used the whole frame time would leave every task parked for ever. And the game's kinds and
  rubevy's own are counted apart: a late tick makes one answer of each, so neither road starves
  the other.
* `Rubevy::Proxy` is sugar for `Rubevy.ask`, so `garden.nearest(:Plant)` becomes synchronous by
  registering `"garden.nearest"`; the Ruby side does not change.
* What the closure costs comes out of the scripts' own frame, since it runs in the middle of it.

## Components by name

A script reads a component as a Hash of its fields and writes one back by naming the fields it
means. Nothing in rubevy knows what a `Transform` is: the plugin walks whatever Bevy's type
registry holds, through `ReflectComponent`, so a game's own component works the same way.

```ruby
e  = Rubevy.entity
tf = e[:Transform]              # {translation: [x, y, z], rotation: [x, y, z, w], scale: [...]}
tf[:translation][0] += 1.0
e[:Transform] = tf              # applied after this frame's scripts have run

e.has?(:Velocity)               # true / false
e.components                    # ["Transform", "Sprite", ...] — the short type names, sorted
Rubevy.find(:Npc)               # every entity with that component, as Rubevy::Entity objects
```

**What a type has to do to be there.** `#[derive(Reflect)]`, `#[reflect(Component)]`, and
`app.register_type::<T>()`. A type nobody registered is not an error on the Ruby side: the
component reads as `nil`, `has?` answers `false`, and it is not in `components`.

**Where Bevy's own types come from is a feature, not a plugin** (checked against bevy 0.19.1 on
2026-09-20). `bevy_transform`, `bevy_camera`, `bevy_sprite`, `bevy_ui`, `bevy_input` and
`bevy_window` contain no `register_type` call at all any more; what registers their types is
bevy's `reflect_auto_register` feature, which puts every `Reflect` type in the build into the
registry `App::default()` starts with (`bevy_app-0.19.1/src/app.rs:113-120`). It is part of
bevy's `default_app` feature and so of bevy's default features, which is why a game built on
`DefaultPlugins` finds `Transform` there — and why rubevy's own tests do not: this crate takes
bevy with `default-features = false`, so `examples/components.rs` names `Transform` itself. A
few plugins do still register by hand: `TimePlugin` registers the four `Time`s
(`bevy_time-0.19.1/src/lib.rs:76-79`), which is why `Time<Virtual>` is reachable by name under
`MinimalPlugins` with the feature off (`tests/resources.rs`).

**What a value looks like.**

| reflected | Ruby |
|---|---|
| struct | Hash, keys as Symbols |
| `Vec2` / `Vec3` / `Vec3A` / `Vec4` / `Quat` | Array of Floats — they are structs of `x`, `y`, … in bevy_reflect, but `tf[:translation][0]` is what a script wants |
| tuple struct, tuple, array, list, set | Array |
| map | Hash |
| enum, unit variant | the variant's name as a Symbol (`:Hidden`) |
| enum, struct variant | a one-entry Hash whose value is a **Hash** of the fields, `{Srgba: {red: …}}` |
| enum, tuple variant | a one-entry Hash whose value is an **Array** of the fields, `{Orthographic: [{scale: 1.0}]}` |
| numbers, `bool`, String, `char` | the Ruby ones |
| `Entity` | a `Rubevy::Entity` object, not a number |
| anything else opaque | nil |

**What a write does.** Only the fields the Hash names, so `e[:Hp] = { current: 4.0 }` leaves
`max` alone and two scripts that touch different fields do not undo each other — and reading a
component, changing one number and writing it back is one round trip, not two. An Array over a
struct goes by position, which is how `[1.0, 2.0, 3.0]` reaches a `Vec3`. A Symbol switches an
enum to that variant, as long as the variant has no fields: bevy builds a new variant from the
value handed to it, and a Ruby Hash says nothing about types, so a tuple or struct variant can
only have its fields written while it is already the current one. A field that cannot take what
it was given is logged with its path (`translation.x: takes a number`) and skipped; the rest of
the write still happens.

**A variant's fields are written in the shape the read answers**, and the two variants differ:
a *struct* variant takes the Hash of names, a *tuple* variant takes the Array —
`cam[:Projection] = { Orthographic: [ { scale: 2.5 } ] }` zooms a 2D camera, where the outer
one-entry Hash names the variant, the Array is `Projection::Orthographic(..)`'s one positional
field, and the inner Hash is an ordinary partial write of the `OrthographicProjection` struct
(`near`, `far`, `scaling_mode` and the rest keep their values). A tuple variant's field
**cannot be named**: `{ Orthographic: { "0" => … } }` is refused with `no such field`, because a
tuple variant's fields have no names in bevy_reflect, not even `"0"`. `tests/camera.rs` holds
both shapes, and `docs/worklog/2026-09-20-camera-from-ruby.md` is where it was worked out.

**A read costs no frame** (since 2026-09-17). `e[:Transform]` is still `Rubevy.ask` under a nicer
name, and the task is still parked on a queue while the question is out — but the question is
answered inside the very tick that asked it, so the value is there in the line that asked for it.
`tick_scripts` holds the world for as long as the scripts run, so a frame is a loop rather than a
single run:

> run the ready tasks → answer the component reads they parked on → run them again

`Vm::task_run_limits` comes back as soon as no task can run, and that moment is exactly when the
reads of that round are all waiting on the host. rubevy answers them out of the `&World` it is
holding, which puts those tasks back on the ready list, and hands them what is left of the frame.
The loop stops when a round answers nobody (waking no one, another run would find the same tasks
asleep), or when the frame's budget of instructions or its `frame_time` is spent. It has no limit
of its own and needs none: a script cannot ask a question without spending instructions on the
asking, so the budget the frame already had bounds the rounds.

**A read costs no frame, except at the tail of a frame that ran out of time** (since
2026-09-20). The answering is measured against `frame_time` too, not only the runs of the VM:
after every answer the tick looks at the clock, and once the frame time is up the questions it
has not reached are put off to the next frame, where they are taken **before** anything that
frame asks. So the promise is exact:

* A script whose question is reached inside the frame time reads the value in the line that
  asked for it, as it always did.
* A script whose question was still in the queue when the time ran out reads it in the same
  line, on the **next** frame. Nothing is lost, nothing is asked twice, and a question that is
  put off is not put off again in favour of one asked later — the order is first in, first out
  across frames as well as within one.
* Which scripts those are is not a matter of who asked first but of where the round ended: a
  thousand tasks that all park on a read in the same round are answered in the order they parked
  in, and the frame time decides where the line is drawn.

This is what makes `frame_time` a bound on the tick rather than a suggestion: with three
thousand scripts reading and asking, a `frame_time` of 8 ms used to give a tick of 14.9 ms and
one of 1 ms a tick of 3.3 ms (the whole round was answered whatever the clock said); the same
runs now give 8.3 ms and 1.4 ms. What a tick can still pass `frame_time` by is in the rustdoc of
the field and under "Time" below — the short of it is one answer, plus whatever the VM's own
notice of the deadline costs. The measurements are in
`docs/worklog/2026-09-20-frame-time-as-a-limit.md`.

Nothing of the world crosses into the VM to make this work. A native is handed `&mut Vm` and
nothing else, exactly as before; the world and the VM are two arguments of one function of
rubevy's, held apart by the borrow checker. There is no `unsafe` in the crate.

What a read sees, and what it still costs:

* **The world as it stands in `RubevySet::Tick`** — that is, before this frame's writes. It is
  what makes `.before(RubevySet::Tick)` worth writing: a system that must have moved the world
  before the scripts look at it belongs there.
* **A value written in the same tick is not readable yet.** `e[:Hp] = …` still lands at the end of
  the frame (`apply_component_writes`), so a read *after* a write in the same tick answers the
  **old** value. That is the promise `Commands` makes and the one the games are built on: read,
  decide, write.
* **What got cheap is the waiting, not the walking.** A read is about 2.2 µs on the machine it was
  measured on — one round of the loop: re-entering the scheduler, the type lookup, the
  reflection, the Hash built in the VM, and the ~75 instructions Ruby spent asking. A task that
  does nothing but read managed 2,667 of them in one frame, where before it managed one. But
  `Rubevy.find` still walks every entity in the world and `e.components` still walks the type
  registry, and that is unchanged: those two are for a lookup now and then, not for every frame.
* **A read made outside a tick waits for one.** A script run at `Startup` leaves its question on
  the queue like any other, and the first tick answers it before it runs anything else. There is
  no separate path and no exception.

The measurements are in `docs/worklog/2026-09-17-sync-reads.md`, and `tests/scheduling.rs` counts
the frames from the script's own side — zero for these, one for the questions the game answers,
in the same file.

The six kinds rubevy answers itself — `component.get`, `component.has`, `components`,
`entities.with`, `resource.get`, `writes.rejected` — never reach `ScriptWorld::take_requests`:
they are sorted out
where the request is made, so a game's answering system sees only its own. A game that wants
those names picks others.

**Why a read can wait inside `[]`.** Short as the wait is now, the task really is parked in the
middle of `[]`, and that took something of the VM. mrbc folds a one-argument `[]` into
`OP_GETIDX`, which the VM used to dispatch with `funcall` — a nested run loop, which is a native
boundary, and a task cannot be parked across one (`blocking pop cannot be called from within a C function boundary`).
For a while the read was `e.get(:Transform)` for that reason. SabiRuby now does what the
reference does: the opcode answers an Array, Hash or String itself and *sends* everything else
in the frame the call was made in, so a `[]` written in Ruby is an ordinary frame that a task
can be parked in. `e[:Transform]` is the spelling; `e.get(:Transform)` is the same call, kept
because spelling the round trip out reads better where it matters. The Ruby side of all this is
`src/prelude.rb`, compiled to `.mrb` and run when the VM starts, so a script has these without
requiring anything.

## Resources by name

A resource is read and written the way a component is, with the entity left out of it:

```ruby
s = Rubevy.resource(:Score)              # {points: 7.0, best: 12.0}, or nil
Rubevy.set_resource(:Score, { points: s[:points] + 1.0 })   # `best` is not named, so it stays
```

Everything the section above says about values holds here — the same table of what a struct, a
`Vec3`, an enum or an `Entity` looks like, the same partial write, the same path in the warning
when a field cannot take what it was given. **A read costs no frame**: `Rubevy.resource` is
answered inside the tick that asked it, out of the same `&World`, in the same gap between two
runs of the VM as `e[:Transform]`. **A write lands at the end of the frame**
(`apply_resource_writes`, right after `apply_component_writes`), so a read after a write in the
same tick answers the old value — read, decide, write, as everywhere else.

**What a type has to do to be there.** `#[derive(Resource, Reflect)]`, **`#[reflect(Resource)]`**,
and `app.register_type::<T>()`. The `#[reflect(Resource)]` is what makes the difference between
a resource and a component here: in Bevy 0.19 a resource *is* a component, kept on an entity of
its own, and `ReflectResource` carries no functions at all — it is the type's own word that it
is a resource. rubevy asks for that word, so `Rubevy.resource(:Transform)` is `nil` even though
`Transform` is registered and really is on entities. Four things are `nil` and none of them is
an error: a type nobody registered, a type that is not a resource, a resource nothing has
inserted into the world yet, and a name of no type at all.

**The name is the short type path**, exactly as a component's is — `Score`, and the whole path
(`my_game::Score`) where two types share the short one. A **generic** type carries its
parameters in that name, and a defaulted parameter is written out rather than left off: bevy's
`Time` is `Time<()>`, so `Rubevy.resource("Time<()>")` finds it and `Rubevy.resource(:Time)`
does not. `Rubevy.resource("Time<Virtual>")` is the one a game usually wants. Names with `<` in
them need quoting on the Ruby side (`"Time<Virtual>"` or `:"Time<Virtual>"`); a String and a
Symbol are the same thing to the call.

A game answers nothing for this: `resource.get` is one of the kinds rubevy answers itself, so it
never reaches `ScriptWorld::take_requests`. `tests/resources.rs` is the whole of it, including a
second VM under a name tag reading and writing the app's one resource, and
`tests/read_cost.rs::a_resource_read_beside_a_component_read` is the `#[ignore]`d instrument
that says what the extra lookup costs on the machine it is run on.

## A write the world would not take (`Rubevy.rejected_writes`)

A write lands at the end of the frame, so there is nothing to answer where it is made:
`e[:X] = hash` gives back the hash it was handed, whether the world took it or not. Until the
next frame there is nothing to say — and what the host had to say, it said to its log. So the
refusals of a frame's writes are kept, and a script reads its own on a later frame:

```ruby
cam[:Projection] = { Orthographic: [ { scale: 2.0 } ] }
sleep 0                                    # the clock moves, and the write has landed or not
Rubevy.rejected_writes.each do |w|
  Rubevy.log "#{w[:name]} on #{w[:entity].inspect}: #{w[:reason]}"
end
```

| key | what it is |
|---|---|
| `:entity` | the `Rubevy::Entity` the component was on, or **nil** for a resource write |
| `:name` | the type as the script spelled it (`"Projection"`, `"my_game::Hp"`) |
| `:reason` | the sentence the log carries, with the path inside the value where there is one (`translation.x: takes a number`) |

**Its own, and not everybody's.** A script is answered the writes *it* made — the entity of the
task that asked is what says which those are, and a task a script makes with `Task.new` carries
its maker's entity, so a script's second task counts as the same script. `:entity` is what was
written *to*, which is not the same thing: a script may write another entity's component, and it
is still that script's write. A task with no entity at all is answered an empty Array. The host's
side, `ScriptWorld::rejected_writes() -> &[RejectedWrite]`, is the wider one — every script's,
each carrying `by` (who wrote) beside `on` (what was written to).

**What is in it** is a refusal and not a mishap: a type nobody registered, an entity without that
component, a resource nothing has inserted, an enum in another variant than the one the value
names, a field that cannot take what it was given. A write to an entity that was despawned in
the meantime is **not** in it — there was nothing left to refuse — and that case makes no line
in the log either, so the list and the log say the same things.

**One frame's worth.** The list holds what one frame's writes refused, and the next frame in
which the scripts write anything replaces the lot. It is not emptied by every frame on purpose:
a script cannot ask to be woken on the very next one (`sleep 0` waits for the VM's clock, which
moves in whole ticks of 4 ms), so a list that lasted one frame would usually be gone before the
script that wrote could look at it. Holding it until the scripts next write is the soonest
anything in it could be out of date.

**It has no limit and needs none.** A script cannot write without spending instructions on the
writing, so the frame's `budget` already bounds how many refusals a frame can make: 33
instructions each in the narrowest loop that can make one, which is some six thousand under the
default budget (`tests/rejected_writes.rs::what_one_rejected_write_costs`, an `#[ignore]`d
instrument — the count is of the VM and not of the machine).

**The log is unchanged.** Every refusal still makes the `warn!` it always made, because a game's
log is where a game already looks; this is for the script, which could not see the log.
`tests/rejected_writes.rs` is the whole of it and
`docs/worklog/2026-09-20-rejected-writes.md` is how it was decided.

## Events

`Rubevy.subscribe(:hit)` answers a `Task::Queue` that a game pushes onto. Waiting for something
to happen is then the same thing as waiting for an answer — the task is parked and costs
nothing — and it can be done in a task of the script's own:

```ruby
hits = Rubevy.subscribe(:hit)          # this entity's own, and everyone's
Task.new(name: "reflex") do
  loop do
    by, damage = hits.pop              # parked here until the game publishes
    Rubevy.log "hit by #{by.inspect} for #{damage}"
  end
end
```

A reflex is then not a callback that interrupts the brain; it is another task that happens to be
ready — and it is the script's task in every way that matters here: **a task made with
`Task.new` carries the entity of the task that made it**, so it may `Rubevy.subscribe`,
`Rubevy.ask` and `Rubevy.entity` for itself. (The entity is an ordinary instance variable on the
task, `@rubevy_entity`, and the prelude copies it in `Task.new`, where the task doing the making
is still the one running. A task that should act for another entity sets it itself.) Subscribing
in the script's own task and sharing the queue, as above, is still the clearer shape when
several tasks read one stream.

The game publishes:

```rust
scripts.publish(Some(entity), "hit", Answer::Num(damage as f64));   // that entity's scripts
scripts.publish(None, "bell", Answer::Num(frame as f64));           // everyone who subscribed
```

and for a payload that is not flat, `publish_value(entity, name, |vm| …)` builds the value
inside the VM, the way `answer_value` does. An entity inside a list is
`entity_object(vm, class, entity)`, with `class` read from `ScriptWorld::entity_class()` before
the closure.

**Bevy's events are joined by the game, one line per event type.** rubevy does not tie them to
Ruby by itself: which events a script may see, and what each one carries, is the game's to say.

```rust
app.add_observer(|on: On<Hit>, mut scripts: ResMut<ScriptWorld>| {
    scripts.publish(Some(on.entity), "hit", Answer::Num(on.damage as f64));
});
```

An ordinary system reading `MessageReader` does just as well. `examples/events.rs` has both, and
a script (`assets/scripts/events.rb`) with a brain and a reflex.

**Rules.**

* Nothing is queued for a name nobody subscribed to, so a game may publish freely.
* A queue holds `ScriptWorld::queue_limit` messages and drops the oldest past that. A script
  that wakes late gets the last few things that happened, not the first few — and a script that
  never reads its queue is not a leak with a slow fuse. The limit starts at
  `ScriptWorld::QUEUE_LIMIT` (64), and both ends can move it: see **How much a queue holds**
  below.
* A subscription is let go of when the script's task ends and when its `ScriptTask` is removed
  (a reload, a despawn). A script that runs off its end keeps its `ScriptTask` — that is what
  stops it starting again — so both places matter. `ScriptWorld::subscriptions()` says how many
  are standing.
* **The queue is closed when the subscription goes, and a `pop` waiting on it raises
  `Rubevy::Unsubscribed`.** The script's own task is terminated with its `ScriptTask`, but a
  task it made with `Task.new` is not: parked on a message that will never come, it would stand
  there for as long as the VM lives, holding its context and everything its block closed over,
  and its `ensure` would never run. The raise unwinds it instead:

  ```ruby
  Task.new(name: "reflex") do
    begin
      loop { handle(hits.pop) }
    rescue Rubevy::Unsubscribed        # the script is going; end on our own terms
      Rubevy.log "reflex off"
    ensure
      release_the_wheel                # this runs either way
    end
  end
  ```

  Whatever was already in the queue is popped first, so a script that was behind still reads
  what it missed before it hears the end. An unrescued `Rubevy::Unsubscribed` simply becomes
  that task's result, as any unhandled exception in a task does; nothing else stops.

  It is the subscription's queue that says this, not every queue: `Rubevy.subscribe` extends the
  one it answers with `Rubevy::Subscription` (`pop`, `shift`, `deq`, `dropped`). A queue from
  `Rubevy.ask` is an ordinary `Task::Queue` and keeps mruby-task's own meaning, where a closed
  queue answers `pop` with nil.

### How much a queue holds, and what it lost

A full queue drops its oldest message to make room for the newest. That used to happen in
silence — no log, no counter — and at a number (64) that nothing could change. Both ends can now
see it and both ends can set it.

**Seeing it.** `ScriptWorld::dropped()` is how many messages this VM has dropped since it
started, over every subscription: one number for a HUD, and a number that climbs is a game
publishing faster than its scripts read. `Rubevy::Subscription#dropped` is the same count for one
subscription, which is the one a script can act on — it says that there is a hole between what it
read last and what it is reading now, which `pop` cannot say, because a dropped message leaves
nothing behind:

```ruby
belt = Rubevy.subscribe(:belt)
missed = 0
loop do
  item = belt.pop
  if belt.dropped > missed
    Rubevy.log "fell behind by #{belt.dropped - missed}"
    missed = belt.dropped
  end
  handle item
end
```

**Setting it.** The VM's own default is a field beside `budget`:

```rust
world.queue_limit = 512;      // this VM's subscriptions, unless one asked for its own
```

It is read where a message is published, not where a script subscribed, so changing it moves
every standing subscription that did not name its own — and a second VM (`ScriptWorld<Mods>`) has
a limit of its own, as it has a budget of its own. A script that knows its own stream says so:

```ruby
belt  = Rubevy.subscribe(:belt, limit: 512)   # a heavy stream this script reads in bulk
alarm = Rubevy.subscribe(:alarm, limit: 1)    # only the latest matters
```

`limit:` takes a whole number of messages, one or more; anything else — a Float, a zero, a
keyword that is not `limit:` — is an `ArgumentError` where the script wrote it. A subscription's
own limit is its own for as long as it stands, whatever the VM's field does afterwards.

**What to set it to.** 64 is where it starts, and 64 is not a measured number: it was chosen in
2026-09-15 for the qualitative reason that it is "enough for a script that reads its queue every
few frames". Two measured numbers bound it from either side
(`docs/worklog/2026-09-20-overflow-and-limits.md`, 2026-09-20, one machine):

* **From above.** A script that does nothing but `pop` and count gets through about **3,900
  messages in a frame** before the frame's 200,000 instructions stop it — 51 instructions a
  message. A limit larger than that is a queue its owner can never empty, so overflow is delayed
  rather than avoided.
* **From below.** A message waiting in a queue costs **16–21 bytes** as a number, **300–340** as
  a short string and **330–520** as a small array (the VM's own object, not the queue's slot).
  A subscriber that never reads holds `limit` of them, so the worst case for a VM is
  `limit × bytes × subscriptions`: at the default 64, a thousand such subscriptions hold 1.0 MB
  of numbers or 20.5 MB of short strings.

So: a game that publishes numbers to a handful of scripts can raise this into the thousands and
lose nothing; a game that publishes strings or arrays to hundreds of scripts should think about
lowering it. rubevy knows neither, which is why the number is yours and the default is the one
that hurts least.

There is no `drop: :newest`: the oldest goes, which is what a script that wakes late wants, and
nobody has asked for the other.

## An optional layer, and the first one (`Rubevy::Camera`)

A **layer** is Ruby the crate carries and does not run. The app says whether its scripts get it,
in one line at `Startup`:

```rust
fn add_the_camera_layer(mut world: ResMut<ScriptWorld>) {
    world.load_and_run(rubevy::layers::CAMERA).expect("the layer runs");
}
```

`ScriptWorld::load_and_run(&[u8])` runs a `.mrb` in the VM the way the prelude is run — its top
level runs now, in the system that called it, and what it leaves behind is there for every
script. It is not how a *script* is run (a script belongs to an entity and gets a task), so the
program must not park: no `sleep`, no `Rubevy.ask`, nothing that waits for a frame. A layer that
defines classes and methods does none of that. The same call takes a library of the game's own.

**Why a layer and not the prelude.** The prelude is the host API: a script has
`Rubevy::Entity#[]` because it is rubevy's, as `Rubevy.ask` is. A layer is a way of speaking
about *something a game has*, and rubevy does not know what a game has. Nothing in `src/` knows
what a camera is and nothing is to start: the camera layer is Ruby over `Rubevy.find`,
`e[:Transform] =`, `e[:Projection] =` and `Rubevy.ask`, and the crate's dependencies are the same
with it as without it (`bevy_camera` is a dev-dependency, for the tests and the example). An app
that loads nothing has no `Rubevy::Camera`, and `tests/camera_layer.rs` begins by checking that.

### The camera layer

```ruby
cam = Rubevy::Camera.find            # the 2D one first (`markers`), or nil
Rubevy::Camera.find_all              # every camera, as Rubevy::Camera objects
Rubevy::Camera.attach(e, :Camera3d)  # one on an entity you already hold

cam.move_to(100.0, 0.0)              # z is kept unless you pass one
cam.pan(20.0, -40.0)                 # and two pans in one tick both count
cam.zoom 2                           # twice as close: 2D and 3D alike
cam.zoom 2                           # 4x, not 2x
cam.scale                            # 4.0 — the magnification, not bevy's `scale`
cam.position                         # [x, y, z]
cam.follow(target)                   # a task of its own; `unfollow` stops it
cam.world_at(x, y)                   # a question for the game; answers the queue
cam.reload                           # read the projection again, from 1x
cam.rejected_writes                  # what this camera's writes ran into
```

**`zoom 2` is twice as close on both cameras**, which is the layer's main reason for being: the
two are zoomed by different numbers, in different units, in opposite directions.

| | what holds the zoom | what `zoom f` writes |
|---|---|---|
| `Projection::Orthographic` (2D) | `scale` — how much world fits across the window, so apparent size is `1 / scale` | `scale / f` |
| `Projection::Perspective` (3D) | `fov` — the angle seen, in radians; what sets apparent size is `tan(fov / 2)` | `2 * atan(tan(fov / 2) / f)` |

Halving the angle itself would be wrong by more and more as the angle grows, and could pass pi,
which is no angle at all; through the tangent, any positive magnification lands somewhere in
`0 < fov < pi`, because `atan` does. `zoom 2` then `zoom 0.5` comes back exactly where it
started, on either camera.

**What the layer remembers, and why.** Two of the three are forced by the frame:

* **The variant.** A Ruby write cannot switch an enum's variant, so the layer reads `Projection`
  once (`attach`, `reload`) and writes into the variant it found. `reload` is how a script is
  told to look again after the *host* has changed the projection from Rust.
* **The magnification.** A write lands at the end of the frame, so reading the projection,
  multiplying it and writing it back would lose every `zoom` in a tick but the first. The layer
  holds the magnification on the Ruby side and writes an absolute value. `scale` answers that
  magnification — 1.0 after a `reload`, whatever bevy's own number happens to be.
* **Where it put the camera**, for this frame only: `position` answers what the layer wrote if it
  wrote it this frame (`$rubevy[:frame]`, which costs no round trip) and the world's own
  otherwise, which is what makes two `pan`s in one tick add up.

**`follow(target, every = 0, offset = nil)`** is a task of its own (`Task.new`, so it carries the
script's entity) that moves the camera to the target's x and y. `every` is what it sleeps between
turns — 0 is "every frame the VM's clock moves" — and `offset` is `[dx, dy]` or `[dx, dy, dz]`,
where the third puts the camera dz from the target's z instead of leaving its own alone. There is
no smoothing and no speed in it, because either would be a number this layer has no grounds for;
a game that wants one writes its own task around `move_to`. It stops on `unfollow`, and by itself
when the target is gone — a despawned entity's `Transform` reads nil.

**`world_at(x, y)` is a question with no answerer.** Where a point on the window is in the world
is worked out from the camera's placing and its projection; no component holds it, so the layer
asks `Rubevy.ask("camera.world_at", camera_entity, x, y)` and hands the **queue** back without
waiting. The camera is in the question because a game may have several. Answering is the game's,
and **nobody answers it by default**: a `pop` on the queue of a question no system answers parks
that task for ever. So pop it where the game says it answers this, or in a task of its own.
`examples/camera_from_ruby.rs` shows the answering side.

## A dynamic proxy (`Rubevy::Proxy`)

`assets/scripts/proxy.rb` is a small Ruby library that turns a call into a question:

```ruby
require "proxy"

robot = Rubevy::Proxy.new("robot")
robot.move_to(1, 2)          # => Rubevy.ask("robot.move_to", 1, 2).pop
robot.hp                     # => Rubevy.ask("robot.hp").pop
```

The game answers `"robot.move_to"` as it answers any other kind, and the task is parked on the
answer in the usual way — the call takes as many frames as the answer does. The proxy has a
`kind`, an `inspect` and a `respond_to_missing?` that says yes to everything, so `respond_to?`
agrees with what a call actually does.

**Registration is the plain way; a proxy is for objects the game did not register.** A real
method — `Rubevy.spawn`, `Rubevy.move_to`, or one the host adds with `Vm::define_fn` — says what
it takes, fails at the call when the name is wrong, and costs nothing to look up. A proxy says
nothing about itself: every name is accepted, every mistake becomes a question the game has to
recognise or ignore, and the script finds out at run time. Use it where the thing on the other
side is not rubevy's to register — another robot, an entity of a game that decides its own verbs,
a service reached over the `ask` channel — and register everything else.

It rests on one property of the VM: a `method_missing` written in Ruby is dispatched in the frame
the call was made in, not in a nested run loop, so the body may park the task on `pop` (SabiRuby
`docs/design/fibers.md`, "Native boundaries"). Before that the same file raised
`blocking pop cannot be called from within a C function boundary`.

## What the script sees of the frame

`$rubevy` is refreshed at the head of every frame: `:frame` (the count), `:delta` (seconds since
the last frame) and `:time` (seconds since the start). Reading it is how a script knows where it
is in the run without asking.
The plugin puts it there with `Vm::global_set`, so it is an ordinary global: a script may write
to it, and what it writes stands until the next frame replaces it.

## Time

`sleep` in a script waits in real time: the plugin gives mruby-task's clock Bevy's `Time`
(`task_external_clock` + `task_advance_ticks`), so `sleep 0.5` wakes on the frame about half a
second later. What still comes from the instruction count is the timeslice, which is what keeps
one script from eating a frame — a script that never sleeps is preempted and resumed next frame.

The instruction count cannot stop everything, so each frame also runs under time limits
(`Vm::task_run_limits`, on a clock the plugin gives the VM — Bevy's `Instant`, which a browser
has too):

| `ScriptWorld` field | default | what it does |
|---|---|---|
| `budget` | 200,000 instructions | checked between timeslices, as before; **zero pauses the scripts** |
| `frame_time` | 8 ms | the tick's bound: the running timeslice is cut short once the frame's scripts have taken this long, and the answering stops there too |
| `overrun` | 50 ms | a script that cannot be switched out — inside a native waiting for a block, `sort { }` or `Array.new(1) { loop { } }` — gets `Task::Overrun` past this, and the frame comes back |

All three are fields of `ScriptWorld<M>`, so an app with a second VM has a second set of them and
nothing adds the two together: the worst case per frame is the sum ("Two VMs in one app").

**`frame_time` bounds the tick, and here is what it does not cover.** A tick is a loop — run the
ready tasks, answer what they parked on, run them again — and until 2026-09-20 the clock was
looked at only at the head of a round, so a round that had parked three thousand tasks then made
three thousand answers however late it was. Now the answering is measured against the same
clock. What is still outside the number, which is to say by how much a tick can pass it:

* **One answer.** A late tick still makes one answer of rubevy's own kinds and one of the
  game's, because the run of the VM before them may have used the whole frame time by itself,
  and a tick that answered nobody would leave every task parked on its question for ever (a
  script with no `sleep` and no question in it does exactly that to the others; `tests/frame_time.rs`
  is that test). The overshoot is the dearest single answer: a couple of microseconds for a
  component or resource read, longer for `Rubevy.find`, whatever the game wrote for an
  `answer_in_tick` closure.
* **The VM's own notice of the deadline.** `Vm::task_run_limits` is handed what is left of the
  frame and comes back when it is gone, but not to the nanosecond: with three thousand tasks
  waiting, a tick whose scripts ask *nothing at all* still takes about 2.9 ms under a
  `frame_time` of 1 ms. That overshoot is inside the VM's scheduler — every push and every wake
  walks the waiting tasks — and rubevy cannot shorten it from the outside; it is on SabiRuby's
  list. At a few hundred tasks it is not there at all.
* **A second VM**, as above: nothing adds the two `frame_time`s together.
* **Everything in the frame that is not the tick.** Starting the scripts of new entities
  (`Vm::load` and a `task_spawn` an entity), carrying out their commands, the writes at the end
  of the frame, and what the game publishes to them are all systems of their own.

The numbers above are of one machine on one day (`docs/worklog/2026-09-20-frame-time-as-a-limit.md`,
which has the before-and-after table); what they are on yours is what the instruments say.

**`Time<Virtual>` is readable by name.** Bevy's `TimePlugin` registers its four `Time`s, so a
script can read the game's clock as an ordinary resource — `Rubevy.resource("Time<Virtual>")`,
and `"Time<()>"` for the default one ("Resources by name"). `$rubevy` is still the cheap way to
ask what frame it is; the resource is there when a script wants what the *game* calls time,
including a pause the game made with `Time<Virtual>`.

**Pausing.** `world.budget = 0` is the pause: the VM checks the budget at the head of its own
loop, so not one instruction runs. While it is zero the plugin also stops moving mruby-task's
clock on, so **a pause is time the scripts did not live through** — a script that had 0.07 s of a
`sleep 0.1` left when the pause began has 0.07 s left when the budget comes back. Before this,
the clock ran while nothing did: a hundred paused frames made every sleeping script due at once,
and the frame after the resume woke the lot of them (SabiRuby Battle's `P` key, rubevy_games
`docs/worklog/2026-09-16-showpieces-d2-d3.md`). `tests/pause.rs` pauses for a hundred frames and
checks that the sleeper does not wake on the frame after the resume.

It is a budget of zero and not a `pause(bool)` because a flag would be a second way to say the
same thing, and two of them can disagree. The game keeps its own old budget to put back.

Everything else in the frame goes on while paused: `$rubevy` is still refreshed (so a HUD or a VM
inspector panel sees a live frame count), and questions are still taken and answered — they simply
reach a script that is not running. Bevy's own `Time` is the game's to pause (`Time<Virtual>`);
this is about the VM's scheduler only, so `$rubevy[:time]` keeps growing.

`Task::Overrun` is an `Exception`, not a `StandardError`, so a script's `rescue => e` does not
keep it going; the script ends with it (`ScriptEnded { status: Failed }`). Timeslices themselves
stay counted in instructions, so what a script does is the same on every machine; the clock only
bounds a frame. A single native that takes long (reversing a 20 MB string) is not interrupted and
is noticed after it returns. `tests/replace.rs` checks that a stuck script no longer holds the
frame (without `overrun`, that test never finishes). The VM side is written up in SabiRuby's
`docs/gems.md`, mruby-task, "Time limits".

## Building against the VM

`Cargo.toml` names the VM from git while the entry points this plugin needs are still being added
(`Vm::task_instructions` and `task_location`, and since 2026-09-15 `define_closure`,
`set_host_state`, `define_fn`, `data_new`, and `task_running` / `ivar_get` / `ivar_set` /
`global_get` / `global_set` / `is_exception`, are newer than the published 0.4.0). Those last six
are what this plugin used to reach into `Vm`'s public fields for, and with them it no longer
touches `vm.heap`, `vm.task` or `vm.globals` anywhere. A clone still builds on its own — cargo fetches it — and it goes back to a crates.io version once the API
settles. To work against a checkout of the VM next to this one, redirect it in a git-ignored
`.cargo/config.toml` instead of editing `Cargo.toml`:

```toml
[patch."https://github.com/sabiruby/sabiruby"]
sabiruby = { path = "../sabiruby" }
sabiruby-compiler = { path = "../sabiruby/compiler" }
```

**rubevy also builds for the browser** (`wasm32-unknown-unknown`), where std has no clock, no
filesystem, no threads and no sockets — and compiles them all regardless, so the failure comes as
a panic in the first frame rather than as a build error (`docs/worklog/2026-09-18-wasm-instant.md`
is the day it did). `cargo test --test no_wasm_unsupported` therefore reads `src/` for the std
APIs a browser does not have; a line that really is native-only says so on the line, in a
`// wasm: <reason>` comment.

## Adding to the VM (`ScriptWorld::vm`, at `Startup`)

A game usually wants more in the VM than rubevy puts there: a JSON class, a module of its own, a
native or two. `ScriptWorld::vm` is public and the resource exists as soon as `RubevyPlugin` is
added, while the first script does not start until the first `Update` — so a `Startup` system is
the place, and rubevy needs no entry point for it.

```rust
app.add_plugins(RubevyPlugin::default())
    .add_systems(Startup, install_host_api);

fn install_host_api(mut world: ResMut<ScriptWorld>) {
    let vm = &mut world.vm;
    sabiruby_serde::install_json(vm);                 // JSON.parse / JSON.generate / #to_json
    let object = vm.core.object;
    vm.define_fn(object, "arena_size", |_vm: &mut Vm| -> f64 { 240.0 });
}
```

`sabiruby-serde` is the crate beside the VM in the same repository (`sabiruby/serde`, feature
`json` on by default). **rubevy does not depend on it** — it is a dev-dependency here, for the
test and for this example. A game that wants `JSON` names it itself, which is also what says who
owns the decision: the `JSON` class is the game's, not the engine's.

`tests/vm_setup.rs` checks both the `Startup` system and the other way of saying it (reaching the
resource while the `App` is still being built, before `run()`), and that a script's very first
line already sees what was installed.

**What not to do with it.** It is the VM the scheduler is running, not one of your own.

* Do not run tasks through it — no `task_run_limits`, `task_run_once`, `Task.run`. `tick_scripts`
  is what gives the scheduler its frame. (`Vm::load_and_run` at `Startup` is fine: that is running
  a program, not the scheduler.)
* Do not keep a `Value` or an `ObjId` past the call unless you `gc_register` it. `Request` and
  `ScriptTask` are the two things rubevy keeps that way, and both are registered.
* Do not try to touch Bevy's `World` from a native: a native gets `&mut Vm` and nothing else.
  Leave something behind for a system to pick up — which is what `Rubevy.ask` and
  `ScriptWorld::take_requests` already are.

Adding to the VM after `Startup` is not forbidden, and is sometimes what a game means (a class
that exists only once a level is loaded). What it costs is that a script which has already run may
have seen the VM without it.

## Two VMs in one app

`RubevyPlugin::default()` is the app's first VM, and that is all most games ever want: their
scripts are written together, so sharing globals and constants is a feature. A game that runs
somebody else's code — mods, a player's own script, a lesson in a teaching app — wants that side
kept out of its own. rubevy's answer is a **second VM**, which is the same plugin added again
under a **name tag**:

```rust
use rubevy::RubevyPlugin;

struct Mods;                                              // the name tag: an empty struct

app.add_plugins(RubevyPlugin::default())                  // the game's VM, spelled as always
   .add_plugins(RubevyPlugin::<Mods>::for_vm("assets/mods"));   // and the mods'
```

The tag is a type and nothing else — it implements nothing, holds nothing, and costs nothing at
run time. What it does is give every one of the plugin's types a second copy:

| the first VM | the VM named `Mods` |
|---|---|
| `RubevyPlugin::default()`, `RubevyPlugin::with_asset_root(root)` | `RubevyPlugin::<Mods>::for_vm(root)` |
| `Script::new(handle)` | `Script::<Mods>::for_vm(handle)` |
| `ScriptWorld` (a `Resource`) | `ScriptWorld<Mods>` |
| `ScriptTask` (a `Component`) | `ScriptTask<Mods>` |
| `ScriptDone::new()` | `ScriptDone::<Mods>::for_vm()` |
| `ScriptEnded` (a message) | `ScriptEnded<Mods>` |
| `RubevySet::Deliver` / `Tick` / `Answer` | `RubevySet::<Mods>::deliver()` / `tick()` / `answer()` |

The plain spellings are the first VM's, so **an app that has one VM writes exactly what it wrote
before**. The reason they are `new` / `Answer` on one side and `for_vm` / `answer()` on the other
is Rust's and not Bevy's: a default type parameter (`ScriptWorld<M = ()>`) is filled in where a
*type* is written, but not where a value is, so `Script::new(h)` cannot be generic and stay
spellable without a turbofish.

An answering system of the second VM goes in that VM's set:

```rust
use rubevy::{Answer, RubevySet, ScriptWorld};

fn answer_mods(mut world: ResMut<ScriptWorld<Mods>>) {
    for request in world.take_requests() {
        world.answer(&request, Answer::Nil);
    }
}
app.add_systems(Update, answer_mods.in_set(RubevySet::<Mods>::answer()));
```

### What the two VMs do not share

Heap, globals, constants, classes, symbols, the symbol table, GC, mruby-task's scheduler and its
queues, the host state (the command queue **and the subscription list**), the frame budget and
`require`'s load path. A script of one VM has no way to name anything in the other: they are two
`Vm` values in two Bevy resources, and SabiRuby keeps no global state at all — no `static mut`,
no `thread_local!`, no lazily built table — so two VMs in one process are as separate as two
processes would be.

Three of those are worth spelling out.

**`publish` goes to that VM's subscribers.** `ScriptWorld::publish` walks the subscription list,
and the list lives inside the VM (`Vm::set_host_state`). So a message published on `ScriptWorld`
never reaches a script that subscribed in `ScriptWorld<Mods>`, and a game that wants both to hear
something publishes twice — once per VM, which is also where it gets to decide that the mods hear
less than the game does. There is no all-VMs spelling on purpose: it would need a second
subscription index outside the VMs, which is a new concept for a case that a two-line loop covers.
`tests/two_vms.rs` checks both directions.

**The budget is per VM.** `budget`, `frame_time`, `overrun` and `queue_limit` are fields of
`ScriptWorld<M>`, so each VM gets its own share and **nothing caps them together**: with N VMs a
frame's worst case is N × `frame_time` (8 ms each by default, so two VMs is 16 ms). A game that adds a VM should lower
the new one's numbers rather than leave two default budgets running:

```rust
fn give_the_mods_less(mut mods: ResMut<ScriptWorld<Mods>>) {
    mods.budget = 20_000;                                  // the default is 200_000
    mods.frame_time = Some(Duration::from_millis(1));       // the default is 8 ms
    mods.queue_limit = 16;                                  // the default is 64
}
```

A whole-frame budget shared between the VMs is deliberately **not** in rubevy: it would change
what `frame_time` means for the app that has one VM, and it would make one VM's runaway script
able to starve another's — which is the thing a second VM was for.

**A `.mrb` loaded into two VMs is two copies.** The asset (`MrbAsset`, its bytes) is shared —
`init_asset::<MrbAsset>` is registered once however many plugins are added — but what the VM
builds out of those bytes, the irep, is the VM's own heap. Two VMs running the same script hold
two of it. A VM is about half a megabyte resident before any script, so this is the price of
isolation: memory and start-up (about 0.6 ms per VM, nearly all of it reading mrblib), not frame
time.

### What they do share

The Bevy world. Both VMs' scripts spawn, despawn, read and write components, subscribe and ask
through the same `World` — isolation here is about the *language*, not about authority. A mod
that should not touch the game's entities is the answering system's business: it sees a
`Request` that came from `ScriptWorld<Mods>` and can refuse it. What the name tag gives is that
the answering system *knows* which VM asked, because it is a different system reading a
different resource.

The same entity may carry a script of each VM (`Script` and `Script<Mods>` are different
components), which is how a game would let a mod add behaviour to something that already has its
own. Their tasks, their ends (`ScriptDone<M>`) and their statistics stay apart.

`examples/two_vms.rs` is a working app: a mod that cannot see the game's `$world_seed` or `Boss`,
whose `loop {}` runs three million instructions without costing the game's script a beat, and
whose `require "helper"` gets a `LoadError` while the game's own gets the file.

### What this is not

It is not a sandbox against a hostile script. Three things say so plainly.

There is no heap cap yet, so a mod can still take the memory (`docs/outlook.md`, possibility 4).

**The load path separates by name, not by a jail.** `require "helper"` in the mods' VM raises
`LoadError` because `$LOAD_PATH` is `["assets/mods/scripts", "assets/mods"]` and the game's file
is in neither. But SabiRuby's `require` (`src/mrblib/require.rb`, the shape picoruby-require has)
treats a name beginning with `/`, `./` or `../` as a path that says where it lives and looks it up
**with no load path at all** — and rubevy's `FileHost::read_file` is `std::fs::read`, so that path
is resolved against the process's working directory. `require "./assets/scripts/helper"` from a
mod reads the game's file; `require "assets/scripts/helper"` does not, because a bare name still
goes through the load path and becomes `assets/mods/assets/scripts/helper.mrb`. A game that hands
out a VM to code it does not trust wants a `Host` of its own — the trait is the plugin's
`FileHost`, and a host that refuses a path outside its root is a dozen lines — rather than the
load path alone.

And a native the game installs into the mods' VM is exactly as powerful as the game makes it: a
second VM is isolation from the *game's Ruby*, not authority over what the host answers.

It is also compile-time: the VMs an app has are the name tags its source names. One VM per player
or per loaded mod, decided while the game runs, would be a different design (a VM on an entity,
or a `Vec` of them in one resource) and would show up in the game's own code;
`docs/plans/multi-vm-plan.md` §5 stage 4 says why it is not built.

## What a HUD can show of a script

`ScriptWorld::stats(&ScriptTask)` answers a [`ScriptStats`]: the instructions the script has run
(the difference between two frames is what it spent on that frame), where it stands in its own
source (file and line — while it waits as well as while it runs), and whether it is finished.
It is what makes a panel like "Scout — robots/scout.rb:12 — 4,200 insn" possible, which is the
thing this VM can show and an engine's usual scripting cannot.

## What a HUD can show of a frame (`FrameStats`)

`ScriptWorld::last_frame()` answers a [`FrameStats`]: what the last tick of that VM came to.
Where `stats` is one script, this is the frame.

| field | what it is |
|---|---|
| `frame` | bevy's `FrameCount` when the tick ran, so a reader can tell this frame's answer from last frame's |
| `instructions` | what the tick took out of `budget`, over every script of the VM |
| `rounds` | times round the tick's loop — run the ready tasks, answer what they parked on, run them again |
| `reflect_answers` / `in_tick_answers` | questions answered in the tick: rubevy's own kinds (a component or resource by name, `Rubevy.find`) and the game's (`answer_in_tick`), counted apart because they are bounded apart |
| `carried_reflect` / `carried_in_tick` | questions this frame did not reach and left for the next one, waiting at the head of its answering ("Time") |
| `dropped` | messages the VM's full queues lost since the previous tick — the difference of `ScriptWorld::dropped` |
| `time_ns` | the wall clock of the tick, on the clock the VM itself was given. **This is the number `frame_time` bounds** |
| `longest_answer_ns` | the dearest single answer of the tick, or `None` where the app set no `frame_time` (then the tick reads no clock and has nothing to say) |
| `more_to_run` | whether the VM still had a task that would run — a yes or no, because `Vm::task_pending` is a yes or no |
| `loaded_programs` | the VM's standing count, the one number here that is not of the frame |

Nothing here is a total kept for you: every field but the last is of that one tick, and an
average or a worst case over several frames is the game's to keep, because how many frames it
should be kept over is the game's question. Nothing of it reaches a script either — `$rubevy`
carries the frame, the delta and the time and no more — and a game that wants its scripts to see
what its scripts cost publishes it or answers a question with it.

```rust
fn hud(scripts: Res<ScriptWorld>) {
    let f = scripts.last_frame();
    info!("{} insn in {:.2} ms, {} answers", f.instructions, f.time_ns as f64 / 1e6, f.reflect_answers);
    if f.carried_reflect + f.carried_in_tick > 0 {
        info!("{} questions put off to the next frame", f.carried_reflect + f.carried_in_tick);
    }
}
```

**A paused frame is a frame**: with `budget = 0` the tick ends before its first round, so the
counters are zero and `more_to_run` is what tells a paused VM from one whose scripts have all
ended. **A second VM has its own**, as it has its own budget.

**`time_ns` is not what a system after `RubevySet::Tick` measures.** That set holds four systems
— the tick, the commands the scripts left, and the two kinds of write — so timing it from
outside times all four. On one machine with a thousand scripts the two differ by a millisecond
(`docs/verification/scale.md`), and it is the smaller number that `frame_time` is about.

**Two instruments print these numbers** beside the wall clock of the frame, for anyone who wants
to know what their own machine carries:

```
cargo run --release --example how_many_scripts        # scripted entities, from ten to three thousand
cargo run --release --example how_many_subscribers    # subscribers and messages a frame
```

Both are native-only measuring instruments that assert nothing, both take their sizes as
arguments, and both have a `mem` mode that runs one configuration in a process of its own.
`docs/verification/scale.md` is a run of them on one machine, beside what the same instruments
said before the work of 2026-09-20.

## Replacing and removing a script

A game reloads a script by removing the entity's `ScriptTask` and inserting a new `Script`, and
ends one by despawning the entity. Either way `ScriptTask`'s `on_remove` hook terminates the task
in the VM (`Task#terminate`) and lets the collector have it.

**Replacing is `replace_script`**, which is those lines under a name:

```rust
use rubevy::{replace_script, Script};

replace_script(&mut commands, entity, Script::new(compiled).with_name("brain.rb"));
```

It takes the `Script` rather than a handle, so the new script's name and priority are set the way
they are set anywhere else, and the VM it belongs to comes from the `Script`'s own name tag
(`Script::<Mods>::for_vm(h)` replaces a script of the VM named `Mods` and leaves the first VM's
script on the same entity alone). What it does is take the `ScriptTask` off, take the `ScriptDone`
marker off in case the old script had already ended — without that, an entity whose script ran to
its end could never be given another one — and insert the new `Script`. Both sample games had
those three lines copied into them, and it is a reload button in each.

**Stopping** a script is still just removing its `ScriptTask` or despawning the entity: there is
no function for it because there is nothing else to do.

Before that hook the task was only forgotten by the ECS: it stayed in the scheduler's queues and
kept running, still carrying its entity, so a reloaded robot had two brains asking for the same
body — the old one invisible, since nothing showed it any more. `tests/replace.rs` checks both
paths (a question already asked when the script is replaced may still be answered once; no new
one is asked).

**Several at once is a frame like any other.** Removing a `ScriptTask` also closes what that
entity subscribed to, so every task parked on one of its queues wakes with
`Rubevy::Unsubscribed` and ends. Ten scripts with six such tasks each is sixty tasks ending in
the same frame, and that used to cost sixty frames in which *nothing in the VM ran at all* — the
scripts nobody had touched included — because a task ending with a nil result looked to the VM's
frame loop exactly like an empty scheduler (SabiRuby `docs/design/gems.md`, mruby-task; the
report is rubevy_games' garden). With a VM that has the 2026-09-17 scheduler fix, a burst costs
the frame it is in and no more: `tests/restart_burst.rs` replaces ten such scripts in one frame
and checks that an eleventh, untouched, keeps running on every frame after it. Against a VM
without the fix that test fails, which is what it is there for.

## One program, one irep

A hundred entities running one `.mrb` load it **once**. The plugin keeps every program it has
loaded, by the bytes of the program itself, and `start_scripts` looks there before it asks the VM
to load anything; `Vm::task_spawn` takes an irep, so the hundred tasks are spawned from that one
copy. `ScriptWorld::loaded_programs()` is how many distinct programs this VM holds.

The key is the program and not the handle it arrived in, because a game that compiles a player's
Ruby itself adds a **new** asset every time (`Assets::add` hands out a fresh id, and both sample
games compile once per script *and once per creature*). Two assets with the same bytes are one
program; a program that changed is other bytes, so it misses the table and is loaded — there is
nothing to invalidate, and no window in which a reloaded file could be started with the code it
had before. `tests/shared_irep.rs` checks both directions, including a file replaced under its own
handle the way the asset server replaces one.

What it is worth, measured on 2026-09-20 with the survey's instrument (`taskset -c 2`, release):
three thousand entities of one 294-byte `.mrb` used to leave the VM holding 6000 ireps and now
leave it holding 2, the frame they all start in went from 7.56 ms to 5.11 ms (1.86 → 0.91 ms at a
thousand entities), and resident memory at the start went from 2.21 kB an entity to 1.46 kB. What
is left of the starting cost is `Vm::task_spawn` — a context and a stack for each script, which
they do not share.

**A program that will not load is remembered too**, and for the same reason: so that it is met
once. A `.mrb` that is a truncated download, or bytes another version's compiler wrote, used to be
logged and passed over, which left the entity exactly as `start_scripts` had found it — so the
next frame parsed the same bytes again, and wrote the same line again, once per entity per frame
for as long as the entity lived. Now the first meeting is the only one that parses and the only
one that speaks, and the entity is **told** the way every other script's entity is told: a
`ScriptEnded` of `ScriptStatus::Failed` whose value is what the VM said, sent once, with a
`ScriptDone` to mark the entity as one this VM is not going to start.

It is a pause, not a verdict. An asset that **changes** takes the mark off again — the same
handle with other bytes (an editor's Apply, the asset server's hot reload) or another handle
through `replace_script` — and the script starts if the new bytes load. `ScriptWorld::broken_programs()`
is how many distinct programs this VM could not load, the pair to `loaded_programs()`;
`tests/broken_script.rs` holds the whole of this, twenty frames to one ending.

**An irep is never given back.** SabiRuby 0.5.2 has no way to drop one — nothing in the crate
removes from `Vm::ireps` — so every *new* text a game starts a script from is one more irep for
the life of the app. That is not something rubevy can fix from the outside, and the table is what
keeps it from being worse: applying the same text again, or applying a change and taking it back,
costs nothing the second time. A game with an editor that applies a change every few seconds for
an hour should know the number it is spending; a game whose scripts are files on disk spends it
once each.

## A script the game compiles itself (`Program`, `in_the_authors_lines`)

A game whose scripts are `.mrb` files on disk needs none of this: Bevy's asset server reads them
and `Script::new(handle)` runs them. A game that lets a **player** write Ruby — an editor panel, a
file the player is invited to change — compiles that text itself, and then meets two things both
sample games wrote by hand.

The first is that the player's file is not the whole program. A game of this kind has a **prelude**
in front of it: the DSL the player writes in, the methods the game will call back. The two are
compiled as one program, which is why neither needs a `require` and why the player's file may call
the prelude's methods at the top level.

```rust
use rubevy::Program;

let program = Program::new(&prelude, "brain.rb", &players_text, "run");
// program.source        — hand this to whatever compiler this build has
// program.name          — what to call the file: the separator comment has it, and so does
//                         the compiler (`compile(&program.source, &program.name)`)
// program.prelude_lines — how far down that pushed the player's first line
```

The name is on the program because it is one name: before, every caller passed it to
`Program::new` for the separator comment and again to its compiler for the file name, and a name
written twice is a name that can be changed in one place only.

**Line numbers after it starts running are another matter.** A backtrace — the file and line an
exception carries, and what `ScriptWorld::stats` shows of a script's frames — is in the program
only if it was compiled with debug info, and `sabiruby_compiler::Options::debug_info` is `false`
by default. So a game that compiles with `Options { filename, ..Default::default() }` gets the
compiler's own messages in the player's lines (above) and, for an error the script raises while
it runs, no line at all. The browser's bridge defaults the other way: the playground's C ABI takes
the flag as an argument (`sabi_compile(src, len, debug)`) and its JS wrapper passes it on with
`compile(src, debug = true)`, so a page compiles with line numbers unless it says otherwise.
Asking for them costs the bytes the DBG section takes, in the binary and in the VM.

The second is that every line number the compiler then reports is a line of *that* program.
rubevy_games' garden reported `beetle.rb:600` for something its author had written on line 118,
and its sibling sabibots still reports numbers 295 lines too far down. `in_the_authors_lines` puts
them back:

```rust
use rubevy::in_the_authors_lines;

match compile(&program.source) {                      // the game's own compiler; see below
    Ok(bytes) => { /* … */ }
    Err(message) => {
        let shown = in_the_authors_lines(&message, program.prelude_lines, "prelude.rb");
        // "brain.rb:118:19: syntax error, unexpected '<'; …"
    }
}
```

It works on the **text**, and rubevy compiles nothing here. Both compilers a game can have print
`FILE:LINE:COL: message`, one diagnostic to a line — `sabiruby_compiler::Diagnostic`'s `Display`,
"as `mrbc` prints it" — and in a browser the compiler is a function the page defines, which is
handed a source and nothing else, so it prints the same thing with the file always called
`playground.rb`. So the only thing to find in a line is the `:LINE:COL:` in it, and the first one
on the line is the only one that can be it; what is in front of it is a file name, which may have
colons of its own.

Three things come back (`tests/source.rs` has each, with the real compiler's real message):

| where the compiler pointed | what is shown |
|---|---|
| past the prelude | the author's own line: `n - prelude_lines` |
| past the author's last line (the `tail`) | the line after the file's end, which is where the program's last statement is |
| inside the prelude | the prelude's name and the line it really is — not a number the author cannot find, and not a negative one |
| nowhere (`compile error`, a missing file) | the message, whole |

`prelude_lines` is counted off the text that really went in front, so it is right whether or not
the prelude ended with a newline of its own, and it is the same number a panel showing a script's
frames subtracts from the line the VM reports (rubevy_games' `VmInspector::fill`).

## `require` out of the binary (`EmbeddedHost`, `rubevy-build`)

`require` reads the asset directory with `std::fs` (`FileHost`), and a browser has no filesystem:
a page is served a wasm module and whatever it fetches, and none of that is a path. `EmbeddedHost`
is the same road with a table in place of the directory.

```rust
use rubevy::{EmbeddedHost, ScriptWorld};

// what the build script wrote
include!(concat!(env!("OUT_DIR"), "/ruby_files.rs"));   // pub static RUBY_FILES: &[(&str, &str)]

fn embed_the_scripts(mut world: ResMut<ScriptWorld>) {
    world.require_from(
        EmbeddedHost::new(RUBY_FILES).compile_with(|src, opts| page_compile(src, opts)),
        &["ruby"],
    );
}
app.add_systems(Startup, embed_the_scripts);
```

The paths in the table are the paths a script requires: the VM joins a load path and the name the
script wrote (`"ruby"` + `"helper"` + `".rb"`) and the host looks that text up, so the keys are
paths relative to the crate with `/` in them. A leading `./` is taken off first.

**Which is why the host and the load path go in together.** `ScriptWorld::require_from(host,
load_path)` is `vm.set_host` and `vm.set_load_path` under one name, and both are still there for
a game that means only one of them. Installing the host alone leaves the VM asking for the paths
the *plugin's* host reads — `{asset_root}/scripts/helper.rb` — which the table does not hold, and
the script gets a `LoadError` naming a file that is in the binary all along. Nothing in the types
says so, and because the host being replaced is usually the browser's, it is a thing that happens
**in a browser only**, where the log is a console nobody has open. `EmbeddedHost` says so once
when it happens: an ask it could never have answered — a path whose first directory is not one
the table uses — is a `warn!` naming the directory asked for, the ones it holds, and the call
above. A miss on its own is not: `require` misses on the way to every hit (`.mrb` before `.rb`,
each load path in turn), and a host is never told which candidate is the last.

**A `.mrb` needs no compiler**: `with_binaries` takes a table of bytes, the VM sees the RITE magic
and runs them, and a build with neither the `ruby-source` feature nor a `compile_with` can still
`require` one. A `.rb` is source, so it needs a compiler, and `compile_with` is where a browser's
comes from — there is no C build in a page, and what the playground publishes is a function the
page defines. The closure has `Host::compile`'s own shape, because that is who calls it; a caller
that cannot honour the options' `line` and `scopes` (a browser bridge cannot: it is given a
source) may ignore them, and then `require` works and `eval` of a string that reads its caller's
locals does not. Without a `compile_with` the crate's own answer is used, which is the reference
compiler with the `ruby-source` feature and the same refusal `FileHost` gives without it.

**The table is the build script's** — `rubevy-build`, the second package in this repository:

```rust
// build.rs
fn main() {
    rubevy_build::Embed::new("ruby").write();
}
```

It walks `ruby/` for `.rb` files and writes `OUT_DIR/ruby_files.rs`: a `pub static RUBY_FILES` of
`(relative path, include_str!(absolute path))`, sorted, with a `cargo:rerun-if-changed` for the
directory and for every file in it. The name of the static, the file, the extensions and whether
they are text or bytes (`include_bytes!`, for `.mrb`) are all settable; the defaults are what both
sample games' build scripts wrote, which were the same 35 lines to the byte. It is **std and
nothing else** on purpose: a build-dependency is built for the build machine, and a build script
that named rubevy would build Bevy a second time for every game that embeds a script.

## Further reading

`rust-bridge.ja.md` walks through one question from a robot in SabiRuby Battle to the system that
answers it and back, and compares each step with what embedding the C mruby would take (in
Japanese).
