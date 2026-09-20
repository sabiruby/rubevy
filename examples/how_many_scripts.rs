//! **How many scripted entities your machine carries.** A measuring instrument, not a test:
//! nothing in here asserts, and the numbers are of the machine it ran on.
//!
//!     cargo run --release --example how_many_scripts                      # the whole table
//!     cargo run --release --example how_many_scripts -- 90 3 1000 0.004   # one row
//!     cargo run --release --example how_many_scripts -- mem 1000          # memory, alone
//!
//! It is **native only** (`std::time::Instant`, `/proc/self/status`) and it is meant to be run
//! under `taskset -c 2` or the like on a quiet machine; a run of it on one machine, with the
//! before-and-after of the work that made these numbers what they are, is in
//! `docs/verification/scale.md`.
//!
//! Each entity runs the shape a game gives a thing that thinks now and then:
//!
//! ```ruby
//! e = Rubevy.entity
//! loop do
//!   e[:Dial]                          # a component read, answered inside the tick
//!   Rubevy.ask("lookup", e).pop       # a question the game answers inside the tick
//!   Rubevy.ask("woke", frame)         # the instrument: nobody pops it
//!   sleep S
//! end
//! ```
//!
//! The third line is the instrument and not part of the shape: it is how the host learns which
//! frame each script actually ran in, which is the starvation question. It costs one command on
//! the queue and one `Answer::Nil` a turn, and the `+bare` row of each block is the same script
//! without it, so the table says what asking it costs.
//!
//! What comes out, per row:
//!
//! * **the tick**, twice over — `tick_*` is what two systems around `RubevySet::Tick` see, and
//!   `stats_tick` is what rubevy says of the same tick (`ScriptWorld::last_frame().time_ns`).
//!   The two do not measure quite the same thing, and that is why both are here: the set holds
//!   four systems (the tick, then the commands the scripts left, then the two kinds of write),
//!   so a system `.after(RubevySet::Tick)` is timing all four, while `time_ns` is the tick
//!   alone — the thing `frame_time` bounds. Before `FrameStats` there was no way to have the
//!   second number, and the instruments of the survey printed the first one under the name of
//!   the second;
//! * **the frame** — the wall clock of the whole `app.update()`, paced to 60 Hz so that `sleep`,
//!   which is real time in rubevy, means what a game would mean by it;
//! * **what the tick did** — instructions, answers made, questions put off to the next frame,
//!   the longest single answer, and the programs the VM holds (`FrameStats`);
//! * **how many frames apart each script's turns were** — the min, median and max over the
//!   scripts, and the worst single gap, which is where starvation would show;
//! * **the starting frame** — the one frame in which all of them start, with the budget at zero
//!   so that it is `Vm::load` + `task_spawn` and nothing else.
//!
//! **Resident memory is a mode of its own** (`mem`) and not a column. Two RSS readings of one
//! process carry whatever the rows before them left behind, and the allocator gives nothing back
//! between rows: the survey's `rss_start_kb` / `rss_run_kb` columns moved by hundreds of
//! kilobytes between runs of the same configuration (`docs/worklog/2026-09-20-frame-time-as-a-limit.md`).
//! One configuration in a fresh process has neither problem.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, FrameStats, MrbAsset, Request, RubevyPlugin, RubevySet, Script, ScriptWorld};

/// What an entity is, as far as this instrument is concerned: something with a number on it that
/// a script reads and an in-tick answerer looks up. rubevy knows no game's vocabulary, so
/// neither does this.
#[derive(Component, Reflect, Debug, Default)]
#[reflect(Component)]
struct Dial {
    value: f32,
}

// --- the numbers this instrument runs with, and where each of them comes from -----------------
//
// Every one of them is an argument (see `main`); these are the defaults, and each is the value
// the survey of 2026-09-20 used, so that a run of this today can be laid beside the tables in
// `docs/worklog/2026-09-20-factory-survey.md` and the three worklogs after it.

