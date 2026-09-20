# Changelog

What changed in each release of `rubevy`. Every claim names the commit behind it (the merge, where
the work came in on a branch) and the document under `docs/` that records it; measurements are the
ones those documents carry, and nothing is estimated.

## Unreleased

* **Every number rubevy holds, written down — and the last one that could not be changed, made
  settable** (R10 of `docs/plans/generalize-plan.md`, on branch `generalize`; the merge's commit
  goes here when the branch comes in — `docs/numbers.md`,
  `docs/worklog/2026-09-20-numbers-inventory.md`). The `const`s and the numeric literals of
  `src/`, `rubevy-build/` and the two standing instruments were swept mechanically (33 `const`
  declarations, 361 lines holding a literal) and the **44** that are numbers or fixed names
  rather than indices, unit conversions and identities are now one table in `docs/numbers.md`:
  what each is, where an app changes it, and where its default came from. Each line of that
  table is also a paragraph in the rustdoc of the thing it is about, and the "Time" table in
  `docs/host-api.md` has gained a column for it. **No default moved**, and the tests pass
  unchanged on the defaults, which is what says so.
  **`ScriptWorld::max_depth()` / `set_max_depth()`** is the one number that had no way to be
  changed: how deep a value is followed across the boundary, `reflect::MAX_DEPTH` and 16 since
  the reflection bridge was written. The default is still 16 and an ordinary component is
  untouched; an app whose own components nest deeper now says so rather than reading nil, and
  one that wants a shallower boundary says that. It is a pair of methods and not a `pub` field
  because a write (`e[:X] = hash`) is read out of the VM by a **native**, which is handed
  `&mut Vm` and nothing else — so the number lives in the VM's host state, in one place, which
  is the same problem `queue_limit` had in the other direction. `tests/max_depth.rs` is the
  default reading and writing four levels down and a shallower setting stopping both.
  **Seven defaults have no recorded origin**, and the table says so rather than inventing one:
  `budget` 200,000, `frame_time` 8 ms and `overrun` 50 ms (stated, never explained, in the two
  commits that added them), `max_depth` 16, the `scripts` in the load path, and two of the
  instruments'. Against that, four numbers turned out to be **quoted from elsewhere** and not
  rubevy's at all — a script's default priority 128 and the clock's 4 ms tick are mruby-task's
  (`MRB_TASK_PRIORITY_DEFAULT`, `MRB_TICK_UNIT`), `"assets"` is bevy's asset directory, and the
  five types read as Arrays are bevy_reflect's. The tally's own finding: **not one of rubevy's
  defaults was chosen by measuring.** What has been measured is what a default *buys* — 2,600
  component reads in a frame's budget, 3,900 messages, 8.28 ms of tick — which is the material
  for choosing, and `docs/numbers.md` keeps the two apart. `queue_limit` stays at **64** by the
  author's decision, with the measured bounds around it (about 3,800–3,900 messages one
  subscriber can read in a frame, and 16–335 bytes a waiting message) written where the
  rustdoc used to imply a derivation there never was.
  No dependency was added, there is no `unsafe`, and the public API grew by two methods.

