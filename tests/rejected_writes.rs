//! What a script is told about a write the world would not take.
//!
//! A write lands at the end of the frame, so the line that made it is long gone by the time
//! anything is known about it: `e[:X] = hash` answers the hash it was given whether the write
//! landed or not, and until now the only word about a refusal was a `warn!` in the host's log.
//! `Rubevy.rejected_writes` is that word, a frame later and on the script's own side; the host's
//! side is `ScriptWorld::rejected_writes`. The record is
//! `docs/worklog/2026-09-20-rejected-writes.md`.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, Request, RubevyPlugin, Script, ScriptTask, ScriptWorld};

#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
struct Hp {
    current: f32,
    max: f32,
}

#[derive(Resource, Reflect, Debug, Default)]
#[reflect(Resource)]
struct Score {
    points: f32,
}

/// What the scripts asked the game, so a test can read what they saw.
#[derive(Resource, Default)]
struct Seen(Vec<Request>);

/// The last refusals the host saw. `ScriptWorld::rejected_writes` holds what one frame's writes
/// refused and holds it until the scripts write again, so a system that ran every frame would
/// see the same list over and over; what a test wants is the one it stood at.
#[derive(Resource, Default)]
struct Refused(Vec<String>);

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn answer_nil(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>) {
    for r in world.take_requests() {
        world.answer(&r, Answer::Nil);
        seen.0.push(r);
    }
}

/// After `RubevySet::Tick`, which is where the writes are made and the list is filled.
fn collect_refusals(world: Res<ScriptWorld>, mut refused: ResMut<Refused>) {
    if world.rejected_writes().is_empty() {
        return;
    }
    refused.0 = world
        .rejected_writes()
        .iter()
        .map(|w| format!("{:?}|{:?}|{}|{}", w.by, w.on, w.name, w.reason))
        .collect();
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Seen>()
    .init_resource::<Refused>()
    .register_type::<Transform>()
    .register_type::<Hp>()
    .register_type::<Score>()
    .insert_resource(Score { points: 1.0 })
    .add_systems(Update, answer_nil)
    .add_systems(Update, collect_refusals.after(rubevy::RubevySet::Tick));
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(5));
        app.update();
    }
}

fn run(app: &mut App, entity: Entity, src: &str) {
    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
    app.world_mut().entity_mut(entity).insert(Script::new(script));
}

/// The plain case, and the one the camera layer met: a component the entity does not have. The
/// script writes, waits for the clock to move, and reads what became of it — the type it named,
/// the entity it wrote to, and the sentence the log carries.
#[test]
fn a_script_reads_what_became_of_its_own_write() {
    let mut app = app();
    let who = app.world_mut().spawn(Transform::default()).id();
    run(
        &mut app,
        who,
        r#"
          e = Rubevy.entity
          e[:Hp] = { current: 3.0 }          # nothing on this entity has an Hp
          sleep 0
          bad = Rubevy.rejected_writes
          Rubevy.ask("read",
                     bad.length,
                     bad[0][:name],
                     bad[0][:entity].class.to_s,
                     (bad[0][:entity] == e).to_s,
                     bad[0][:reason]).pop
        "#,
    );
    frames(&mut app, 20);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.num(0), Some(1.0), "one refusal, not two and not none");
    assert_eq!(r.text(1), Some("Hp"), "the type as the script spelled it");
    assert_eq!(r.text(2), Some("Rubevy::Entity"), "what was written to, as an object");
    assert_eq!(r.text(3), Some("true"), "and it is this entity");
    assert_eq!(r.text(4), Some(&format!("{who} has no Hp")[..]), "the words the log carries");
}