/// Scripted entities per row. From the survey: 10 and 100 are what the two sample games run,
/// 1000 is about where a 60 Hz frame is spent on this machine, and 3000 is past it on purpose.
const SCRIPTS: [usize; 5] = [10, 100, 300, 1000, 3000];

/// Sleeps per row, in seconds. 0.004 is one of mruby-task's ticks (`TICK_UNIT_MS`, 4 ms), so a
/// script that sleeps that long is ready again on the next frame of a 60 Hz app; 0.25 is about
/// fifteen frames, which is the script that looks around now and then.
const SLEEPS: [f64; 2] = [0.004, 0.25];

/// Measured frames per repeat. The survey's `factory_machines 90 3`.
const FRAMES: usize = 90;

/// Repeats of each row, of which the median is printed and the spread of the others beside it.
/// Three, as everything since the survey has used; the instrument prints `(lo..hi)` so that a
/// row whose repeats disagree says so rather than looking like one number.
const REPEATS: usize = 3;

/// Frames run before anything is recorded, so that the scripts have reached their loop and are
/// spread over the frames their `sleep` puts them on. The longest sleep in [`SLEEPS`] is about
/// fifteen frames at 60 Hz, and this is two of those.
const SETTLE: usize = 30;

/// The frame the whole app is paced to, which is what makes `sleep` — real time in rubevy — mean
/// a number of frames. 60 Hz because that is what a game's display asks for.
const FRAME: Duration = Duration::from_nanos(16_666_667);

/// One turn per script, recorded from the `"woke"` question.
#[derive(Default)]
struct Turns {
    count: u32,
    last: u32,
    max_gap: u32,
}

#[derive(Resource, Default)]
struct Woke {
    per_entity: HashMap<Entity, Turns>,
    recording: bool,
}

/// The wall clock of `RubevySet::Tick` alone, taken from outside by the two systems below, so
/// that the rest of the frame (starting scripts, draining commands, the writes, this
/// instrument's own answering system) is not in the number — and beside it what rubevy says of
/// the same tick (`ScriptWorld::last_frame`), which is the point of comparing the two.
#[derive(Resource, Default)]
struct Ticks {
    started: Option<Instant>,
    outside: Vec<Duration>,
    stats: Vec<FrameStats>,
    recording: bool,
}

fn tick_started(mut t: ResMut<Ticks>) {
    t.started = Some(Instant::now());
}

fn tick_ended(mut t: ResMut<Ticks>, scripts: Res<ScriptWorld>) {
    if let (true, Some(at)) = (t.recording, t.started) {
        let took = at.elapsed();
        let stats = scripts.last_frame();
        t.outside.push(took);
        t.stats.push(stats);
    }
}

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "script.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

fn script(sleep_s: f64, instrumented: bool) -> String {
    // the instrument, which the `bare` shape leaves out so that the difference between the two
    // rows is what measuring the turns cost
    let woke = if instrumented { r#"Rubevy.ask("woke", $rubevy[:frame].to_f)"# } else { "" };
    format!(
        r#"
  e = Rubevy.entity
  loop do
    e[:Dial]
    Rubevy.ask("lookup", e).pop
    {woke}
    sleep {sleep_s}
  end
"#
    )
}

/// The game's answering system: it answers nothing but the instrument's question, and records it.
fn answer_and_record(mut scripts: ResMut<ScriptWorld>, mut woke: ResMut<Woke>) {
    for r in scripts.take_requests() {
        if woke.recording && r.kind == "woke" && let Some(entity) = r.entity {
            let frame = r.num_or(0, 0.0) as u32;
            let t = woke.per_entity.entry(entity).or_default();
            if t.count > 0 {
                t.max_gap = t.max_gap.max(frame.saturating_sub(t.last));
            }
            t.count += 1;
            t.last = frame;
        }
        scripts.answer(&r, Answer::Nil);
    }
}

/// The in-tick answerer a game would write: an O(1) look at one component of one entity.
fn install_lookup(mut scripts: ResMut<ScriptWorld>) {
    scripts.answer_in_tick(
        "lookup",
        Box::new(|world: &World, request: &Request| {
            match request.entity_arg(0).and_then(|e| world.get::<Dial>(e)) {
                Some(d) => Answer::Num(d.value as f64),
                None => Answer::Nil,
            }
        }),
    );
}

/// Resident set size in kilobytes, from `/proc/self/status`. Linux only, which this example is.
fn rss_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|n| n.parse().ok()))
        })
        .unwrap_or(0)
}