* **An optional Ruby layer, and the first one is a camera** (R9 of
  `docs/plans/generalize-plan.md`, merged in `57774f3` — `docs/worklog/2026-09-20-camera-layer.md`).
  rubevy carries Ruby it does not run: **`rubevy::layers::CAMERA`**, taken up by the app in one
  line at `Startup` with the new **`ScriptWorld::load_and_run(&[u8])`** (which runs any `.mrb` in
  the VM the way the prelude is run, and takes a library of the game's own just as well). An app
  that does not load it has no `Rubevy::Camera` — that is what makes it a layer and not the host
  API. **Nothing in `src/` knows what a camera is and nothing has started to**: the layer is
  `src/layers/camera.rb`, Ruby over `Rubevy.find`, `e[:Transform] =`, `e[:Projection] =` and
  `Rubevy.ask`, and the crate's dependencies are unchanged.
  `Rubevy::Camera.find` / `.find_all` / `.attach`, then `position`, `move_to`, `pan`, `zoom`,
  `scale`, `follow` / `unfollow`, `world_at`, `reload` and `rejected_writes`. **`zoom 2` is twice
  as close on a 2D camera and on a 3D one alike**, which is most of the reason it exists: an
  orthographic camera zooms by `scale` (divided by the factor) and a perspective one by `fov`,
  where what sets apparent size is `tan(fov / 2)` — so the layer writes
  `2 * atan(tan(fov / 2) / factor)`, and not `fov / factor`, which would be wrong by more and
  more as the angle grew and could pass pi. It remembers the projection's variant (Ruby cannot
  switch one), the magnification (a write lands at the end of the frame, so `zoom 2` twice in a
  tick would otherwise come to 2) and where it last put the camera this frame (so two `pan`s add
  up). `world_at` is thrown at the game and the **queue** handed back unwaited: no system in
  rubevy answers `camera.world_at`, and a `pop` on it would park that task for ever.
  `examples/camera_from_ruby.rs` is the same example as before, rewritten through the layer and
  with the answering side of `world_at` in it; `tests/camera_layer.rs` is eleven tests, and
  `docs/host-api.md` has "An optional layer, and the first one".

* **A script can ask what became of a write the world would not take** (before R9 of
  `docs/plans/generalize-plan.md`, merged in `57774f3` — `docs/worklog/2026-09-20-rejected-writes.md`).
  A write lands at the end of the frame, so `e[:X] = hash` has always answered the hash it was
  handed whether the world took it or not, and what the host had to say about a refusal — the
  type is not registered, the entity has no such component, the enum is in another variant, a
  field cannot take what it was given — it said to its log, where no script can read it. The
  camera work of R0 found this twice over: a `zoom` into the wrong variant of `Projection`
  changes nothing and says nothing. Now a frame's refusals are kept and answered:
  **`Rubevy.rejected_writes`** hands a script an Array of `{entity:, name:, reason:}` for the
  writes **it** made — the entity of the task that asked is what says which those are, and
  `entity` is what was written *to*, nil for a resource — and **`ScriptWorld::rejected_writes()
  -> &[RejectedWrite]`** is the host's wider view, every script's, each carrying `by` beside
  `on`. The `warn!` is unchanged and carries the same sentence, because a game's log is where a
  game already looks.
  **The list is one frame's worth, and it is replaced by the next frame that writes** rather
  than emptied by every frame: a script cannot ask to be woken on the very next one (`sleep 0`
  waits for the VM's clock, which moves in whole ticks of 4 ms), so a list that lasted one frame
  would usually be gone before the script that wrote could look at it. It has **no limit and
  needs none** — a write costs instructions, so the frame's `budget` bounds the count: 33
  instructions each, some six thousand under the default budget, measured by
  `tests/rejected_writes.rs::what_one_rejected_write_costs`, which also shows the budget doing
  the bounding (a script asked for 1,100 and its last frame held 524). It is deliberately **not**
  in `FrameStats`: every number there is one the tick already had, and the writes are applied in
  two systems after the tick. Nothing was added to the crate's dependencies, there is no
  `unsafe`, and the whole of it is `tests/rejected_writes.rs` (7 tests and one `#[ignore]`d
  instrument) with the section "A write the world would not take" in `docs/host-api.md`.

* **What a frame's tick came to, and two instruments that measure what a machine carries** (R5 of
  `docs/plans/generalize-plan.md`, merged in `3e215c1` — `docs/worklog/2026-09-20-frame-stats.md`, `docs/verification/scale.md`).
  A game could ask what one script had spent (`ScriptWorld::stats`) and nothing at all about the
  frame the scripts share: how long the tick took, how much of the budget it used, how many
  questions it answered and how many it had to put off, what its queues lost. Each of those is a
  number the tick was already keeping or a clock reading the deadline already made it take, so
  **`ScriptWorld::last_frame() -> FrameStats`** gathers them rather than measuring anything new —
  twelve fields, listed in `docs/host-api.md` ("What a HUD can show of a frame"). Two of them are
  worth naming here: `time_ns` is the tick alone, which is the number `frame_time` bounds and
  **not** what a system placed after `RubevySet::Tick` measures (that set holds four systems, and
  the difference on one machine with a thousand scripts is a millisecond); and `longest_answer_ns`
  is the dearest single answer of the tick, which is the overshoot a late tick is allowed — the
  way to find the slow `answer_in_tick` closure that R4 could only warn about. It is `None` where
  the app set no `frame_time`, because then the tick reads no clock and this adds none.
  **What collecting it costs was measured rather than assumed**: the same instrument built
  against the commit before and the commit after, paired and repeated on a quiet machine, differs
  by −5.2% to +1.8% with a thousand and with three thousand scripts — scattered both ways and
  inside the spread of the before version against itself — so there is **no setting to turn it
  off**, which would have been a number and a knob for a cost nothing could measure.
  **The two measuring instruments the survey of 2026-09-20 wrote are now examples**:
  `examples/how_many_scripts.rs` (scripted entities, ten to three thousand, with the tick both as
  rubevy times it and as a game would, the frames between one script's turns, and a `mem` mode in
  a process of its own) and `examples/how_many_subscribers.rs` (subscribers × messages a frame,
  what one reader can take as the queue limit moves, and what a waiting message costs). They
  assert nothing, they are native only, every size they run with is an argument whose default is
  written down with where it came from, and the three holes the earlier stages found in them are
  closed: a column that was mostly the instrument's own two clock reads is gone (and what one
  such pair costs is measured and printed), the two resident-memory columns that moved by
  hundreds of kilobytes between identical runs are a mode of their own, and the row where a
  single repeat came out 2.3 times its neighbours now takes repeats. `docs/verification/scale.md`
  is a run of both on one machine with the before-and-after of R1–R4 beside it, and the
  reproduction steps carry the four traps the stages walked into. 150 tests pass (140 before, of
  which 9 are new and one is the example in the new rustdoc), no dependency was added, there is
  no `unsafe`, and the only public names added are `FrameStats` and `last_frame`.