/// A write that lands leaves nothing behind. The same script writes something the entity really
/// has, and the list it reads afterwards is empty — which is what makes the list worth reading.
#[test]
fn a_write_that_landed_says_nothing() {
    let mut app = app();
    let who = app.world_mut().spawn((Transform::default(), Hp { current: 1.0, max: 5.0 })).id();
    run(
        &mut app,
        who,
        r#"
          Rubevy.entity[:Hp] = { current: 3.0 }
          sleep 0
          Rubevy.ask("read", Rubevy.rejected_writes.length, Rubevy.entity[:Hp][:current]).pop
        "#,
    );
    frames(&mut app, 20);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.num(0), Some(0.0), "nothing was refused");
    assert_eq!(r.num(1), Some(3.0), "because the write landed");
    assert!(app.world().resource::<Refused>().0.is_empty(), "the host saw none either");
}

/// A resource write has no entity to name, so `entity` is nil — which is how a script tells the
/// two apart without a key of its own for it.
#[test]
fn a_refused_resource_write_names_no_entity() {
    let mut app = app();
    let who = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        who,
        r#"
          Rubevy.set_resource(:Nowhere, { points: 2.0 })   # no type of that name
          Rubevy.set_resource(:Score, { points: 9.0 })     # this one lands
          sleep 0
          bad = Rubevy.rejected_writes
          Rubevy.ask("read",
                     bad.length,
                     bad[0][:name],
                     bad[0][:entity].inspect,
                     bad[0][:reason],
                     Rubevy.resource(:Score)[:points]).pop
        "#,
    );
    frames(&mut app, 20);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.num(0), Some(1.0), "one of the two writes was refused");
    assert_eq!(r.text(1), Some("Nowhere"));
    assert_eq!(r.text(2), Some("nil"), "a resource belongs to no entity");
    assert_eq!(r.text(3), Some("no registered resource named Nowhere"));
    assert_eq!(r.num(4), Some(9.0), "and the write beside it still landed");
}

/// A script is answered its own writes and nobody else's. Two scripts refuse a write in the same
/// frame, each naming a different type, and each sees one — the one it made.
#[test]
fn a_script_does_not_see_another_scripts_refusals() {
    let mut app = app();
    let one = app.world_mut().spawn_empty().id();
    let two = app.world_mut().spawn_empty().id();
    let src = r#"
          Rubevy.entity[:NAME] = { current: 1.0 }
          sleep 0
          bad = Rubevy.rejected_writes
          Rubevy.ask("read", bad.length, bad.map { |w| w[:name] }.join(",")).pop
        "#;
    run(&mut app, one, &src.replace("NAME", "Alpha"));
    run(&mut app, two, &src.replace("NAME", "Beta"));
    frames(&mut app, 20);

    let seen = app.world().resource::<Seen>();
    assert_eq!(seen.0.len(), 2, "both scripts got there");
    let mut names: Vec<&str> = seen.0.iter().map(|r| r.text(1).unwrap_or("?")).collect();
    names.sort();
    assert_eq!(names, vec!["Alpha", "Beta"], "each saw its own name");
    for r in &seen.0 {
        assert_eq!(r.num(0), Some(1.0), "one each, not two");
    }

    // and the host saw both, with the entity of the script that wrote each
    let refused = &app.world().resource::<Refused>().0;
    assert_eq!(refused.len(), 2, "the host's side is the wider one: {refused:?}");
    assert!(refused.iter().any(|w| w.contains(&format!("Some({one})")) && w.contains("|Alpha|")));
    assert!(refused.iter().any(|w| w.contains(&format!("Some({two})")) && w.contains("|Beta|")));
}