/// Peak resident set size in kilobytes (`VmHWM`), which is the one an allocator cannot take back.
fn hwm_kb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1).and_then(|n| n.parse().ok()))
        })
        .unwrap_or(0)
}

fn quantile(sorted: &[Duration], q: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

/// The median of one field of the frames' [`FrameStats`], which is how a row says what a typical
/// frame of it did.
fn median_of(stats: &[FrameStats], f: impl Fn(&FrameStats) -> u64) -> u64 {
    if stats.is_empty() {
        return 0;
    }
    let mut v: Vec<u64> = stats.iter().map(&f).collect();
    v.sort_unstable();
    v[v.len() / 2]
}

/// Runs `n` frames at [`FRAME`] and answers with the wall clock of each `app.update()`.
fn paced_frames(app: &mut App, n: usize) -> Vec<Duration> {
    let mut times = Vec::with_capacity(n);
    for _ in 0..n {
        let at = Instant::now();
        app.update();
        let took = at.elapsed();
        times.push(took);
        if let Some(left) = FRAME.checked_sub(took) {
            std::thread::sleep(left);
        }
    }
    times
}

/// The sets of limits a row is measured under, which is how the table says *which* limit a row
/// is spending.
#[derive(Clone, Copy)]
enum Limits {
    /// the plugin's own defaults
    Default,
    /// wide enough that neither limit can be what ended the frame: 100M instructions is five
    /// hundred times the default, and no clock at all
    Generous,
    /// the default budget with an eighth of the frame time: if the tick does not shrink with it,
    /// what it is spending is not inside the answering
    Tight,
    /// `generous` with a clock that never comes due (a second of frame time against a frame that
    /// takes tens of milliseconds): the same work as `generous`, and the only difference between
    /// the two rows is the clock the answering reads after every answer. It is how the cost of
    /// looking at the clock that often is measured rather than guessed.
    Clocked,
}

impl Limits {
    fn name(self) -> &'static str {
        match self {
            Limits::Default => "default(200k/8ms)",
            Limits::Generous => "generous(100M/none)",
            Limits::Tight => "tight(200k/1ms)",
            Limits::Clocked => "clocked(100M/1s)",
        }
    }

    fn apply(self, scripts: &mut ScriptWorld) {
        match self {
            Limits::Default => {
                scripts.budget = 200_000;
                scripts.frame_time = Some(Duration::from_millis(8));
            }
            Limits::Generous => {
                scripts.budget = 100_000_000;
                scripts.frame_time = None;
            }
            Limits::Tight => {
                scripts.budget = 200_000;
                scripts.frame_time = Some(Duration::from_millis(1));
            }
            Limits::Clocked => {
                scripts.budget = 100_000_000;
                scripts.frame_time = Some(Duration::from_secs(1));
            }
        }
    }
}

struct Run {
    tick_median: Duration,
    tick_p95: Duration,
    /// the same tick as rubevy timed it (`FrameStats::time_ns`), for the two to be compared
    stats_tick_median: Duration,
    frame_median: Duration,
    frame_p95: Duration,
    start_frame: Duration,
    insn: u64,
    answers: u64,
    carried: u64,
    longest_answer: Option<u64>,
    programs: usize,
    /// frames per turn = measured frames / turns, per script
    per_turn_min: f64,
    per_turn_median: f64,
    per_turn_max: f64,
    /// the worst single gap any script saw, in frames
    worst_gap: u32,
    /// how many scripts never ran at all
    silent: usize,
}

