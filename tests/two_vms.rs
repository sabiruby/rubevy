//! Two VMs in one app, and the six ways they have to stay out of each other's way.
//!
//! The app adds `RubevyPlugin` twice, under two name tags, so everything the plugin owns exists
//! twice over: two `ScriptWorld`s, two sets of systems, two `RubevySet` chains. What the tests
//! below check is that the *scripts* of one never reach the other: not through a global, a class
//! or a constant; not through an exception; not through the frame budget; not through a task
//! ending; not through `ScriptStats`; and not even when both scripts sit on the same entity.
//! A seventh test is about the plugin rather than the scripts — the one thing the two VMs do
//! share is the `.mrb` asset, and the second plugin must not register it again.
//!
//! The name tags are deliberately empty structs that implement nothing at all (`struct A;`),
//! because that is what a game would write and because it is what the hand-written `Debug` /
//! `Clone` / `Hash` impls in rubevy exist for — a derive would have asked the tag for them.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, RubevyPlugin, RubevySet, Script, ScriptTask, ScriptWorld};

/// The first VM's name tag is `()`; these are the second and third. Nothing is implemented on
/// them on purpose — see the module comment.
struct A;
struct B;

fn compile(src: &str, filename: &str) -> MrbAsset {
    // `debug_info` so that `ScriptStats::location` has a file to name — one of the tests below
    // tells the two VMs' numbers apart by the file each script came from
    let opts = sabiruby_compiler::Options {
        filename: filename.into(),
        debug_info: true,
        ..Default::default()
    };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// What the scripts of each VM said, through questions the game answers but the script never
/// pops: `Rubevy.ask` is how a script tells the test something without waiting for it.
#[derive(Resource, Default)]
struct Marks {
    a: Vec<f64>,
    b: Vec<f64>,
}

fn answer_a(mut world: ResMut<ScriptWorld<A>>, mut marks: ResMut<Marks>) {
    for request in world.take_requests() {
        marks.a.push(request.num_or(0, -1.0));
        world.answer(&request, Answer::Nil);
    }
}

fn answer_b(mut world: ResMut<ScriptWorld<B>>, mut marks: ResMut<Marks>) {
    for request in world.take_requests() {
        marks.b.push(request.num_or(0, -1.0));
        world.answer(&request, Answer::Nil);
    }
}

/// Two VMs, neither of them the app's first one. `A` and `B` rather than `()` and `A` so that
/// nothing in these tests can pass by falling back to a default type parameter.
fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::<A>::for_vm("assets"),
        RubevyPlugin::<B>::for_vm("assets"),
    ))
    .init_resource::<Marks>()
    .add_systems(Update, answer_a.in_set(RubevySet::<A>::answer()))
    .add_systems(Update, answer_b.in_set(RubevySet::<B>::answer()));
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

fn start_a(app: &mut App, src: &str, filename: &str) -> Entity {
    let asset = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src, filename));
    app.world_mut().spawn(Script::<A>::for_vm(asset)).id()
}

fn start_b(app: &mut App, src: &str, filename: &str) -> Entity {
    let asset = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src, filename));
    app.world_mut().spawn(Script::<B>::for_vm(asset)).id()
}

/// A script that says its number every hundredth of a second, for ever.
const HEARTBEAT: &str = r#"
  n = 0.0
  loop do
    n += 1.0
    Rubevy.ask("beat", n)
    sleep 0.01
  end
"#;

// ---------------------------------------------------------------------------------------------

/// A global, a class and a constant one VM makes are three things the other cannot see. The
/// second script reports them as one number — 1 for `$x`, 2 for `Foo`, 4 for `CONST` — so the
/// failure says *which* of the three leaked.
#[test]
fn a_global_a_class_and_a_constant_do_not_cross() {
    const WRITER: &str = r#"
      $x = 1
      class Foo; end
      CONST = 7
      Rubevy.ask("wrote", 1.0)
    "#;
    const READER: &str = r#"
      seen = 0.0
      seen += 1.0 if $x
      begin
        Foo
        seen += 2.0
      rescue StandardError
      end
      begin
        CONST
        seen += 4.0
      rescue StandardError
      end
      Rubevy.ask("seen", seen)
    "#;

    let mut app = app();
    start_a(&mut app, WRITER, "writer.rb");
    frames(&mut app, 6);
    assert_eq!(app.world().resource::<Marks>().a, vec![1.0], "the first VM wrote its three");

    // only now, so that there is no question of which ran first
    start_b(&mut app, READER, "reader.rb");
    frames(&mut app, 6);
    let seen = app.world().resource::<Marks>().b.clone();
    assert_eq!(seen, vec![0.0], "the second VM saw 1=$x, 2=Foo, 4=CONST — it should have seen none");

    // and the control: the same reader inside the first VM finds all three, which is what says
    // the reader is looking for the right things
    start_a(&mut app, READER, "reader.rb");
    frames(&mut app, 6);
    assert_eq!(
        app.world().resource::<Marks>().a,
        vec![1.0, 7.0],
        "in the VM that wrote them, all three are there"
    );
}

