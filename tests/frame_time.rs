//! `ScriptWorld::frame_time` as a limit on the tick.
//!
//! The tick is a loop — run the ready tasks, answer what they parked on, run them again — and
//! until 2026-09-20 the clock was looked at only at the head of a round. A round that parked a
//! thousand tasks then made a thousand answers whatever the clock said, which is how a
//! `frame_time` of 1 ms became a tick of 3.58 ms
//! (`docs/worklog/2026-09-20-factory-survey.md`). Now the answering stops when the time is up
//! and the questions it did not reach are put off to the next frame, where they are taken before
//! anything that frame asks.
//!
//! What is checked here is the behaviour and not the clock: how long a tick takes is a
//! measurement (`docs/verification/`, and the instrument in the R4 worklog), and a test that
//! asserted on wall-clock milliseconds would fail on a busy machine for reasons that have
//! nothing to do with rubevy. The three tests below are written so that the thing that ends the
//! frame is **one answer that is certainly longer than `frame_time`** — a closure that sleeps,
//! or a `Rubevy.find` over a world with twenty thousand entities in it — so the frame each
//! question comes back on is a whole number that does not move.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, RubevyPlugin, RubevySet, Script, ScriptWorld};

/// One answer of the game's own, far longer than any `frame_time` in this file. It sleeps rather
/// than spins so that a machine running other work is not made to fight for the core; what the
/// tick's clock sees is the same either way.
const ONE_SLOW_ANSWER: Duration = Duration::from_millis(20);

/// The frame time the slow-closure tests run under: the plugin's own default, which is four
/// times shorter than [`ONE_SLOW_ANSWER`].
const FRAME_TIME: Duration = Duration::from_millis(8);

/// What the scripts reported, in the order the game's answering system took it.
#[derive(Resource, Default)]
struct Said(Vec<(String, f64, f64)>);

/// Something to look for with `Rubevy.find`, on twenty thousand entities.
#[derive(Component, Reflect, Default)]
#[reflect(Component)]
struct Marker;

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "t.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn frames(app: &mut App, n: usize) {
    for _ in 0..n {
        app.update();
    }
}

/// The game's side of `Rubevy.ask`: it answers everything with nothing and writes down what it
/// was told. The scripts report with a question nobody pops.
fn take_what_the_scripts_said(mut scripts: ResMut<ScriptWorld>, mut said: ResMut<Said>) {
    for request in scripts.take_requests() {
        said.0.push((request.kind.clone(), request.num_or(0, -1.0), request.num_or(1, -1.0)));
        scripts.answer(&request, Answer::Nil);
    }
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Said>()
    .register_type::<Marker>()
    .add_systems(Update, take_what_the_scripts_said.in_set(RubevySet::Answer));
    app
}

/// A closure that takes [`ONE_SLOW_ANSWER`] to answer, registered as `"slow"`.
fn install_the_slow_answer(app: &mut App) {
    app.world_mut().resource_mut::<ScriptWorld>().answer_in_tick(
        "slow",
        Box::new(|_world, _request| {
            std::thread::sleep(ONE_SLOW_ANSWER);
            Answer::Num(1.0)
        }),
    );
}

/// `n` scripts, all spawned in the same frame so that they all start in the same one.
fn spawn(app: &mut App, n: usize, src: &dyn Fn(usize) -> String) {
    for i in 0..n {
        let asset = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(&src(i)));
        app.world_mut().spawn(Script::new(asset));
    }
}

/// What each script said it waited, by the id it was given, in the order the reports arrived.
fn reported(app: &App, kind: &str) -> Vec<(f64, f64)> {
    app.world()
        .resource::<Said>()
        .0
        .iter()
        .filter(|(k, _, _)| k == kind)
        .map(|(_, a, b)| (*a, *b))
        .collect()
}

// ---------------------------------------------------------------------------------------------

/// The one this stage is about: three tasks park on a question in the same round, one answer
/// uses the whole frame time, and the other two are answered on the frames after it — each on
/// the next one, because a question put off goes to the head of the next frame's answering.
///
/// **What the numbers count is the frame the script went on in**, not the frame its answer was
/// made in, because that is all a script can see of this (`$rubevy[:frame]` before the question
/// and after it). The first task is answered inside the tick that asked, and still reads `1`:
/// the frame time was up by then, so the round after it never happened and the line following
/// the question ran on the next frame. That number is the same before this stage and after it.
/// What the stage changed is the two behind it — `2` and `3` where all three used to read `1`,
/// because all three answers used to be made whatever the clock said (with `ONE_SLOW_ANSWER`
/// 20 ms and `FRAME_TIME` 8 ms, that was a tick of 60 ms).
#[test]
fn a_question_the_frame_time_did_not_reach_is_answered_on_the_next_frame() {
    let mut app = app();
    app.world_mut().resource_mut::<ScriptWorld>().frame_time = Some(FRAME_TIME);
    install_the_slow_answer(&mut app);
    spawn(&mut app, 3, &|i| {
        format!(
            r#"
              f0 = $rubevy[:frame]
              Rubevy.ask("slow").pop
              Rubevy.ask("waited", {i}.to_f, ($rubevy[:frame] - f0).to_f)
            "#
        )
    });
    frames(&mut app, 12);

    assert_eq!(
        reported(&app, "waited"),
        vec![(0.0, 1.0), (1.0, 2.0), (2.0, 3.0)],
        "one answer a frame, in the order they were asked in"
    );
}

