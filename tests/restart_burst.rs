//! Restarting ten scripts in the same frame must not stop the VM.
//!
//! rubevy_games' garden found this (`docs/worklog/2026-09-17-restart-burst.md`): replacing ten
//! creatures at once — each one a brain task plus six reflex tasks parked on a subscription's
//! `pop` — froze every script in the VM for seconds, the ones that had not been touched
//! included. Nine at once was fine and spreading the restarts 0.4 s apart was fine, which is
//! what made it look like a matter of how many contexts end at the same moment.
//!
//! It was not. Removing a `ScriptTask` closes the subscriptions of that entity, each parked
//! reflex task wakes with `Rubevy::Unsubscribed`, its `rescue` clause makes the block's value
//! nil, and a task that ends with a nil result answers the same nil that an empty scheduler
//! does. `Vm::task_run_limits` used to read that as "nothing is runnable" and give the rest of
//! the frame back, so sixty tasks ending together cost sixty frames in which nothing else in
//! the VM ran at all.
//!
//! This test needs the fix in the VM (sabiruby `task_run_limited`, 2026-09-17). Against a VM
//! without it the eleventh script — which nothing touched — stands still for frame after frame.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, RubevyPlugin, Script, ScriptDone, ScriptTask, ScriptWorld};

/// Creatures replaced in one frame. Nine was fine, ten was not.
const CREATURES: usize = 10;
/// Reflex tasks each creature parks on a subscription of its own.
const REFLEXES: usize = 6;

/// A creature: six subscriptions, six tasks parked on them, and a brain that asks the game
/// something every turn. The `rescue` with no body is what makes a reflex task's value nil when
/// its queue closes — the shape the garden's creatures are written in.
const CREATURE: &str = r#"
  qs = []
  6.times { |i| qs << Rubevy.subscribe("e#{i}") }
  qs.each do |q|
    Task.new do
      begin
        loop { q.pop }
      rescue
      end
    end
  end
  loop { Rubevy.ask("brain").pop }
"#;

/// The eleventh script, which nothing in this test touches.
const WATCHER: &str = r#"loop { Rubevy.ask("watch").pop }"#;

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn answer_all(mut world: ResMut<ScriptWorld>) {
    for r in world.take_requests() {
        world.answer(&r, Answer::Nil);
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .add_systems(Update, answer_all);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

/// What the entity's script has spent so far, or `None` where it has no task yet.
fn spent(app: &App, entity: Entity) -> Option<u64> {
    let task = *app.world().get::<ScriptTask>(entity)?;
    Some(app.world().resource::<ScriptWorld>().stats(&task).instructions)
}

#[test]
fn ten_scripts_restarted_in_one_frame_do_not_stop_the_vm() {
    let mut app = app();
    let (creature, watcher) = {
        let mut assets = app.world_mut().resource_mut::<Assets<MrbAsset>>();
        (assets.add(compile(CREATURE)), assets.add(compile(WATCHER)))
    };
    let creatures: Vec<Entity> = (0..CREATURES)
        .map(|_| app.world_mut().spawn(Script::new(creature.clone())).id())
        .collect();
    let watching = app.world_mut().spawn(Script::new(watcher)).id();

    // everything running: every creature has its six reflex tasks parked, and the watcher is
    // going round its own loop
    frames(&mut app, 25);
    assert_eq!(
        app.world().resource::<ScriptWorld>().subscriptions(),
        CREATURES * REFLEXES,
        "every creature is listening"
    );
    let watched_before = spent(&app, watching).expect("the watcher has a task");
    assert!(watched_before > 0, "the watcher ran before the burst");
    for (i, e) in creatures.iter().enumerate() {
        assert!(spent(&app, *e).unwrap_or(0) > 0, "creature {i} ran before the burst");
    }

    // the burst: all ten replaced in the same frame, which is what the garden's "Apply" does
    for e in &creatures {
        app.world_mut()
            .entity_mut(*e)
            .remove::<ScriptTask>()
            .remove::<ScriptDone>()
            .insert(Script::new(creature.clone()));
    }

    // three frames, and every one of them must move the VM on
    let mut watched = watched_before;
    for frame in 1..=3 {
        frames(&mut app, 1);
        let now = spent(&app, watching).expect("the watcher still has a task");
        assert!(
            now > watched,
            "frame {frame} after the burst ran nothing for the script nobody touched: \
             {watched} instructions before the frame, {now} after"
        );
        watched = now;
    }

    // and the new brains are running, not queued behind sixty tasks ending one frame each
    for (i, e) in creatures.iter().enumerate() {
        let now = spent(&app, *e).expect("creature {i} was restarted");
        assert!(now > 0, "the new script of creature {i} has not run three frames after the burst");
    }
    assert_eq!(
        app.world().resource::<ScriptWorld>().subscriptions(),
        CREATURES * REFLEXES,
        "the new scripts are listening, and the old subscriptions went with the old tasks"
    );
}