fn one_run(scripts_n: usize, sleep_s: f64, frames: usize, limits: Limits, instrumented: bool) -> Run {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Woke>()
    .init_resource::<Ticks>()
    .register_type::<Dial>()
    .add_systems(Startup, install_lookup)
    .add_systems(Update, tick_started.before(RubevySet::Tick))
    .add_systems(Update, tick_ended.after(RubevySet::Tick))
    .add_systems(Update, answer_and_record.in_set(RubevySet::Answer));

    let asset = app
        .world_mut()
        .resource_mut::<Assets<MrbAsset>>()
        .add(compile(&script(sleep_s, instrumented)));

    // one frame with nothing in it, so that `Startup` and bevy's own first-frame work are not in
    // the number the starting frame gets
    app.update();

    for i in 0..scripts_n {
        app.world_mut().spawn((Script::new(asset.clone()), Dial { value: i as f32 }));
    }

    // the frame in which `start_scripts` turns every `Script` into a task: one `task_spawn` an
    // entity, and one `Vm::load` for the program they share. The budget is zero, so not one
    // instruction of any script runs in it and the number is the starting and nothing else.
    app.world_mut().resource_mut::<ScriptWorld>().budget = 0;
    let at = Instant::now();
    app.update();
    let start_frame = at.elapsed();
    limits.apply(&mut app.world_mut().resource_mut::<ScriptWorld>());

    // let them reach their loop before anything is counted
    paced_frames(&mut app, SETTLE);
    app.world_mut().resource_mut::<Woke>().recording = true;
    app.world_mut().resource_mut::<Ticks>().recording = true;
    let mut times = paced_frames(&mut app, frames);
    app.world_mut().resource_mut::<Woke>().recording = false;
    app.world_mut().resource_mut::<Ticks>().recording = false;

    let (mut ticks, stats) = {
        let mut t = app.world_mut().resource_mut::<Ticks>();
        (std::mem::take(&mut t.outside), std::mem::take(&mut t.stats))
    };
    ticks.sort();
    let mut stats_ticks: Vec<Duration> =
        stats.iter().map(|s| Duration::from_nanos(s.time_ns)).collect();
    stats_ticks.sort();

    let woke = app.world().resource::<Woke>();
    let mut per_turn: Vec<f64> = Vec::with_capacity(scripts_n);
    let mut worst_gap = 0;
    for t in woke.per_entity.values() {
        per_turn.push(frames as f64 / t.count as f64);
        worst_gap = worst_gap.max(t.max_gap);
    }
    let silent = scripts_n - woke.per_entity.len();
    per_turn.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times.sort();

    Run {
        tick_median: quantile(&ticks, 0.5),
        tick_p95: quantile(&ticks, 0.95),
        stats_tick_median: quantile(&stats_ticks, 0.5),
        frame_median: quantile(&times, 0.5),
        frame_p95: quantile(&times, 0.95),
        start_frame,
        insn: median_of(&stats, |s| s.instructions),
        answers: median_of(&stats, |s| (s.reflect_answers + s.in_tick_answers) as u64),
        carried: median_of(&stats, |s| (s.carried_reflect + s.carried_in_tick) as u64),
        longest_answer: if stats.iter().any(|s| s.longest_answer_ns.is_some()) {
            Some(median_of(&stats, |s| s.longest_answer_ns.unwrap_or(0)))
        } else {
            None
        },
        programs: stats.last().map(|s| s.loaded_programs).unwrap_or(0),
        per_turn_min: per_turn.first().copied().unwrap_or(f64::NAN),
        per_turn_median: per_turn.get(per_turn.len() / 2).copied().unwrap_or(f64::NAN),
        per_turn_max: per_turn.last().copied().unwrap_or(f64::NAN),
        worst_gap,
        silent,
    }
}

