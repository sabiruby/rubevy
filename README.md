# rubevy

Ruby scripting for [Bevy](https://bevy.org/) (0.19): a script is mruby bytecode running on the
[SabiRuby](https://crates.io/crates/sabiruby) VM as one **task**, with a host API onto the ECS —
components and resources by name, questions the game answers, events as queues.

This page says what is in the crate now. Which release each thing arrived in, and the commit
behind every claim, is
[`CHANGELOG.md`](https://github.com/sabiruby/rubevy/blob/main/CHANGELOG.md).

## What works

### Scripts as tasks

* `.mrb` files load as `MrbAsset` through Bevy's asset server (the ones in this repository are
  compiled with mruby 4.1's `mrbc`; `sabiruby-compiler` writes the same RITE binary).
* **One VM for the app by default, one task per script** — and a second VM under a name
  tag where a game wants isolation (below). A `Script` component becomes a task of
  mruby-task's scheduler; every frame the plugin moves the scheduler's clock on by the
  frame time and lets the ready tasks run for a budget of instructions. A task that
  calls `sleep` costs nothing until its time comes, and one that never yields is
  preempted at its timeslice, so a frame cannot be lost to a runaway script.
* **Priorities**: `Script::with_priority` (0 first, 128 by default), mruby-task's.
* **One irep per program, however many entities run it**: the plugin keeps the programs it has
  loaded by their bytes, so a thousand entities on one `.mrb` load it once
  (`ScriptWorld::loaded_programs`). A `.mrb` that will not load is reported once rather than
  every frame (`broken_programs`), and tried again if the asset changes.
* **Replacing and stopping**: `replace_script(&mut commands, entity, script)` swaps a script;
  `stop_script(&mut commands, entity)` stops one (removing `ScriptTask` alone only restarts it).
* The collector runs at the scheduler's idle points (`GC.scheduler_driven`).

### From a script to the game and back

* **A host API**: `Rubevy.log`, `.spawn`, `.despawn`, `.set_position`, `.move_to`, and
  `.entity` (the entity this script is attached to, as a `Rubevy::Entity` object that
  carries `Entity::to_bits` as a handle — `to_i` for the number, `==` by it). A native
  cannot touch the Bevy world, so these leave a command behind — on a queue inside the
  VM (`Vm::set_host_state`), one per `App` — and a system carries it out after the
  frame's scripts have run, the same promise `Commands` makes.
* **A question the game answers when it can**: `Rubevy.ask("scan", 40)` parks the script's
  task on a queue, a system of the game takes the request and answers it on that frame or a
  later one. An argument may be a Hash or an Array as well as a number, a string or an entity
  — the value itself reaches the host (`Arg::Value`), kept alive for as long as the request is
  and let go of with it. Where the answer is work rather than a lookup, `ScriptWorld::answer_with` takes
  a `Future` instead: it goes to Bevy's task pool and the plugin answers the script on the
  frame it finishes. Bevy's pools are threads only with bevy's own `multi_threaded` feature —
  this crate does not ask for it, the app does (`docs/host-api.md`).
* **A question answered in no frame at all**: `ScriptWorld::answer_in_tick("nearest", …)` gives a
  kind of `Rubevy.ask` to a closure that the tick calls between two runs of the VM, with the world
  as it stands in `RubevySet::Tick` — the road a component read already takes. It is for the
  spatial questions a script asks every frame ("who is nearest", "what is within 4 m"), which a
  system answers a frame late and Ruby cannot afford to walk itself (`cargo run --example
  nearest`, `docs/host-api.md`).
* **A question whose answer is an action that takes frames**: `App::hold_requests("walk_to")`
  says that a kind of `Rubevy.ask` belongs not to `take_requests` but to the entity that asked —
  it arrives as a `Held` component there, `Added<Held>` is where the game starts the action, and
  `held.answer(&mut scripts, "walk_to", …)` ends it. rubevy does the tidying up: answered,
  despawned, replaced, ended, raised, overran — every way the waiting can end takes the waiting
  questions with it (`cargo run --example walk_to`).
* **A question written as a call**: `require "proxy"` gives a script `Rubevy::Proxy`, whose
  `robot.move_to(1, 2)` is `Rubevy.ask("robot.move_to", 1, 2).pop`. Registering a real method
  is the plain way; a proxy is for objects the game did not register (`docs/host-api.md`).

### The world by name

* **Components**: `e[:Transform]` answers a Hash of the component's fields,
  `e[:Transform] = tf` writes back the fields the Hash names, and `e.has?`, `e.components`
  and `Rubevy.find(:Npc)` say what is where. It goes through Bevy's reflection, so no type
  is named in rubevy: a game's own component joins in with `#[derive(Reflect)]`,
  `#[reflect(Component)]` and `register_type` (`docs/host-api.md`).
  **A read costs no frame**: the tick runs the scripts, answers the reads they stopped on out
  of the world it is holding and runs them again, so the value is there in the line that asked
  for it — about 2.2 µs a read, and 2,667 of them in one frame where there used to be room
  for one (`docs/worklog/2026-09-17-sync-reads.md`). A write still lands at the end of the
  frame, as `Rubevy.spawn` does.
* **Resources**: `Rubevy.resource(:Score)` and `Rubevy.set_resource(:Score, { points: 8.0 })`
  are the same road with the entity left out, for a type with `#[reflect(Resource)]`. A generic
  carries its parameters: `Rubevy.resource("Time<Virtual>")`.
* **A write the world would not take** says so: `Rubevy.rejected_writes` hands a script the
  refusals of the writes **it** made (the type is not registered, the entity has no such
  component, the enum is in another variant), and `ScriptWorld::rejected_writes()` is every
  script's, for a game's HUD.
* **Events**: `Rubevy.subscribe(:hit)` answers a queue the game pushes onto
  (`ScriptWorld::publish`), so a script waits for something to happen exactly as it waits
  for an answer — in its own task, or in one it made with `Task.new`, which is what a reflex
  wants. A game joins one of Bevy's events with one observer. How much a queue holds is the
  app's (`ScriptWorld::queue_limit`) or the subscription's own (`Rubevy.subscribe(:belt,
  limit: 512)`), and what it dropped is counted (`ScriptWorld::dropped`,
  `Rubevy::Subscription#dropped`).

### The frame

* The script reads `$rubevy` (`:frame`, `:delta`, `:time`); `puts`/`p` go to Bevy's log.
* **Waiting one frame exactly**: `Rubevy.next_frame` parks the task until the head of the next
  tick and answers that frame's number; `Rubevy.each_frame { |dt| … }` is the loop around it.
  (`sleep` is a length of real time on a clock that moves in whole ticks of 4 ms, which is not
  the same promise.)
* **The limits are the app's**: `ScriptWorld::budget` (instructions a frame), `frame_time` (a
  bound on the whole tick, not only on the runs of the VM inside it) with `overrun`,
  `queue_limit`, and `set_max_depth` for how deep a value is followed across the boundary. A
  budget of zero pauses the scripts, and a paused VM does not age. How to choose `budget` and
  `frame_time` for your own game is in `docs/host-api.md` ("Time"); every number the crate holds,
  with where its default came from — or that nobody wrote it down — is in `docs/numbers.md`.
* **What a frame and a script cost**: `ScriptWorld::last_frame() -> FrameStats` is twelve numbers
  the tick already had (its time, the budget it spent, the questions it answered and put off,
  what its queues lost), and `ScriptWorld::stats(&task)` is one script's — what a HUD can show.
* A `ScriptEnded` message carries what the task answered, or the exception it did not handle
  (mruby-task makes that the task's result, so one broken script does not stop the others), and
  `ScriptEnded::at` is the file and line it stopped on, in the numbers the author of that file
  can see.

### Around the crate

* **`require`** reads from the asset directory: `.mrb` always, `.rb` in a build with the
  `ruby-source` feature (which brings the reference compiler along). A build with no filesystem
  — a browser — serves `require` out of tables built into the binary instead (`EmbeddedHost`,
  `ScriptWorld::require_from`), and the build script that writes those tables is
  [`rubevy-build`](https://crates.io/crates/rubevy-build), the second package in this
  repository.
* **For a game that compiles the player's own Ruby**: `Program::new` puts a prelude in front of
  an author's file and says how far down that pushed the author's first line, and
  `in_the_authors_lines` takes the prelude back off the line numbers the compiler reported.
* **Optional Ruby layers**: the crate carries Ruby it does not run. `rubevy::layers::CAMERA`,
  taken up in one line with `ScriptWorld::load_and_run`, gives scripts `Rubevy::Camera` — `pan`,
  `zoom` that means the same on a 2D and a 3D camera, `follow`, `world_at`. An app that does not
  load it has no `Rubevy::Camera`, and nothing in `src/` knows what a camera is
  (`cargo run --example camera_from_ruby`).
* **The browser**: the crate builds and runs for `wasm32-unknown-unknown` without
  `ruby-source`, and a test fails if `src/` names anything a browser lacks
  (`tests/no_wasm_unsupported.rs`).

Scripts share the VM, so they share globals and constants. That is the design; a use
that needs isolation — mods, a player's own script — gives that side **a second VM**.
A name tag is all it takes, and the app's own VM is spelled the way it always was:

```rust
struct Mods;                                          // the name tag; an empty struct
app.add_plugins(RubevyPlugin::default())              // the game's VM, unchanged
   .add_plugins(RubevyPlugin::<Mods>::for_vm("assets/mods"));   // and one for mods
// in a system: ResMut<ScriptWorld<Mods>>, and Script::<Mods>::for_vm(handle) on an entity
```

Heap, globals, constants, classes, symbols, GC, the scheduler, subscriptions, the frame
budget and `require`'s load path are then that VM's own, and handing one VM's `ScriptTask`
to the other is a compile error. What it costs is memory (~0.5 MB and one copy of every
`.mrb` per VM) and frame time (the budget is per VM, so the worst case is the sum). It is
isolation, not a sandbox — there is no heap cap yet, and the load path separates by name:
`docs/host-api.md`, "Two VMs in one app".

Not yet: building a component from Ruby to spawn with, queries as blocks
(`each(:Enemy, :Transform) { }`), hot reload that keeps a script's state, a heap
cap ([`docs/outlook.md`](https://github.com/sabiruby/rubevy/blob/main/docs/outlook.md)).

How a script and the game actually meet — `Rubevy.ask`, answering from a system, the clock, reading
where a script stands, stopping it — and what that gains over embedding the C mruby, is written up
in Japanese in
[`docs/rust-bridge.ja.md`](https://github.com/sabiruby/rubevy/blob/main/docs/rust-bridge.ja.md).
The whole of what a script can say to the game and the game to a script is
[`docs/host-api.md`](https://github.com/sabiruby/rubevy/blob/main/docs/host-api.md).

## Install

```
cargo add rubevy
```

```toml
[dependencies]
rubevy = "0.1"
bevy = "0.19.1"
```

The VM comes from crates.io with it: [`sabiruby`](https://crates.io/crates/sabiruby) 0.6.
Nothing else is required: a script that `require`s a `.mrb` works out of the box. An app that
names `sabiruby` itself must name the same 0.6 — two different sources for the VM are two VMs in
the binary, and a `Value` of one is not a `Value` of the other.

`ruby-source` is the one feature. It lets a script `require` a `.rb` as well, by bringing
[`sabiruby-compiler`](https://crates.io/crates/sabiruby-compiler) 0.3 (the reference mruby
compiler, built as C) along — so it wants a C toolchain, and it does not build for
`wasm32-unknown-unknown`. Without it, compile the `.rb` ahead of time (`tools/compile_scripts.sh`)
and ship the `.mrb`.

```toml
rubevy = { version = "0.1", features = ["ruby-source"] }
```

The version is 0.1: the API is still moving, and what each release contains — including what to
change when coming from 0.0.1 — is
[`CHANGELOG.md`](https://github.com/sabiruby/rubevy/blob/main/CHANGELOG.md).

## Try it

To work against a checkout of `sabiruby/sabiruby` next to this one, redirect it in
`.cargo/config.toml` (git-ignored) instead of editing `Cargo.toml`:

```toml
[patch.crates-io]
sabiruby = { path = "../sabiruby" }
sabiruby-compiler = { path = "../sabiruby/compiler" }
```

```
tools/compile_scripts.sh          # the .rb that ships here -> .mrb (Docker, reference mrbc)
cargo run --example headless      # MinimalPlugins + AssetPlugin + RubevyPlugin, no window
cargo run --example sensor        # Rubevy.ask, answered by a system two frames later
cargo run --example async         # Rubevy.ask, answered from a future on the task pool,
                                  # then the same question written as a call on a proxy
cargo run --example components    # a script reads its own Transform, moves it, writes it back
cargo run --example nearest       # a question the game answers inside the tick: 0 frames, beside
                                  # the same question answered by a system: 1 frame
cargo run --example walk_to       # a question that does not come back until the walking is over:
                                  # the request waits on the entity that asked it (Held)
cargo run --example events        # an observer publishes to a queue; a reflex task waits on it
cargo run --example two_vms       # a second VM for mods: the three things it cannot reach
cargo run --example camera_from_ruby   # the optional camera layer the app loads itself: pan,
                                  # zoom that means the same on a 2D and a 3D camera, and a
                                  # question about the window that the game answers
cargo run --release --example how_many_scripts       # measuring instruments, not tests: what this
cargo run --release --example how_many_subscribers   # machine carries (docs/verification/scale.md)
```

```rust
use rubevy::{MrbAsset, RubevyPlugin, Script};

app.add_plugins(RubevyPlugin::default());   // or ::with_asset_root("assets")
// in a system:
let mrb: Handle<MrbAsset> = server.load("scripts/npc.mrb");
commands.spawn((Script::new(mrb).with_name("npc").with_priority(100), Transform::default()));
```

```ruby
# assets/scripts/npc.rb
loop do
  Rubevy.move_to($rubevy[:time].sin * 5, 0, 0)
  sleep 0.1          # the task is off the CPU until then
end
```

## License

MIT.

## Outlook

What rubevy will build on SabiRuby's planned features, why it fits Bevy, and an honest assessment:
[`docs/outlook.md`](https://github.com/sabiruby/rubevy/blob/main/docs/outlook.md) (日本語の平易版:
[`docs/outlook.ja.md`](https://github.com/sabiruby/rubevy/blob/main/docs/outlook.ja.md)).
