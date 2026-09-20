//! Events: `Rubevy.subscribe(:hit)` answers a queue the game pushes onto, so waiting for
//! something to happen is the same thing as waiting for an answer — in the script's own task or
//! in one it made. Only what a script subscribed to reaches it, a message addressed to an
//! entity reaches that entity's scripts alone, and the subscription goes when the script does.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{entity_object, Answer, MrbAsset, Request, RubevyPlugin, Script, ScriptWorld};
use sabiruby::Value;

#[derive(Resource, Default)]
struct Seen(Vec<Request>);

/// An event of the game's own, to show the observer line a game writes.
#[derive(EntityEvent)]
struct Hit {
    entity: Entity,
    damage: f32,
}

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Seen>()
    .add_systems(Update, answer_nil);
    app
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        app.update();
    }
}

fn answer_nil(mut world: ResMut<ScriptWorld>, mut seen: ResMut<Seen>) {
    for r in world.take_requests() {
        world.answer(&r, Answer::Nil);
        seen.0.push(r);
    }
}

fn run(app: &mut App, src: &str) -> Entity {
    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
    app.world_mut().spawn(Script::new(script)).id()
}

/// The same, compiled with the line table (`mrbc -g`), which is what an `Exception#backtrace`
/// is made of: `Options::default()` leaves the DBG section out, and a program with no line
/// table has nothing to say about where it raised.
fn run_with_lines(app: &mut App, src: &str) -> Entity {
    let opts = sabiruby_compiler::Options {
        filename: "t.rb".into(),
        debug_info: true,
        ..Default::default()
    };
    let bytes = sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles");
    let script = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(MrbAsset { bytes });
    app.world_mut().spawn(Script::new(script)).id()
}

/// The frames it takes for a script to reach its first `Rubevy.ask("ready")` and be answered.
fn until_ready(app: &mut App) {
    for _ in 0..20 {
        frames(app, 1);
        if !app.world().resource::<Seen>().0.is_empty() {
            return;
        }
    }
}

fn asked(app: &App, kind: &str) -> Option<Request> {
    app.world().resource::<Seen>().0.iter().find(|r| r.kind == kind).cloned()
}

#[test]
fn only_what_a_script_subscribed_to_reaches_it() {
    let mut app = app();
    run(
        &mut app,
        r#"
          hits = Rubevy.subscribe(:hit)
          Rubevy.ask("ready").pop
          damage = hits.pop
          Rubevy.ask("got", damage, hits.size).pop
        "#,
    );
    until_ready(&mut app);

    {
        let mut world = app.world_mut().resource_mut::<ScriptWorld>();
        world.publish(None, "hit", Answer::Num(7.0));
        world.publish(None, "miss", Answer::Num(1.0));
    }
    frames(&mut app, 25);

    let got = asked(&app, "got").expect("the script woke on its queue");
    assert_eq!(got.num(0), Some(7.0));
    assert_eq!(got.num(1), Some(0.0), "the miss went nowhere: nobody subscribed to it");
}

#[test]
fn a_message_to_an_entity_reaches_that_entity_alone() {
    let mut app = app();
    let src = r#"
      hits = Rubevy.subscribe(:hit)
      Rubevy.ask("ready").pop
      sleep 0.01
      Rubevy.ask("count", Rubevy.entity.to_i, hits.size).pop
    "#;
    let a = run(&mut app, src);
    let _b = run(&mut app, src);
    // both scripts have to be parked on "ready" before the message goes out
    for _ in 0..20 {
        frames(&mut app, 1);
        if app.world().resource::<Seen>().0.len() >= 2 {
            break;
        }
    }
    assert_eq!(app.world().resource::<Seen>().0.len(), 2, "both scripts asked");

    app.world_mut().resource_mut::<ScriptWorld>().publish(Some(a), "hit", Answer::Num(1.0));
    frames(&mut app, 25);

    let counts: Vec<(f64, f64)> = app
        .world()
        .resource::<Seen>()
        .0
        .iter()
        .filter(|r| r.kind == "count")
        .map(|r| (r.num_or(0, -1.0), r.num_or(1, -1.0)))
        .collect();
    assert_eq!(counts.len(), 2, "both scripts reported");
    for (who, n) in counts {
        let want = if who == a.to_bits() as f64 { 1.0 } else { 0.0 };
        assert_eq!(n, want, "entity {who} had {n} messages");
    }
}