* **`frame_time` is a limit on the tick, not only on the runs of the VM inside it** (R4 of
  `docs/plans/generalize-plan.md`, merged in `270b316` — `docs/worklog/2026-09-20-frame-time-as-a-limit.md`). A tick is a loop — run
  the ready tasks, answer what they parked on, run them again — and the clock was looked at only
  at the head of a round. The answering itself was not measured against anything, so a round that
  had parked three thousand tasks made three thousand answers however late the frame already was:
  with three thousand scripts, a `frame_time` of 8 ms gave a tick of 14.9 ms and one of 1 ms gave
  3.3 ms, two to four times what the app asked for. Now the tick looks at the clock after every
  answer and puts what it has not reached to the next frame, down the road the leftovers of a
  spent frame already took — in order, and taken **before** anything the next frame asks, so a
  question that is put off is not put off again. The same runs now give **8.3 ms** and
  **1.4 ms**. The clock is the one the VM itself was given (`clock_ns`, Bevy's `Instant`, which a
  browser has), and it is read only by an app that set a `frame_time`: with `frame_time = None`
  nothing about this happens and every question of the round is answered as before (measured:
  no change outside the run-to-run spread).
  **What a script pays for it** is one frame, for the questions left at the tail of a frame that
  ran out: `e[:Transform]` still answers in the line that asked it, except there. **What a tick
  can still pass `frame_time` by** is written down rather than glossed — one answer (a late tick
  always makes one, or a run of the VM that spent the whole frame time would leave every task
  parked for ever), the VM's own notice of the deadline (three thousand tasks that ask nothing at
  all still take 2.9 ms under a 1 ms `frame_time`, inside SabiRuby's scheduler), a second VM, and
  the systems of the frame that are not the tick. The rustdoc of the field and `docs/host-api.md`
  ("Components by name", "Answering inside the tick", "Time") say all of it, and the "Time"
  section now also says that a game can read `Time<Virtual>` by name.
  **How often the clock is looked at was measured, not chosen**: one read of it is 16.7 ns on the
  machine of the day against 2.2–2.4 µs for one answer, so looking after every answer costs under
  a percent of an answer and nothing measurable in a frame — which is why this stage adds **no
  new number** to the crate. 140 tests pass (135 before), no dependency was added, there is no
  `unsafe` and the public API is unchanged.