/// An unhandled exception ends the task it was raised in. The VM next door does not notice.
#[test]
fn an_exception_in_one_vm_leaves_the_other_running() {
    const BOOM: &str = r#"
      Rubevy.ask("alive", 1.0)
      sleep 0.02
      raise "boom"
    "#;

    let mut app = app();
    let thrower = start_a(&mut app, BOOM, "boom.rb");
    start_b(&mut app, HEARTBEAT, "heartbeat.rb");

    frames(&mut app, 30);
    let task = *app.world().entity(thrower).get::<ScriptTask<A>>().expect("it started");
    assert!(
        app.world().resource::<ScriptWorld<A>>().stats(&task).finished,
        "the script that raised has ended"
    );

    let before = app.world().resource::<Marks>().b.len();
    frames(&mut app, 30);
    let after = app.world().resource::<Marks>().b.len();
    assert!(
        after > before,
        "the second VM beat {before} times before the exception and {after} after — it stopped too"
    );
}

/// A budget of zero pauses one VM's scripts. The other VM's budget is its own, so its scripts
/// keep their frames. (The plan calls this out as the reason not to invent a whole-frame budget:
/// the number means the same thing it always did, once per VM.)
#[test]
fn a_budget_of_zero_pauses_one_vm_and_not_the_other() {
    let mut app = app();
    start_a(&mut app, HEARTBEAT, "heartbeat.rb");
    start_b(&mut app, HEARTBEAT, "heartbeat.rb");

    frames(&mut app, 20);
    assert!(!app.world().resource::<Marks>().a.is_empty(), "the first VM beat before the pause");
    assert!(!app.world().resource::<Marks>().b.is_empty(), "the second VM beat before the pause");

    app.world_mut().resource_mut::<ScriptWorld<A>>().budget = 0;
    let (a_before, b_before) = {
        let marks = app.world().resource::<Marks>();
        (marks.a.len(), marks.b.len())
    };
    frames(&mut app, 40);
    let marks = app.world().resource::<Marks>();
    assert_eq!(marks.a.len(), a_before, "the paused VM ran an instruction");
    assert!(
        marks.b.len() > b_before,
        "the other VM beat {b_before} times and still {} — one VM's pause stopped it",
        marks.b.len()
    );
}

/// Ending one VM's task ends that task and nothing else. This is where an `ObjId` would do its
/// quiet damage: two VMs built the same way hand out the same heap indices, so the two
/// heartbeats below are very likely the *same number* in two different heaps, and terminating
/// one through the other's VM would kill a stranger. The name tag is what makes that a compile
/// error; this test is what says the runtime does the right thing as well.
///
/// Despawning rather than `remove::<ScriptTask<A>>()`, because removing the task alone is how a
/// game *restarts* a script — the `Script<A>` is still there and `start_scripts` makes a new
/// task of it on the next frame. (That is what the first draft of this test did, and it failed
/// with the script beating on happily: right behaviour, wrong question.)
#[test]
fn ending_one_vms_task_leaves_the_others_running() {
    let mut app = app();
    let a_entity = start_a(&mut app, HEARTBEAT, "heartbeat.rb");
    let b_entity = start_b(&mut app, HEARTBEAT, "heartbeat.rb");

    frames(&mut app, 20);
    let b_task = *app.world().entity(b_entity).get::<ScriptTask<B>>().expect("it started");
    let a_task = *app.world().entity(a_entity).get::<ScriptTask<A>>().expect("it started");
    let (a_id, b_id) = (a_task.task(), b_task.task());

    app.world_mut().entity_mut(a_entity).despawn();

    let (a_before, b_before) = {
        let marks = app.world().resource::<Marks>();
        (marks.a.len(), marks.b.len())
    };
    frames(&mut app, 30);
    let marks = app.world().resource::<Marks>();
    assert_eq!(marks.a.len(), a_before, "the task that was let go of kept beating");
    assert!(
        marks.b.len() > b_before,
        "the other VM's task went with it (the two tasks are {a_id:?} and {b_id:?})"
    );
    assert!(
        app.world().entity(b_entity).get::<ScriptTask<B>>().is_some(),
        "the other VM's entity still has its task"
    );
    assert!(
        !app.world().resource::<ScriptWorld<B>>().stats(&b_task).finished,
        "the other VM's task was terminated too (the two tasks are {a_id:?} and {b_id:?})"
    );
}