#[test]
fn another_task_can_wait_on_the_queue() {
    let mut app = app();
    run(
        &mut app,
        r#"
          hits = Rubevy.subscribe(:hit)
          $seen = []
          Task.new(name: "reflex") { loop { $seen << hits.pop } }
          Rubevy.ask("ready").pop
          sleep 0.01
          Rubevy.ask("reflex", $seen.length, $seen[0].to_f, $seen[1].to_f).pop
        "#,
    );
    until_ready(&mut app);

    {
        let mut world = app.world_mut().resource_mut::<ScriptWorld>();
        world.publish(None, "hit", Answer::Num(3.0));
        world.publish(None, "hit", Answer::Num(4.0));
    }
    frames(&mut app, 25);

    let r = asked(&app, "reflex").expect("the reflex task read the queue");
    assert_eq!(r.num(0), Some(2.0));
    assert_eq!((r.num(1), r.num(2)), (Some(3.0), Some(4.0)), "in the order they were published");
}

#[test]
fn a_payload_may_carry_an_entity() {
    let mut app = app();
    let by = app.world_mut().spawn_empty().id();
    run(
        &mut app,
        r#"
          hits = Rubevy.subscribe(:hit)
          Rubevy.ask("ready").pop
          who, damage = hits.pop
          Rubevy.ask("by", who, damage, who.class.to_s).pop
        "#,
    );
    until_ready(&mut app);

    {
        let mut world = app.world_mut().resource_mut::<ScriptWorld>();
        let class = world.entity_class();
        world.publish_value(None, "hit", move |vm| {
            let who = entity_object(vm, class, by);
            vm.ary_new(vec![who, Value::Float(2.5)])
        });
    }
    frames(&mut app, 25);

    let r = asked(&app, "by").expect("the script woke");
    assert_eq!(r.entity_arg(0), Some(by), "an entity crosses as an entity, not as a number");
    assert_eq!(r.num(1), Some(2.5));
    assert_eq!(r.text(2), Some("Rubevy::Entity"));
}

#[test]
fn an_observer_is_the_one_line_a_game_writes() {
    let mut app = app();
    app.add_observer(|on: On<Hit>, mut scripts: ResMut<ScriptWorld>| {
        scripts.publish(Some(on.entity), "hit", Answer::Num(on.damage as f64));
    });
    let script = run(
        &mut app,
        r#"
          hits = Rubevy.subscribe(:hit)
          Rubevy.ask("ready").pop
          Rubevy.ask("hurt", hits.pop).pop
        "#,
    );
    until_ready(&mut app);

    app.world_mut().trigger(Hit { entity: script, damage: 12.0 });
    frames(&mut app, 25);

    assert_eq!(asked(&app, "hurt").and_then(|r| r.num(0)), Some(12.0));
}

#[test]
fn the_oldest_messages_are_dropped_when_nobody_reads() {
    let mut app = app();
    run(
        &mut app,
        r#"
          q = Rubevy.subscribe(:tick)
          Rubevy.ask("ready").pop
          sleep 0.01                       # the host fills the queue while this waits
          Rubevy.ask("kept", q.size, q.pop).pop
        "#,
    );
    until_ready(&mut app);

    {
        let mut world = app.world_mut().resource_mut::<ScriptWorld>();
        for i in 0..100 {
            world.publish(None, "tick", Answer::Num(i as f64));
        }
    }
    frames(&mut app, 25);

    let r = asked(&app, "kept").expect("the script woke");
    assert_eq!(r.num(0), Some(ScriptWorld::QUEUE_LIMIT as f64), "the queue is capped");
    assert_eq!(r.num(1), Some(36.0), "and it is the newest 64 that are kept, not the oldest");
}