/// One configuration in a process of its own, for the memory numbers.
///
///     cargo run --release --example how_many_scripts -- mem 1000 [frames]
fn memory_of_one(scripts_n: usize, frames: usize) {
    let empty = rss_kb();
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Woke>()
    .register_type::<Dial>()
    .add_systems(Startup, install_lookup)
    .add_systems(Update, answer_and_record.in_set(RubevySet::Answer));
    let asset =
        app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(&script(SLEEPS[0], false)));
    app.update();
    let with_vm = rss_kb();
    for i in 0..scripts_n {
        app.world_mut().spawn((Script::new(asset.clone()), Dial { value: i as f32 }));
    }
    app.world_mut().resource_mut::<ScriptWorld>().budget = 0;
    app.update();
    let started = rss_kb();
    Limits::Default.apply(&mut app.world_mut().resource_mut::<ScriptWorld>());
    paced_frames(&mut app, frames);
    let running = rss_kb();
    println!(
        "{scripts_n}\t{empty}\t{with_vm}\t{started}\t{running}\t{}\t{:.2}\t{:.2}",
        hwm_kb(),
        (started as f64 - with_vm as f64) / scripts_n as f64,
        (running as f64 - with_vm as f64) / scripts_n as f64,
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("mem") {
        let scripts_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
        let frames: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(60);
        println!("# resident memory, one configuration in this process (kB)");
        println!("scripts\tempty\twith_vm\tstarted\trunning\tpeak\tkB_per_script_started\tkB_per_script_running");
        memory_of_one(scripts_n, frames);
        return;
    }
    let frames: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(FRAMES);
    let repeats: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(REPEATS);
    let only: Option<usize> = args.get(3).and_then(|s| s.parse().ok());
    let only_sleep: Option<f64> = args.get(4).and_then(|s| s.parse().ok());

    println!("# scripts x sleep, {frames} measured frames paced to {FRAME:?}, {repeats} repeats");
    println!("# tick_* is measured from outside (two systems around RubevySet::Tick); stats_tick is rubevy's own (FrameStats::time_ns)");
    println!("# insn/answers/carried/longest are medians of FrameStats over the measured frames; memory is the `mem` mode");
    println!(
        "scripts\tsleep_s\tlimits\ttick_median\ttick_p95\tstats_tick\tframe_median\tframe_p95\tstart_frame\t\
         insn\tanswers\tcarried\tlongest_answer\tprograms\tframes_per_turn(min/med/max)\tworst_gap\tsilent"
    );

    for scripts_n in SCRIPTS {
        if only.is_some_and(|n| n != scripts_n) {
            continue;
        }
        for sleep_s in SLEEPS {
            if only_sleep.is_some_and(|s| s != sleep_s) {
                continue;
            }
            // the last row of each block is the same script with the instrument taken out, so
            // that the two `default` rows say what asking `"woke"` every turn costs
            for (limits, instrumented) in [
                (Limits::Default, true),
                (Limits::Generous, true),
                (Limits::Tight, true),
                (Limits::Clocked, true),
                (Limits::Default, false),
            ] {
                let mut runs: Vec<Run> = (0..repeats)
                    .map(|_| one_run(scripts_n, sleep_s, frames, limits, instrumented))
                    .collect();
                runs.sort_by_key(|r| r.frame_median);
                let mid = &runs[runs.len() / 2];
                let lo = runs.first().unwrap().frame_median;
                let hi = runs.last().unwrap().frame_median;
                println!(
                    "{scripts_n}\t{sleep_s}\t{}{}\t{:?}\t{:?}\t{:?}\t{:?} ({:?}..{:?})\t{:?}\t{:?}\t\
                     {}\t{}\t{}\t{}\t{}\t{:.1}/{:.1}/{:.1}\t{}\t{}",
                    limits.name(),
                    if instrumented { "" } else { "+bare" },
                    mid.tick_median,
                    mid.tick_p95,
                    mid.stats_tick_median,
                    mid.frame_median,
                    lo,
                    hi,
                    mid.frame_p95,
                    mid.start_frame,
                    mid.insn,
                    mid.answers,
                    mid.carried,
                    match mid.longest_answer {
                        Some(ns) => format!("{ns} ns"),
                        // no `frame_time`, so the tick read no clock and has nothing to say
                        None => String::from("-"),
                    },
                    mid.programs,
                    mid.per_turn_min,
                    mid.per_turn_median,
                    mid.per_turn_max,
                    mid.worst_gap,
                    mid.silent,
                );
            }
        }
    }
}