/// `by` is the script that wrote, `entity` is what was written to, and they are not the same
/// thing: a script may write another entity's component, and it is the writer that is answered.
#[test]
fn a_write_to_another_entity_is_still_this_scripts_write() {
    let mut app = app();
    let who = app.world_mut().spawn_empty().id();
    let other = app.world_mut().spawn(Transform::default()).id();
    run(
        &mut app,
        who,
        &format!(
            r#"
          Rubevy.find(:Transform)[0][:Hp] = {{ current: 1.0 }}
          sleep 0
          bad = Rubevy.rejected_writes
          Rubevy.ask("read", bad.length, bad[0][:entity].to_i, {}).pop
        "#,
            other.to_bits()
        ),
    );
    frames(&mut app, 20);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.num(0), Some(1.0), "the writer is answered for a write it made elsewhere");
    assert_eq!(r.num(1), r.num(2), "and what it names is the entity written to");
    let refused = &app.world().resource::<Refused>().0;
    assert_eq!(refused.len(), 1);
    assert!(
        refused[0].starts_with(&format!("Some({who})|Some({other})|Hp|")),
        "the host is told both ends: {}",
        refused[0]
    );
}

/// The list is of one frame's writes. The script refuses a write, looks (and sees it), then
/// writes something that lands — and what it reads after that is empty, because the frame that
/// wrote replaced the lot.
#[test]
fn the_next_frame_that_writes_replaces_the_list() {
    let mut app = app();
    let who = app.world_mut().spawn((Transform::default(), Hp::default())).id();
    run(
        &mut app,
        who,
        r#"
          e = Rubevy.entity
          e[:Nothing] = { current: 1.0 }
          sleep 0
          first = Rubevy.rejected_writes.length
          e[:Hp] = { current: 2.0 }           # a frame that writes, and this one lands
          sleep 0
          Rubevy.ask("read", first, Rubevy.rejected_writes.length).pop
        "#,
    );
    frames(&mut app, 30);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the script asked");
    assert_eq!(r.num(0), Some(1.0), "the refusal was there to be read");
    assert_eq!(r.num(1), Some(0.0), "and the next frame that wrote took it away");
}

/// A task with no entity of its own made no write that anything is `by`, so it is answered an
/// empty list rather than everybody's.
#[test]
fn a_task_without_an_entity_is_answered_nothing() {
    let mut app = app();
    let who = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        who,
        r#"
          Rubevy.entity[:Nothing] = { current: 1.0 }
          sleep 0
          t = Task.new(name: "stranger") do
            Task.current.instance_variable_set(:@rubevy_entity, nil)
            Rubevy.ask("read", Rubevy.rejected_writes.length).pop
          end
          sleep 0.05
        "#,
    );
    frames(&mut app, 30);

    let seen = app.world().resource::<Seen>();
    let r = seen.0.first().expect("the task asked");
    assert_eq!(r.num(0), Some(0.0), "not everybody's, and not an error");
}

/// What one refused write costs the frame's budget, which is the number behind
/// `ScriptWorld::rejected_writes` having no limit of its own: the budget already bounds how many
/// a frame can make. `#[ignore]`d because it is an instrument — the count is of the VM, not of
/// the machine, so it is the same everywhere until SabiRuby's compiler changes.
///
///     cargo test --test rejected_writes -- --ignored --nocapture
#[test]
#[ignore]
fn what_one_rejected_write_costs() {
    let counts = [100usize, 1100];
    let mut spent = Vec::new();
    for n in counts {
        let mut app = app();
        let who = app.world_mut().spawn_empty().id();
        run(
            &mut app,
            who,
            &format!(
                r#"
                  {n}.times {{ Rubevy.entity[:Nothing] = {{ current: 1.0 }} }}
                  Rubevy.ask("done").pop
                "#
            ),
        );
        frames(&mut app, 20);
        let task = *app.world().entity(who).get::<ScriptTask>().expect("a task");
        let world = app.world().resource::<ScriptWorld>();
        let stats = world.stats(&task);
        spent.push((n, stats.instructions, world.rejected_writes().len()));
    }
    let (n0, i0, _) = spent[0];
    let (n1, i1, kept) = spent[1];
    let each = (i1 - i0) as f64 / (n1 - n0) as f64;
    println!("rejected writes: {spent:?}");
    println!("one write costs {each:.1} instructions; the last frame kept {kept}");
    println!("a budget of 200,000 bounds a frame at {:.0} of them", 200_000.0 / each);
}