/// What overflowed used to happen in silence — no log, no counter, nothing either side could
/// see. Now the VM counts what it dropped (`ScriptWorld::dropped`, for a game's HUD) and each
/// queue counts its own (`Rubevy::Subscription#dropped`, for the script whose stream has the
/// hole in it), and the two agree where there is one subscription.
#[test]
fn what_overflowed_is_counted_for_the_vm_and_for_the_subscription() {
    let mut app = app();
    run(
        &mut app,
        r#"
          q = Rubevy.subscribe(:tick)
          Rubevy.ask("ready").pop
          sleep 0.01                       # the host fills the queue while this waits
          Rubevy.ask("lost", q.size, q.dropped, q.pop).pop
        "#,
    );
    until_ready(&mut app);
    assert_eq!(app.world().resource::<ScriptWorld>().dropped(), 0, "nothing has been published");

    {
        let mut world = app.world_mut().resource_mut::<ScriptWorld>();
        for i in 0..100 {
            world.publish(None, "tick", Answer::Num(i as f64));
        }
    }
    let over = 100 - ScriptWorld::QUEUE_LIMIT;
    assert_eq!(
        app.world().resource::<ScriptWorld>().dropped(),
        over as u64,
        "the host's own count, which it may read without asking the script anything"
    );

    frames(&mut app, 25);
    let r = asked(&app, "lost").expect("the script woke");
    assert_eq!(r.num(0), Some(ScriptWorld::QUEUE_LIMIT as f64), "the queue is capped");
    assert_eq!(r.num(1), Some(over as f64), "and the script can see how many it missed");
    assert_eq!(r.num(2), Some(over as f64), "the oldest kept is the first one not dropped");
}

/// The limit is a field of the VM, not a constant: a game that publishes more than sixty-four of
/// something a frame can say so, and what a script is handed in one frame moves with it. The
/// default is still what the constant says, so nothing moves for an app that sets nothing.
#[test]
fn the_vms_queue_limit_moves_what_a_script_is_handed() {
    let mut app = app();
    assert_eq!(
        app.world().resource::<ScriptWorld>().queue_limit,
        ScriptWorld::QUEUE_LIMIT,
        "the field starts at the constant"
    );
    app.world_mut().resource_mut::<ScriptWorld>().queue_limit = 250;
    run(
        &mut app,
        r#"
          q = Rubevy.subscribe(:tick)
          Rubevy.ask("ready").pop
          sleep 0.01
          Rubevy.ask("lost", q.size, q.dropped, q.pop).pop
        "#,
    );
    until_ready(&mut app);

    {
        let mut world = app.world_mut().resource_mut::<ScriptWorld>();
        for i in 0..300 {
            world.publish(None, "tick", Answer::Num(i as f64));
        }
    }
    frames(&mut app, 25);

    let r = asked(&app, "lost").expect("the script woke");
    assert_eq!(r.num(0), Some(250.0), "the queue holds what the field says, not sixty-four");
    assert_eq!(r.num(1), Some(50.0), "so only the first fifty of three hundred were dropped");
    assert_eq!(r.num(2), Some(50.0));
    assert_eq!(app.world().resource::<ScriptWorld>().dropped(), 50);
}

/// A stream a script knows is heavy asks for its own queue, and one it only samples may ask for
/// a smaller one than the VM's. Two subscriptions of one script, filed under one name, each keep
/// their own count of what they lost.
#[test]
fn a_subscription_may_ask_for_a_limit_of_its_own() {
    let mut app = app();
    run(
        &mut app,
        r#"
          sampled = Rubevy.subscribe(:tick, limit: 3)
          whole   = Rubevy.subscribe(:tick)
          Rubevy.ask("ready").pop
          sleep 0.01
          Rubevy.ask("both", sampled.size, sampled.dropped, whole.size, whole.dropped).pop
        "#,
    );
    until_ready(&mut app);

    {
        let mut world = app.world_mut().resource_mut::<ScriptWorld>();
        for i in 0..10 {
            world.publish(None, "tick", Answer::Num(i as f64));
        }
    }
    frames(&mut app, 25);

    let r = asked(&app, "both").expect("the script woke");
    assert_eq!(r.num(0), Some(3.0), "the one that asked for three holds three");
    assert_eq!(r.num(1), Some(7.0), "and lost the other seven");
    assert_eq!(r.num(2), Some(10.0), "the one that asked for nothing is under the VM's 64");
    assert_eq!(r.num(3), Some(0.0), "so it lost nothing");
    assert_eq!(
        app.world().resource::<ScriptWorld>().dropped(),
        7,
        "the VM's count is the messages dropped, over every subscription"
    );
}