* **The four small things the first users of the shared entry points found** (R6b of
  `docs/plans/generalize-plan.md`, merged in `7de5ca2` — `docs/worklog/2026-09-20-entry-point-followups.md`). R6 gave both sample games
  one place for the code they had each written by hand; using it turned up four things, and all
  four are additions — no name and no meaning changed.
  **`ScriptWorld::require_from(host, load_path)`** is the host and the load path as one act.
  A game that replaces the host has to replace the load path too — the plugin's points at the
  asset directory, an embedded table's paths are its own — and nothing in the types said so, so
  forgetting it is a `require` that raises `LoadError` for a file that is in the binary all along,
  **in a browser only**, where the log is a console nobody has open. `vm.set_host` and
  `vm.set_load_path` are untouched and still the way to do one of them. `EmbeddedHost` also says
  it once if it happens: an ask it could never have answered — a path whose first directory is
  not one the table uses — is one `warn!` naming that directory, the ones it holds and the call
  above (a plain miss is not, because `require` misses on the way to every hit).
  **`Program::name`** is what to call the file. It was passed to `Program::new` for the separator
  comment and then again to the compiler by every one of the three callers in rubevy_games, and a
  name written twice is a name that can be changed in one place only.
  **A `.mrb` that will not load is met once.** `start_scripts` used to log it and pass over the
  entity unchanged, so the next frame parsed the same bytes again and wrote the same line again —
  once per entity per frame, for as long as the entity lived, in the log the game's own messages
  had to be read out of. Now the VM remembers the programs it could not load by the same key it
  remembers the ones it could (the bytes), so the parse and the line happen once however many
  entities carry that program, and each entity is **told** the way every other entity is told: a
  `ScriptEnded` of `ScriptStatus::Failed` carrying what the VM said, once, with a `ScriptDone`
  marking it as not to be started again. It is a pause and not a verdict — an asset that changes
  under its own handle (an editor's Apply, a hot reload) or through `replace_script` takes the
  mark off and the script starts if the new bytes load. `ScriptWorld::broken_programs()` is the
  count, the pair to `loaded_programs()`; the table it counts needs no cap for the reason the
  loaded one needs none (the key is the program, so only a *distinct* program that will not load
  can add to it, and it costs less than the asset it came from). `tests/broken_script.rs` is six
  tests, the first of which runs twenty frames and expects one ending.
  And **one line of documentation**: a line number in an exception a script raises while it runs
  is in the program only if it was compiled with debug info, which
  `sabiruby_compiler::Options::debug_info` is not by default — while the playground's browser
  bridge passes it as an argument and its JS wrapper defaults it to on. `docs/host-api.md` carries
  that beside `Program`. 135 tests pass (121 before), no dependency was added, there is no
  `unsafe` and no new number in the crate.

* **A queue that overflowed says so, and how much it holds is the app's to say** (R3 of
  `docs/plans/generalize-plan.md`, merged in `7de5ca2` — `docs/worklog/2026-09-20-overflow-and-limits.md`). A subscriber's queue held
  sixty-four messages and dropped its oldest past that, in silence and at a number nothing could
  change: `QUEUE_LIMIT` was a `const`, and what it dropped left no log, no counter and nothing a
  script could notice — a `pop` that skipped forty messages looks exactly like one that skipped
  none. Three things are new. `ScriptWorld::dropped()` is how many messages the VM has dropped
  since it started, for a game's HUD. `Rubevy::Subscription#dropped` is the same count for one
  subscription, which is the one a script can act on. And the limit is now
  `ScriptWorld::queue_limit`, a `pub` field beside `budget` (so a second VM has its own, as it
  has its own budget), with `Rubevy.subscribe(:belt, limit: 512)` for a subscription that knows
  its own stream — one or more, anything else an `ArgumentError` where the script wrote it. The
  limit is read where a message is published rather than where a script subscribed, so an app
  that moves the field moves every subscription that did not name its own, and there is one
  number to keep rather than a copy of it per subscriber. **The default is still 64 and nothing
  about a running game changes.** What that 64 was missing is a reason: the record that chose it
  (2026-09-15) gives a qualitative one only. R3 measured the two things it can be derived from,
  on the machine and settings of `docs/worklog/2026-09-20-factory-survey.md` (`taskset -c 2`,
  release, 60 frames): one subscriber can pop about **3,900 messages in a frame** before the
  frame's 200,000 instructions stop it (51 instructions a message; the limit, not the budget, is
  what the survey's "64 a frame" was measuring), and a message waiting in a queue costs **16–21
  bytes** as a number, **300–340** as a short string and **330–520** as a small array — so a
  thousand subscriptions that never read hold 1.0 MB of numbers or 20.5 MB of short strings at a
  limit of 64. The worklog carries the sums those two make, and the choice of default is the
  author's. `tests/events.rs` gains four tests (the counts after an overflow, the field moving
  what a script is handed, a subscription's own `limit:`, and the four ways a `limit:` is
  refused) and `tests/two_vms.rs` one (each VM's limit and dropped count are its own). Nothing in
  the public API was taken away: `ScriptWorld::<()>::QUEUE_LIMIT` stays as the **default** the
  field starts at, and says so in its rustdoc. Nothing was added to `[dependencies]`.
  **What counting costs**, measured before and after on a quiet machine, twice, interleaved: a
  publish into a queue that is *not* full is unchanged (−20% to +5% across those cells, several
  of them faster), and a publish that has to drop something is **10–14% dearer** — about 12 ns a
  dropped message, which is what reading and writing the count on the queue object costs. A game
  that is not overflowing pays none of it; a game that is overflowing pays it to find out.

* **One program, one irep** (R2 of `docs/plans/generalize-plan.md`, merged in `fa1b7e3` —
  `docs/worklog/2026-09-20-one-irep-per-program.md`). `start_scripts` used to hand `Vm::load` the
  bytes of every `Script` it turned into a task, so a thousand entities running one `.mrb` put a
  thousand copies of the same instructions in the VM (the survey of 2026-09-20 measured 2.2 kB
  and 2.5 µs an entity). The plugin now keeps the programs it has loaded, by the bytes of the
  program itself, and spawns every task of one program from the one irep — which is all
  `Vm::task_spawn` ever needed, and needs nothing of SabiRuby that was not already there. The key
  is the program and not the asset it arrived in because a game that compiles a player's Ruby
  adds a new asset every time, and garden compiles a species' file again for every animal born
  into it; the key is the whole program and not a hash of it so that two different programs can
  never meet in the table. Measured with the survey's own instrument on the machine and settings
  of `docs/worklog/2026-09-20-factory-survey.md` (`taskset -c 2`, release, 90 frames × 3, before
  and after taken the same day): the VM holds **2 ireps whatever the number of entities** where it
  used to hold two per entity (6000 at three thousand), the frame all of them start in goes
  1.86 → 0.91 ms at a thousand and 7.56 → 5.11 ms at three thousand, and resident memory at the
  start goes 2.21 → 1.46 kB an entity (three thousand entities of a 294-byte `.mrb`: 28.9 → 26.3 MB,
  and 49.1 → 47.1 MB once they have run). The steady frame time is unchanged, which the code says
  too — after the starting frame `start_scripts` walks an empty query. The survey's "2.2 kB an
  entity, the copy of the irep" turns out to have been the irep *and* the task: 0.75 kB of it was
  the irep, and that is the part this gives back.
  Nothing in the public API changed except one addition, `ScriptWorld::loaded_programs()`, and
  nothing was added to `[dependencies]`. `tests/shared_irep.rs` (6 tests) checks that a hundred
  entities load one program once, that the same text in another asset is the same irep, that two
  tasks of one irep cannot reach each other through its string literals, and that a replaced or
  reloaded asset runs its new text. What it cannot fix is that **SabiRuby has no way to give an
  irep back** (0.5.2 never removes from `Vm::ireps`), so a game that applies a *new* text over and
  over still spends one program's ireps each time; the table is what keeps applying the *same*
  text again from costing anything, and `docs/host-api.md` says so.

* **Subscriptions are filed by name** (R1 of `docs/plans/generalize-plan.md`, merged in `fa1b7e3` —
  `docs/worklog/2026-09-20-subscription-index.md`). `ScriptWorld::publish` used to walk every
  standing subscription of the VM to find the ones listening for a name, so publishing cost the
  number of subscriptions whether anybody was listening or not: 228 ns a message at a thousand
  subscriptions, for a name nobody had subscribed to. The subscriptions now sit in a map from
  name to the subscribers of that name, in the order they asked, and publishing is one lookup.
  Measured with the survey's own instrument on the machine and settings of
  `docs/worklog/2026-09-20-factory-survey.md` (60 frames × 3, `taskset -c 2`, release), a message
  to a name nobody subscribed to costs 7.9 → 8.1 → 31.3 → 228.0 ns as the subscriptions go
  1 → 10 → 100 → 1000, and afterwards 13.8 → 9.2 → 8.9 → 9.6 ns: no longer a function of how many
  subscriptions the VM holds. Publishing to a name that *is* heard, the frame time and the number
  of messages each script read are unchanged (within the ±2% the instrument repeats to; the
  one-subscriber cell is noisier than the difference — the worklog says what was and was not
  settled there). Nothing in the public API changed, and nothing was added to `[dependencies]`:
  the map is `std::collections::HashMap`, which the crate already used in three places.
  `tests/events.rs` gains one test for the shape the filing gives Ruby (one script listening for
  several names, and within a name the order it subscribed in).

* **Resources by name** (R8 of `docs/plans/generalize-plan.md`, merged in `fa1b7e3` —
  `docs/worklog/2026-09-20-resources-by-name.md`). `Rubevy.resource(:Score)` reads a resource as
  a Hash of its fields and `Rubevy.set_resource(:Score, { points: 8.0 })` writes the fields it
  names — the component road with the entity left out of it, and the same rules throughout: the
  read is answered inside the tick that asked it (no frame), the write lands at the end of the
  frame (`apply_resource_writes`, beside `apply_component_writes`), an unreachable name is `nil`
  rather than an error, and the game never sees the question. A type is reachable with
  `#[derive(Resource, Reflect)]`, `#[reflect(Resource)]` and `register_type`: in Bevy 0.19 a
  resource *is* a component on an entity of its own, so `ReflectResource` — a marker with no
  functions at all — is taken as the type's word that it is a resource, and
  `Rubevy.resource(:Transform)` is `nil`. Names are the component's names, so a generic type
  carries its parameters (`"Time<Virtual>"`, and `Time` is `Time<()>`). `tests/resources.rs`
  (8 tests, including a second VM under a name tag) and the `#[ignore]`d
  `tests/read_cost.rs::a_resource_read_beside_a_component_read`, which measured the two reads at
  the same 2.2–2.7 µs and the Ruby of the asking 6 instructions apart. Nothing was added to
  `[dependencies]`; the public Rust API is unchanged.

* **Three things R0 wrote down, fixed** (same branch and worklog). A write refused at the top of
  a component no longer reports itself with a stray colon (`was not written whole: : the fields
  of …`): `src/reflect.rs` now leaves the prefix off where there is no path inside the value, as
  `join` always did. `docs/host-api.md` says how a tuple variant's fields are written (the Array
  the read answers, never by name) and splits the two enum rows of the read table. And the claim
  that `DefaultPlugins` registers Bevy's own types is replaced by what the sources say: in bevy
  0.19.1 it is the `reflect_auto_register` feature, part of bevy's default features and off in
  this crate, while a few plugins (`TimePlugin`) still register by hand — which is what
  `tests/resources.rs` reads `Time<Virtual>` under `MinimalPlugins` to show.