/// The same three tasks, asking over and over: a question that is put off is not put off again
/// in favour of one asked later. Each task gets its turn, and after a dozen frames no task is
/// more than one turn behind another.
///
/// This is the starvation question, and the answer is the order: the leftovers are taken before
/// what the new round asks, so the queue is first-in-first-out across frames and not only
/// within one.
#[test]
fn a_question_put_off_is_not_put_off_again_while_later_ones_are_answered() {
    const FRAMES: usize = 13;
    let mut app = app();
    app.world_mut().resource_mut::<ScriptWorld>().frame_time = Some(FRAME_TIME);
    install_the_slow_answer(&mut app);
    spawn(&mut app, 3, &|i| {
        format!(
            r#"
              loop do
                Rubevy.ask("slow").pop
                Rubevy.ask("turn", {i}.to_f)
              end
            "#
        )
    });
    frames(&mut app, FRAMES);

    let turns = reported(&app, "turn");
    let mut per_task = [0usize; 3];
    for (id, _) in &turns {
        per_task[*id as usize] += 1;
    }
    let most = per_task.iter().max().copied().unwrap();
    let fewest = per_task.iter().min().copied().unwrap();
    assert!(fewest >= 3, "every task had its turns: {per_task:?}");
    assert!(most - fewest <= 1, "and they are within one turn of each other: {per_task:?}");
    // the ids come back 0,1,2,0,1,2,… — the round robin the fairness rests on, which the counts
    // alone would not show
    let order: Vec<usize> = turns.iter().map(|(id, _)| *id as usize).collect();
    for (i, id) in order.iter().enumerate() {
        assert_eq!(*id, i % 3, "the turns go round: {order:?}");
    }
}

/// The floor under all of this: a tick makes **one** answer however late it already is, so a
/// script that never parks cannot stop the others being answered.
///
/// The spinner here has no `sleep` and no question in it, so every run of the VM in this app
/// ends because the frame time is up — which means the answering is entered with the clock
/// already past the deadline, every frame, for ever. A tick that took "past the deadline" to
/// mean "answer nobody" would leave the two readers parked on their first question until the app
/// was closed. They read once a frame instead, which is slow and is not starvation.
///
/// (The budget is wide here on purpose: with the default 200,000 instructions the spinner would
/// run out of budget rather than of time, and what is being tested is the clock.)
#[test]
fn a_script_that_never_parks_does_not_stop_the_others_being_answered() {
    let mut app = app();
    {
        let mut scripts = app.world_mut().resource_mut::<ScriptWorld>();
        scripts.budget = 50_000_000;
        scripts.frame_time = Some(Duration::from_millis(2));
    }
    let spinner = compile("x = 0\nloop { x += 1 }\n");
    let spinner = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(spinner);
    app.world_mut().spawn(Script::new(spinner));
    spawn(&mut app, 2, &|i| {
        format!(
            r#"
              e = Rubevy.entity
              n = 0
              loop do
                e.has?(:Marker)
                n += 1
                Rubevy.ask("read", {i}.to_f, n.to_f)
              end
            "#
        )
    });
    frames(&mut app, 20);

    let reads = reported(&app, "read");
    for id in [0.0, 1.0] {
        let most = reads.iter().filter(|(who, _)| *who == id).map(|(_, n)| *n).fold(0.0, f64::max);
        assert!(most >= 2.0, "reader {id} got its turns: {reads:?}");
    }
}

/// An app that set no `frame_time` is not touched by any of this: every question of the round is
/// answered in the tick that asked it, however long the answering takes, and the clock is never
/// read. Three answers of 20 ms each, all in the frame they were asked in.
#[test]
fn with_no_frame_time_every_question_is_still_answered_in_the_tick_that_asked_it() {
    let mut app = app();
    app.world_mut().resource_mut::<ScriptWorld>().frame_time = None;
    install_the_slow_answer(&mut app);
    spawn(&mut app, 3, &|i| {
        format!(
            r#"
              f0 = $rubevy[:frame]
              Rubevy.ask("slow").pop
              Rubevy.ask("waited", {i}.to_f, ($rubevy[:frame] - f0).to_f)
            "#
        )
    });
    frames(&mut app, 12);

    assert_eq!(
        reported(&app, "waited"),
        vec![(0.0, 0.0), (1.0, 0.0), (2.0, 0.0)],
        "no clock, so the tick answers all three"
    );
}

/// rubevy's own questions take the same road, and this is the expensive one: `Rubevy.find` walks
/// every entity in the world, so with twenty thousand of them a single `find` is longer than the
/// millisecond this app gives a frame. The first is answered — a tick always makes one answer,
/// however late it is, or the run of the VM before it could leave every task parked for ever —
/// and the two behind it wait a frame each.
///
/// The number of entities is what makes this a test and not a race: one answer has to be longer
/// than `frame_time` for the frames to be whole numbers, and twenty thousand entity checks plus
/// twenty thousand `Rubevy::Entity` objects built in the VM is far more than a millisecond in a
/// test build.
///
/// The numbers read `1, 2, 3` for the reason the slow closure's do: the frame a script goes on
/// in is one after the frame its answer was made in when the frame time is already up.
#[test]
fn the_same_holds_for_the_questions_rubevy_answers_itself() {
    let mut app = app();
    app.world_mut().resource_mut::<ScriptWorld>().frame_time = Some(Duration::from_millis(1));
    for _ in 0..20_000 {
        app.world_mut().spawn(Marker);
    }
    spawn(&mut app, 3, &|i| {
        format!(
            r#"
              f0 = $rubevy[:frame]
              Rubevy.find(:Marker)
              Rubevy.ask("waited", {i}.to_f, ($rubevy[:frame] - f0).to_f)
            "#
        )
    });
    frames(&mut app, 12);

    assert_eq!(
        reported(&app, "waited"),
        vec![(0.0, 1.0), (1.0, 2.0), (2.0, 3.0)],
        "one find a frame, in the order they were asked in"
    );
}