/// `ScriptWorld::stats` only takes the `ScriptTask` of its own VM — handing it the other's is a
/// compile error, which is the whole point of the name tag — and the numbers it answers are that
/// VM's. A busy script and an idle one, one in each VM, are told apart by both.
#[test]
fn each_vm_counts_its_own_scripts() {
    const BUSY: &str = r#"
      n = 0
      loop do
        500.times { n += 1 }
        sleep 0.01
      end
    "#;
    const IDLE: &str = "loop { sleep 0.05 }\n";

    let mut app = app();
    let busy = start_a(&mut app, BUSY, "busy.rb");
    let idle = start_b(&mut app, IDLE, "idle.rb");
    frames(&mut app, 40);

    let busy_task = *app.world().entity(busy).get::<ScriptTask<A>>().expect("it started");
    let idle_task = *app.world().entity(idle).get::<ScriptTask<B>>().expect("it started");
    let busy_stats = app.world().resource::<ScriptWorld<A>>().stats(&busy_task);
    let idle_stats = app.world().resource::<ScriptWorld<B>>().stats(&idle_task);

    assert!(busy_stats.instructions > 0, "the busy script ran nothing");
    assert!(idle_stats.instructions > 0, "the idle script ran nothing");
    assert!(
        busy_stats.instructions > idle_stats.instructions * 10,
        "busy {} instructions against idle {} — the two VMs are counting the same tasks",
        busy_stats.instructions,
        idle_stats.instructions
    );
    assert!(!busy_stats.finished, "the busy script is still running");
    assert!(!idle_stats.finished, "the idle script is still running");

    // and each says the file of its own script, which is the sharpest form of "these numbers
    // came from the right VM"
    for (stats, file) in [(&busy_stats, "busy.rb"), (&idle_stats, "idle.rb")] {
        let (name, _) = stats.location.clone().expect("the task says where it stands");
        assert!(name.ends_with(file), "a task of {file} says it stands in {name}");
    }
}

/// One entity, two scripts, two VMs — which the plan allows on purpose (§3, default 6). They are
/// different components, so nothing stops it; what had to be made true is that one of them
/// ending does not mark the other done, which is why `ScriptDone` carries the name tag too.
#[test]
fn one_entity_can_carry_a_script_of_each_vm() {
    const SHORT: &str = r#"
      Rubevy.ask("once", 1.0)
      :done
    "#;

    let mut app = app();
    let asset_a = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        assets.add(compile(SHORT, "short.rb"))
    };
    let asset_b = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        assets.add(compile(HEARTBEAT, "heartbeat.rb"))
    };
    let entity = app
        .world_mut()
        .spawn((Script::<A>::for_vm(asset_a), Script::<B>::for_vm(asset_b)))
        .id();

    frames(&mut app, 10);
    {
        let e = app.world().entity(entity);
        assert!(e.get::<ScriptTask<A>>().is_some(), "the first VM made a task of its script");
        assert!(e.get::<ScriptTask<B>>().is_some(), "the second VM made a task of its script");
    }
    assert_eq!(app.world().resource::<Marks>().a, vec![1.0], "the short script ran once");

    // the short one has ended by now; the long one is on the same entity and must not have been
    // marked done with it
    let before = app.world().resource::<Marks>().b.len();
    frames(&mut app, 30);
    let marks = app.world().resource::<Marks>();
    assert_eq!(marks.a, vec![1.0], "the short script ran again");
    assert!(
        marks.b.len() > before,
        "the other VM's script on the same entity stopped at {before} beats when its neighbour \
         ended"
    );
}

/// The one thing the two VMs share is the asset type: `MrbAsset` and its loader belong to no VM,
/// and `init_asset` builds a *fresh* `Assets<MrbAsset>` and inserts it, so a second plugin that
/// called it again would throw away every handle already in there. That is not hypothetical —
/// reaching into the app between two `add_plugins` is how `tests/vm_setup.rs` sets a VM up, and
/// with the guard taken out of `RubevyPlugin::build` this test fails with the handle gone.
#[test]
fn the_second_plugin_does_not_throw_away_the_first_vms_assets() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::<A>::for_vm("assets"),
    ));
    let handle = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile(HEARTBEAT, "heartbeat.rb"));

    app.add_plugins(RubevyPlugin::<B>::for_vm("assets"));

    assert!(
        app.world().resource::<Assets<MrbAsset>>().get(&handle).is_some(),
        "the second plugin registered the shared asset again and took the first VM's with it"
    );

    // and the loader is still the one loader for `.mrb`: the asset the first VM's handle names
    // still runs, in the VM that was added second as well
    let entity = app.world_mut().spawn(Script::<B>::for_vm(handle)).id();
    app.init_resource::<Marks>()
        .add_systems(Update, answer_b.in_set(RubevySet::<B>::answer()));
    frames(&mut app, 10);
    assert!(
        app.world().entity(entity).get::<ScriptTask<B>>().is_some(),
        "the asset that survived did not start"
    );
    assert!(!app.world().resource::<Marks>().b.is_empty(), "and it did not run");
}