/// A limit that is not a count of messages is refused where the script wrote it, with the line
/// it wrote it on — not taken as something else and not ignored. So is a keyword that is not
/// `limit:`, because a subscription that quietly keeps the default is the kind of mistake that
/// is found frames later.
#[test]
fn a_limit_that_is_not_a_count_is_an_argument_error() {
    let mut app = app();
    run_with_lines(
        &mut app,
        r##"
          def refused
            yield
            "nothing was raised"
          rescue ArgumentError => e
            "#{e.message} @ #{e.backtrace.first}"
          end
          zero    = refused { Rubevy.subscribe(:tick, limit: 0) }
          decimal = refused { Rubevy.subscribe(:tick, limit: 2.5) }
          typo    = refused { Rubevy.subscribe(:tick, limt: 4) }
          plain   = refused { Rubevy.subscribe(:tick, 4) }
          Rubevy.ask("refused", zero, decimal, typo, plain).pop
        "##,
    );
    frames(&mut app, 25);

    let r = asked(&app, "refused").expect("the script reached the question");
    let zero = r.text(0).unwrap_or("");
    assert!(zero.starts_with("subscribe: limit: takes a whole number"), "zero: {zero}");
    assert!(zero.contains("not 0"), "the message shows what was asked for: {zero}");
    assert!(zero.contains("t.rb:"), "raised in the script's own frame, so it has its line: {zero}");
    let decimal = r.text(1).unwrap_or("");
    assert!(decimal.contains("not 2.5"), "a Float is not a count of messages: {decimal}");
    let typo = r.text(2).unwrap_or("");
    assert!(typo.starts_with("subscribe: unknown keyword: limt"), "typo: {typo}");
    let plain = r.text(3).unwrap_or("");
    assert!(plain.starts_with("subscribe takes the name"), "a bare number is not a limit: {plain}");
    assert_eq!(
        app.world().resource::<ScriptWorld>().subscriptions(),
        0,
        "nothing that was refused left a subscription behind"
    );
}

/// Subscriptions are filed by name (`HostState::subscriptions`), so this is the shape of that
/// filing seen from Ruby: one script may listen for several names, each name reaches only what
/// subscribed to it, and within a name a message is built and handed out in the order the
/// subscriptions were asked for — which is what a host's `publish_value` closure sees, since it
/// runs once per subscriber.
#[test]
fn within_a_name_the_message_goes_out_in_the_order_it_was_subscribed_to() {
    let mut app = app();
    run(
        &mut app,
        r#"
          first  = Rubevy.subscribe(:hit)
          second = Rubevy.subscribe(:hit)
          other  = Rubevy.subscribe(:heal)
          Rubevy.ask("ready").pop
          sleep 0.01                       # the host publishes once while this waits
          Rubevy.ask("got", first.pop, second.pop, other.size).pop
        "#,
    );
    until_ready(&mut app);
    assert_eq!(
        app.world().resource::<ScriptWorld>().subscriptions(),
        3,
        "three subscriptions of one script, two of them for the same name"
    );

    {
        let mut world = app.world_mut().resource_mut::<ScriptWorld>();
        let mut nth = 0.0;
        world.publish_value(None, "hit", move |_vm| {
            nth += 1.0;
            Value::Float(nth)
        });
    }
    frames(&mut app, 25);

    let r = asked(&app, "got").expect("the script woke on its queues");
    assert_eq!(
        (r.num(0), r.num(1)),
        (Some(1.0), Some(2.0)),
        "the subscription asked for first was built for first"
    );
    assert_eq!(r.num(2), Some(0.0), "and the other name heard nothing");
}