* **The entry points both sample games had written by hand** (R6 of
  `docs/plans/generalize-plan.md`, merged in `fa1b7e3` — `docs/worklog/2026-09-20-shared-entry-points.md`). Five of them, for a game
  that compiles a player's Ruby itself:
  * `Program::new(prelude, name, body, tail)` builds one program out of a prelude and an author's
    file and says how far down that pushed the author's first line. The number is counted off the
    text that really went in front, so it is right whether or not the prelude ended with a newline
    (both games' `prelude.lines().count() + 2` assumed it did).
  * `in_the_authors_lines(message, prelude_lines, prelude_name)` takes the prelude back off the
    line numbers a compiler reported — the author's own line, the prelude by name when the error
    is in it, the line past the end for the `tail`, and a message with no place in it handed back
    whole. It reads text and compiles nothing, and `tests/source.rs` checks it against what
    `sabiruby_compiler` really prints and against the same message under the name a browser's
    bridge gives it (`playground.rb`).
  * `replace_script(&mut commands, entity, script)` is the swap both games wrote by hand:
    `ScriptTask` off, `ScriptDone` off, the new `Script` in.
  * `EmbeddedHost` serves `require` out of tables built into the binary — `&[(&str, &str)]` of
    source, `&[(&str, &[u8])]` of `.mrb` — which is the only `require` a browser can have. What
    compiles a `.rb` is given with `compile_with` (a page's own compiler) and otherwise is the
    crate's, which is now shared with `FileHost` so the two cannot disagree.
  * `rubevy-build`, a second package in this repository (std only, no dependencies), is the build
    script that writes those tables. The repository is a workspace for it; `cargo test` and
    `cargo package` at the root are unchanged and `Cargo.lock` gains four lines.

  `docs/host-api.md` has the two new sections and `Replacing and removing a script` now names the
  function. `src/` gained no dependency and nothing a browser lacks
  (`tests/no_wasm_unsupported.rs`); the public API only grew.

* **A camera driven from Ruby, with nothing added to the crate** (R0 of
  `docs/plans/generalize-plan.md`, merged in `fa1b7e3` — `docs/worklog/2026-09-20-camera-from-ruby.md`). `Camera2d`, `Camera3d` and `Projection` are
  ordinary `#[reflect(Component)]` types, so `Rubevy.find(:Camera2d)`, `cam[:Transform] =` and
  `cam[:Projection] = { Orthographic: [ { scale: 2.0 } ] }` already pan and zoom one. The new
  `examples/camera_from_ruby.rs` and `tests/camera.rs` say so and hold the shape of the write;
  `src/` is unchanged and `[dependencies]` is unchanged (the example and the test ask for bevy's
  `bevy_camera` feature as a dev-dependency). What a Ruby camera layer will be built on is in the
  worklog, including the two shapes that are refused: switching an enum's tuple variant, and
  naming a tuple variant's field instead of giving the Array.

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
