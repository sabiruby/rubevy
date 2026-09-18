# Changelog

What changed in each release of `rubevy`. Every claim names the commit behind it (the merge, where
the work came in on a branch) and the document under `docs/` that records it; measurements are the
ones those documents carry, and nothing is estimated.

## 0.0.1 — 2026-09-18

The first release on crates.io (tag `v0.0.1`, `2d9eb92`). Until now the crate was used from git;
this entry says what the first published version contains, in the order it was built.
Bevy 0.19.1, SabiRuby 0.5.1 or later, Rust 1.95 (Bevy's own `rust-version`).

### Scripts as tasks

* **`.mrb` files are Bevy assets** (`MrbAsset`), and a `Script` component runs one (`b27d4f4`).
* **One VM for the app, one task per script** (`4c1e89f`). A script is a task of mruby-task's
  scheduler: every frame the plugin moves the scheduler's clock on by the frame time and lets
  the ready tasks run for a budget of instructions. A task in `sleep` costs nothing until its
  time comes; one that never yields is preempted at its timeslice. `Script::with_priority` is
  mruby-task's priority. A script ends once, and `ScriptEnded` carries what it answered or the
  exception it did not handle (`736178d`).
* **Limits per frame**: an instruction budget, and `frame_time` with `overrun`, on Bevy's clock
  (`0d0662b`). A paused budget stops the scripts' clock as well, so a pause does not spend a
  `sleep` (`fa37eaa`, `docs/worklog/2026-09-17-scheduling-and-vm-access.md`).
* **Stats**: what a script has spent, the line it stands on, the frames it stands in
  (`cdf3119`, `4f1b5ff`) — what a HUD can show of a script.
* **Removing a `ScriptTask` or despawning its entity stops the task** (`50dc862`), and replacing
  ten scripts in one frame no longer freezes the VM — the fix was SabiRuby 0.5.1's, the test
  that needed it is here (`3c6d3af`, `docs/worklog/2026-09-17-restart-burst.md`). This is why
  0.5.1 is the lowest SabiRuby the crate accepts.
* **`require`** reads from the asset directory: `.mrb` always, `.rb` with the feature
  `ruby-source`, which brings `sabiruby-compiler` (C) along.

### From a script to the game and back

* **One way**: `Rubevy.log`, `.spawn`, `.despawn`, `.set_position`, `.move_to` leave a command
  on a queue inside the VM's host state, and a system carries it out after the frame's scripts
  have run (`98a7d7d`). An entity is a `Rubevy::Entity` Data object, `==` by `Entity::to_bits`.
* **`Rubevy.ask`**: the script's task parks on a queue and a system of the game answers on that
  frame or a later one (`13d3210`). Arguments are numbers, strings, entities (`7ce43c2`), and a
  Hash or an Array as the value itself — `Arg::Value`, rooted until the request is dropped
  (`6fe63e0`, `docs/worklog/2026-09-16-arg-value.md`).
* **`RubevySet`** says where a game's answering system goes in the frame (`fa37eaa`); an answer
  may be an object of the game's own (`answer_value` with SabiRuby's `#[derive(RubyClass)]`).
* **`ScriptWorld::answer_with`** takes a `Future`: the work goes to Bevy's task pool and the
  script is answered on the frame it finishes (`7d63dbe`).
* **`ScriptWorld::answer_in_tick`** gives a kind of question to a closure the tick calls between
  two runs of the VM — no frame at all, for the spatial questions a script asks every frame
  (`ff6bf62`, `docs/plans/sync-access-plan.md` S4, `examples/nearest.rs`).
* **`Rubevy::Proxy`** (`require "proxy"`): `robot.move_to(1, 2)` is
  `Rubevy.ask("robot.move_to", 1, 2).pop` (`d689c64`).
* **The plugin uses only the VM's public entry points** — nothing reaches into `Vm`'s fields
  (`8c17f81`), and `funcall` was replaced by the VM's own API where it stood in for one
  (`9104f7c`). A task made with `Task.new` inherits the script's entity (`9104f7c`).

### Components and events

* **Components by name, through Bevy's reflection**: `e[:Transform]` answers a Hash of the
  fields, `e[:Transform] = h` writes the fields the Hash names, `e.has?`, `e.components`,
  `Rubevy.find(:Npc)`. No type is named in rubevy; a game's component joins with
  `#[derive(Reflect)]`, `#[reflect(Component)]` and `register_type` (`bb7c168`, `93311fb`).
* **A read costs no frame** (`4f77881`): the tick runs the scripts, answers the reads they
  stopped on out of the world it is holding, and runs them again. About 2.2 µs a read, 2,667
  in one frame where there used to be room for one; no `unsafe`, SabiRuby untouched
  (`docs/plans/sync-access-plan.md`). A write still lands at the end of the frame.
* **Events as queues**: `Rubevy.subscribe(:hit)` answers a queue the game pushes onto with
  `ScriptWorld::publish`; a game joins one of Bevy's events with one observer (`bb7c168`).
  Unsubscribing ends a task that is waiting on the queue (`9104f7c`).

### More than one VM

* **`RubevyPlugin::<Mods>::for_vm(dir)`** puts a second VM in the app under a name tag
  (`bfa47ed`, `docs/plans/multi-vm-plan.md`, `examples/two_vms.rs`). Heap, globals, classes,
  scheduler, subscriptions, budget and load path are per VM; handing one VM's `ScriptTask` to
  the other is a compile error. The app's own VM is spelled as before (`M = ()`). It is
  isolation, not a sandbox: there is no heap cap, and the load path separates by name.
* **The VM can be added to at `Startup`** (`ScriptWorld::vm`): a game installs its own classes,
  or `sabiruby-serde`'s `JSON`, into the VM its scripts run in (`fa37eaa`, `tests/vm_setup.rs`).

### The browser

* **Builds and runs for `wasm32-unknown-unknown`** (without `ruby-source`). The tick's answer
  loop keeps time on the VM's clock rather than `std::time::Instant`, which a browser does not
  have — the published garden was a black screen until it did (`e7b3eb5`,
  `docs/worklog/2026-09-18-wasm-instant.md`) — and a test fails if `src/` names anything a
  browser lacks (`bcb9cc0`).

### Packaging

* SabiRuby by version instead of git — `sabiruby` 0.5.1, `sabiruby-compiler` 0.2.2 (optional),
  and for the tests `sabiruby-serde` 0.1.0 — with `rust-version`, `repository`, `readme` and
  `categories` (`2d9eb92`, `docs/plans/publish-plan.md`, `docs/worklog/2026-09-18-crates-io.md`).
  The tests pass at those lowest versions and at 0.5.2 / 0.2.3 (75 passed, 0 failed).

### Not in it

Building a component from Ruby to spawn with, queries as blocks (`each(:Enemy, :Transform) { }`),
hot reload that keeps a script's state, a heap cap per VM (`docs/outlook.md`). This file itself is
not in the 0.0.1 package; it ships from the next release on.