#[test]
fn the_subscription_goes_when_the_script_does() {
    let mut app = app();
    run(
        &mut app,
        r#"
          hits = Rubevy.subscribe(:hit)
          Rubevy.ask("ready").pop
          hits.pop                         # and the script ends on the first message
        "#,
    );
    until_ready(&mut app);
    frames(&mut app, 2);
    assert_eq!(
        app.world().resource::<ScriptWorld>().subscriptions(),
        1,
        "the script is parked on its queue"
    );

    app.world_mut().resource_mut::<ScriptWorld>().publish(None, "hit", Answer::Num(1.0));
    frames(&mut app, 5);
    assert_eq!(
        app.world().resource::<ScriptWorld>().subscriptions(),
        0,
        "the script ran off its end, so a queue nobody will read is not kept"
    );

    // and publishing to a name nobody listens for is not an error
    app.world_mut().resource_mut::<ScriptWorld>().publish(None, "hit", Answer::Num(1.0));
}

#[test]
fn a_despawned_script_stops_listening() {
    let mut app = app();
    let entity = run(
        &mut app,
        r#"
          hits = Rubevy.subscribe(:hit)
          Rubevy.ask("ready").pop
          loop { hits.pop }
        "#,
    );
    until_ready(&mut app);
    frames(&mut app, 3);
    assert_eq!(app.world().resource::<ScriptWorld>().subscriptions(), 1);

    app.world_mut().entity_mut(entity).despawn();
    assert_eq!(app.world().resource::<ScriptWorld>().subscriptions(), 0);
}

/// A task waiting on a subscription is ended by the unsubscription, rather than left standing
/// on a queue nothing can fill: the queue is closed, `Rubevy::Subscription#pop` raises
/// `Rubevy::Unsubscribed` in whatever was parked on it, and the task unwinds through its
/// `ensure`. Without that the task stays `WAITING` for as long as the VM lives — one leaked
/// context per script that had a second task.
#[test]
fn a_task_waiting_on_a_subscription_ends_when_the_script_does() {
    let mut app = app();
    let entity = run(
        &mut app,
        r#"
          hits = Rubevy.subscribe(:hit)
          Task.new(name: "reflex") do
            begin
              loop { hits.pop }
            ensure
              Rubevy.ask("reflex_ensure")   # not popped: nothing may park while unwinding
            end
          end
          Rubevy.ask("ready").pop
          loop { sleep 0.05 }
        "#,
    );
    until_ready(&mut app);
    frames(&mut app, 3);
    assert_eq!(app.world().resource::<ScriptWorld>().subscriptions(), 1);
    assert!(asked(&app, "reflex_ensure").is_none(), "the reflex task is waiting, not ending");

    app.world_mut().entity_mut(entity).despawn();
    frames(&mut app, 10);

    assert_eq!(app.world().resource::<ScriptWorld>().subscriptions(), 0);
    assert!(
        asked(&app, "reflex_ensure").is_some(),
        "the waiting task woke on the closed queue and ran its ensure"
    );
}

/// What a script sees of the end: the exception, which it may rescue to finish on its own terms.
#[test]
fn a_script_can_rescue_the_end_of_its_subscription() {
    let mut app = app();
    let entity = run(
        &mut app,
        r#"
          hits = Rubevy.subscribe(:hit)
          $seen = []
          Task.new(name: "reflex") do
            begin
              loop { $seen << hits.pop }
            rescue Rubevy::Unsubscribed
              Rubevy.ask("rescued", $seen.length, $seen[0].to_f)
            end
          end
          Rubevy.ask("ready").pop
          loop { sleep 0.05 }
        "#,
    );
    until_ready(&mut app);
    // one message is in the queue when it closes: the backlog is popped before the end is
    app.world_mut().resource_mut::<ScriptWorld>().publish(Some(entity), "hit", Answer::Num(5.0));
    frames(&mut app, 3);

    app.world_mut().entity_mut(entity).despawn();
    frames(&mut app, 10);

    let r = asked(&app, "rescued").expect("the task rescued the end of its subscription");
    assert_eq!((r.num(0), r.num(1)), (Some(1.0), Some(5.0)), "it read what was queued first");
}
