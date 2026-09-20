//! **How many subscribers and how many messages a frame your machine carries.** A measuring
//! instrument, not a test: nothing in here asserts, and the numbers are of the machine it ran on.
//!
//!     cargo run --release --example how_many_subscribers              # the matrix
//!     cargo run --release --example how_many_subscribers -- 60 3      # frames, repeats
//!     cargo run --release --example how_many_subscribers -- limit     # what one reader can take
//!     cargo run --release --example how_many_subscribers -- mem       # what a waiting message costs
//!
//! It is **native only** (`std::time::Instant`, `/proc/self/status`) and it is meant to be run
//! under `taskset -c 2` or the like on a quiet machine; a run of it on one machine, with the
//! before-and-after of the work that made these numbers what they are, is in
//! `docs/verification/scale.md`.
//!
//! **The matrix** is subscribers S × published-per-frame P. Each subscriber is one script that
//! subscribes to `:belt` and does nothing but `pop` and count; the host publishes P messages a
//! frame to that name, and P to a name **nobody** subscribed to — the second is the cost of a
//! publish that reaches no one, which is what a game pays for an event it puts out whether or
//! not anything is listening.
//!
//! **The `limit` mode** raises `ScriptWorld::queue_limit` row by row against a host that
//! publishes far more a frame than any of those limits holds. What a script reads per frame
//! follows the limit while the limit is the binding constraint and stops following it once the
//! frame's budget is; where it stops is the number a default should be compared against.
//!
//! **The `mem` mode** fills the queues of subscribers that never read, and the resident set size
//! before and after says what one waiting message costs — for three shapes of payload, because a
//! number and a string are not the same thing in a VM's heap.
//!
//! **What the instrument itself costs.** A cell times one frame's worth of publishing with two
//! `Instant::now()` calls and divides, so the pair lands whole on the first message: at P = 1
//! that is the whole number (100–1300 ns, measured 2026-09-20). The default columns therefore
//! start at P = 10, the cost of one pair is measured and printed in the header so that it can be
//! subtracted, and a column of 1 is still reachable by argument.

use std::time::{Duration, Instant};

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use rubevy::{Answer, MrbAsset, RubevyPlugin, RubevySet, Script, ScriptWorld};

/// One subscriber: subscribe, then pop for ever. A negative message is the host saying the
/// measurement is over, and the count goes back in a question nobody answers before the script
/// ends.
const SUBSCRIBER: &str = r#"
  q = Rubevy.subscribe(:belt)
  n = 0
  loop do
    v = q.pop
    break if v < 0.0
    n += 1
  end
  Rubevy.ask("received", n.to_f)
"#;

/// Subscribes and then never reads: the queue fills to its limit and stays there. The sleep is
/// longer than any run of this instrument.
const SLEEPER: &str = r#"
  q = Rubevy.subscribe(:belt)
  Rubevy.ask("ready", 0.0)
  sleep 3600
"#;

// --- the numbers this instrument runs with, and where each of them comes from -----------------
//
// Each is the value the survey of 2026-09-20 used, so that a run of this today can be laid beside
// the tables in `docs/worklog/2026-09-20-factory-survey.md` and the worklogs of R1 and R3.
//
// **Which of them an argument can move** (see `main`): the frames and the repeats of every mode,
// and the `mem` mode's two. The shape of the table — the subscribers of each row, the messages of
// each column, the limits the `limit` mode walks — is not an argument: a cell is only worth
// reading beside the others of its row, so what this instrument offers is the whole table or a
// mode of it, and a run of one cell is a line changed here.

/// Subscribers per row, from the survey: one, and then the three powers of ten a game might
/// plausibly reach.
const SUBSCRIBERS: [usize; 4] = [1, 10, 100, 1000];

/// Messages published per frame, per column. The survey used 1, 10, 100 and 1000; the 1 is gone
/// because the instrument's own two clock reads land whole on that single message (see above),
/// and a column that is mostly the instrument is a column that misleads.
const PER_FRAME: [usize; 3] = [10, 100, 1000];

/// Measured frames per repeat, and repeats per cell. The survey's `factory_events 60 3`.
const FRAMES: usize = 60;
const REPEATS: usize = 3;

/// The queue limits the `limit` mode walks, from R3
/// (`docs/worklog/2026-09-20-overflow-and-limits.md`): the default 64 with powers of two around
/// it, up to one that is past what a frame's budget lets a script read at all.
const QUEUE_LIMITS: [usize; 10] = [1, 16, 64, 128, 256, 512, 1024, 2048, 4096, 8192];

/// What the `limit` mode publishes a frame: the largest of [`QUEUE_LIMITS`], so that for every
/// row but the last it is the limit and not the publishing that the reader is up against.
const LIMIT_MODE_PER_FRAME: usize = QUEUE_LIMITS[QUEUE_LIMITS.len() - 1];

/// The `mem` mode's defaults, from R3: a limit large enough that the queues hold a number of
/// messages worth weighing, and subscribers enough that one process's RSS moves by more than its
/// noise.
const MEM_LIMIT: usize = 1000;
const MEM_SUBSCRIBERS: usize = 10;

/// How many messages the host puts out per frame, and whether it is putting any out at all.
#[derive(Resource, Default)]
struct Plan {
    per_frame: usize,
    publishing: bool,
}

/// The wall clock of the publishing system itself, summed over the frames it ran in, and what
/// rubevy said of those frames.
#[derive(Resource, Default)]
struct PublishCost {
    /// `publish(None, "belt", ...)` — the name every script subscribed to
    heard: Duration,
    /// `publish(None, "nobody", ...)` — a name nobody subscribed to, for the cost of a publish
    /// that reaches no one
    unheard: Duration,
    frames: usize,
    /// `FrameStats::dropped` summed over the measured frames: what the queues lost because a
    /// subscriber was behind. Before `ScriptWorld::dropped` this instrument could only infer it
    /// by counting what the scripts said they had read.
    dropped: u64,
    /// `FrameStats::time_ns` of each measured frame, for the median
    tick_ns: Vec<u64>,
}

/// What the scripts said they had read, one entry per script.
#[derive(Resource, Default)]
struct Received(Vec<f64>);

fn compile(src: &str) -> MrbAsset {
    let opts = sabiruby_compiler::Options { filename: "subscriber.rb".into(), ..Default::default() };
    MrbAsset { bytes: sabiruby_compiler::compile(src.as_bytes(), &opts).expect("compiles") }
}

/// Publishes before the tick, so a message put out this frame can be read by a script this frame.
fn publish_belt(mut scripts: ResMut<ScriptWorld>, plan: Res<Plan>, mut cost: ResMut<PublishCost>) {
    if !plan.publishing {
        return;
    }
    let at = Instant::now();
    for i in 0..plan.per_frame {
        // positive: the sentinel that ends a script is the only negative one
        scripts.publish(None, "belt", Answer::Num(i as f64 + 1.0));
    }
    cost.heard += at.elapsed();
    let at = Instant::now();
    for i in 0..plan.per_frame {
        scripts.publish(None, "nobody", Answer::Num(i as f64));
    }
    cost.unheard += at.elapsed();
    cost.frames += 1;
}

/// After the tick: what rubevy says this frame's tick came to. A game's HUD is this system.
fn record_frame(scripts: Res<ScriptWorld>, plan: Res<Plan>, mut cost: ResMut<PublishCost>) {
    if !plan.publishing {
        return;
    }
    let f = scripts.last_frame();
    cost.dropped += f.dropped;
    cost.tick_ns.push(f.time_ns);
}

fn answer_nil(mut scripts: ResMut<ScriptWorld>, mut got: ResMut<Received>) {
    for r in scripts.take_requests() {
        if r.kind == "received" {
            got.0.push(r.num_or(0, -1.0));
        }
        scripts.answer(&r, Answer::Nil);
    }
}

fn app(src: &str, subscribers: usize) -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_once()),
        bevy::asset::AssetPlugin::default(),
        RubevyPlugin::default(),
    ))
    .init_resource::<Plan>()
    .init_resource::<PublishCost>()
    .init_resource::<Received>()
    .add_systems(Update, publish_belt.before(RubevySet::Tick))
    .add_systems(Update, record_frame.after(RubevySet::Tick))
    .add_systems(Update, answer_nil.in_set(RubevySet::Answer));

    let asset = app.world_mut().resource_mut::<Assets<MrbAsset>>().add(compile(src));
    for _ in 0..subscribers {
        app.world_mut().spawn(Script::new(asset.clone()));
    }
    app
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

/// Runs `n` frames and answers with the wall clock of each `app.update()`. The 2 ms between them
/// is what `tests/read_cost.rs` does: it keeps bevy's `Time` moving and keeps the frame's own
/// cost, rather than the machine's scheduling, in the number.
fn timed_frames(app: &mut App, n: usize) -> Vec<Duration> {
    let mut times = Vec::with_capacity(n);
    for _ in 0..n {
        std::thread::sleep(Duration::from_millis(2));
        let at = Instant::now();
        app.update();
        times.push(at.elapsed());
    }
    times
}

fn quantile(sorted: &[Duration], q: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

fn median_ns(v: &mut [u64]) -> u64 {
    if v.is_empty() {
        return 0;
    }
    v.sort_unstable();
    v[v.len() / 2]
}

/// What one pair of `Instant::now()` costs on this machine, which is what a cell's publish
/// number carries once and what the P = 1 column would carry whole.
fn instrument_cost() -> Duration {
    // 100,000 turns is a few milliseconds at the tens of nanoseconds a pair costs, which makes
    // the one pair of readings around the loop a millionth of what is being divided. The same
    // shape `clock_cost` used in R4 (`docs/worklog/2026-09-20-frame-time-as-a-limit.md`).
    const TURNS: u32 = 100_000;
    let at = Instant::now();
    let mut sum = Duration::ZERO;
    for _ in 0..TURNS {
        let a = Instant::now();
        sum += a.elapsed();
    }
    std::hint::black_box(sum);
    at.elapsed() / TURNS
}

struct Cell {
    frame_median: Duration,
    frame_p95: Duration,
    stats_tick_median: Duration,
    publish_heard: Duration,
    publish_unheard: Duration,
    published: usize,
    dropped: u64,
    read_min: f64,
    read_median: f64,
    read_max: f64,
    reported: usize,
}

fn one_cell(subscribers: usize, per_frame: usize, frames: usize) -> Cell {
    let mut app = app(SUBSCRIBER, subscribers);

    // the frames in which the scripts start and reach their `subscribe`
    for _ in 0..8 {
        app.update();
    }
    let standing = app.world().resource::<ScriptWorld>().subscriptions();
    assert_eq!(standing, subscribers, "every script should be subscribed before the measurement");

    app.world_mut().resource_mut::<Plan>().per_frame = per_frame;
    app.world_mut().resource_mut::<Plan>().publishing = true;
    let mut times = timed_frames(&mut app, frames);
    app.world_mut().resource_mut::<Plan>().publishing = false;

    let (heard, unheard, publish_frames, dropped, mut tick_ns) = {
        let cost = app.world().resource::<PublishCost>();
        (cost.heard, cost.unheard, cost.frames, cost.dropped, cost.tick_ns.clone())
    };
    let published = per_frame * publish_frames;

    // the sentinel, and then frames enough for every script to drain its queue and end. The
    // budget is lifted for the draining only: it is not part of anything measured above.
    {
        let mut scripts = app.world_mut().resource_mut::<ScriptWorld>();
        scripts.publish(None, "belt", Answer::Num(-1.0));
        scripts.budget = u64::MAX;
        scripts.frame_time = None;
    }
    for _ in 0..200 {
        app.update();
        if app.world().resource::<Received>().0.len() == subscribers {
            break;
        }
    }

    let mut read = app.world().resource::<Received>().0.clone();
    read.sort_by(|a, b| a.partial_cmp(b).unwrap());
    times.sort();

    Cell {
        frame_median: quantile(&times, 0.5),
        frame_p95: quantile(&times, 0.95),
        stats_tick_median: Duration::from_nanos(median_ns(&mut tick_ns)),
        publish_heard: heard / publish_frames.max(1) as u32,
        publish_unheard: unheard / publish_frames.max(1) as u32,
        published,
        dropped,
        read_min: read.first().copied().unwrap_or(f64::NAN),
        read_median: read.get(read.len() / 2).copied().unwrap_or(f64::NAN),
        read_max: read.last().copied().unwrap_or(f64::NAN),
        reported: read.len(),
    }
}

/// How many one subscriber reads in a frame at a given limit, and what those frames cost.
fn read_ceiling(limit: usize, per_frame: usize, frames: usize) -> (f64, u64, Duration, Duration) {
    let mut app = app(SUBSCRIBER, 1);
    app.world_mut().resource_mut::<ScriptWorld>().queue_limit = limit;
    for _ in 0..8 {
        app.update();
    }
    assert_eq!(app.world().resource::<ScriptWorld>().subscriptions(), 1, "subscribed");

    app.world_mut().resource_mut::<Plan>().per_frame = per_frame;
    app.world_mut().resource_mut::<Plan>().publishing = true;
    let mut times = timed_frames(&mut app, frames);
    app.world_mut().resource_mut::<Plan>().publishing = false;
    let dropped = app.world().resource::<ScriptWorld>().dropped();

    // the sentinel, then frames enough for the script to end and report; the budget is lifted
    // for the draining only, which is after everything measured above
    {
        let mut scripts = app.world_mut().resource_mut::<ScriptWorld>();
        scripts.publish(None, "belt", Answer::Num(-1.0));
        scripts.budget = u64::MAX;
        scripts.frame_time = None;
    }
    for _ in 0..500 {
        app.update();
        if !app.world().resource::<Received>().0.is_empty() {
            break;
        }
    }
    let read = app.world().resource::<Received>().0.first().copied().unwrap_or(f64::NAN);
    times.sort();
    (read / frames as f64, dropped, quantile(&times, 0.5), quantile(&times, 0.95))
}

/// What a message waiting in a queue costs, for three shapes of payload.
fn message_memory(limit: usize, subscribers: usize) {
    for kind in ["num", "text", "list"] {
        let mut app = app(SLEEPER, subscribers);
        app.world_mut().resource_mut::<ScriptWorld>().queue_limit = limit;
        // every script subscribed and went to sleep
        for _ in 0..60 {
            app.update();
            if app.world().resource::<ScriptWorld>().subscriptions() == subscribers {
                break;
            }
        }
        assert_eq!(app.world().resource::<ScriptWorld>().subscriptions(), subscribers);
        // let the collector settle before the first reading, so what the difference shows is the
        // messages and not a heap that happened to grow under them
        app.update();
        let before = rss_kb();

        {
            let mut scripts = app.world_mut().resource_mut::<ScriptWorld>();
            for i in 0..limit {
                let payload = match kind {
                    "num" => Answer::Num(i as f64),
                    // a short one: a name, an item id — what a game actually publishes
                    "text" => Answer::Text(format!("item{i:04}")),
                    _ => Answer::List(vec![i as f64, 0.5, -1.0, 12.0]),
                };
                scripts.publish(None, "belt", payload);
            }
        }
        let after = rss_kb();
        let held = limit * subscribers;
        let bytes = (after.saturating_sub(before) * 1024) as f64 / held as f64;
        println!("{kind}\t{limit}\t{subscribers}\t{held}\t{before}\t{after}\t{bytes:.1}");
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("mem") => {
            let limit: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(MEM_LIMIT);
            let subscribers: usize =
                args.get(3).and_then(|s| s.parse().ok()).unwrap_or(MEM_SUBSCRIBERS);
            println!("# what one waiting message costs (RSS before and after filling the queues)");
            println!("payload\tlimit\tsubs\tqueued\trss_before_kb\trss_after_kb\tbytes_per_message");
            message_memory(limit, subscribers);
        }
        Some("limit") => {
            let frames: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(FRAMES);
            let repeats: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(REPEATS);
            println!(
                "# one subscriber, {LIMIT_MODE_PER_FRAME} published a frame, {frames} frames, \
                 {repeats} repeats (median), the plugin's own budget and frame_time"
            );
            println!("limit\tread_per_frame\tdropped\tframe_median\tframe_p95");
            for limit in QUEUE_LIMITS {
                // repeats, because a single run of the row where the budget takes over from the
                // limit came out 2.3 times the row above and the row below it (R3, one repeat)
                let mut rows: Vec<(f64, u64, Duration, Duration)> = (0..repeats)
                    .map(|_| read_ceiling(limit, LIMIT_MODE_PER_FRAME, frames))
                    .collect();
                rows.sort_by_key(|row| row.2);
                let (read, dropped, med, p95) = rows[rows.len() / 2];
                println!("{limit}\t{read:.1}\t{dropped}\t{med:?}\t{p95:?}");
            }
        }
        _ => {
            let frames: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(FRAMES);
            let repeats: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(REPEATS);
            let pair = instrument_cost();
            println!(
                "# subscribers x published-per-frame, {frames} measured frames, {repeats} repeats, \
                 queue_limit = {}",
                ScriptWorld::QUEUE_LIMIT
            );
            println!(
                "# one Instant::now() pair costs {pair:?} here: a publish column carries it once, \
                 so it is {:?} of a P=10 cell",
                pair / 10
            );
            println!("# stats_tick and dropped are rubevy's own (FrameStats); dropped is the whole run, not a frame");
            println!(
                "subs\tper_frame\tframe_median\tframe_p95\tstats_tick\tpublish_heard\tpublish_unheard\t\
                 published\tdropped\tread_min\tread_median\tread_max\treported"
            );

            for subscribers in SUBSCRIBERS {
                for per_frame in PER_FRAME {
                    let mut cells: Vec<Cell> =
                        (0..repeats).map(|_| one_cell(subscribers, per_frame, frames)).collect();
                    cells.sort_by_key(|c| c.frame_median);
                    let mid = &cells[cells.len() / 2];
                    let lo = cells.first().unwrap().frame_median;
                    let hi = cells.last().unwrap().frame_median;
                    println!(
                        "{subscribers}\t{per_frame}\t{:?} ({:?}..{:?})\t{:?}\t{:?}\t{:?}\t{:?}\t{}\t{}\t{}\t{}\t{}\t{}",
                        mid.frame_median,
                        lo,
                        hi,
                        mid.frame_p95,
                        mid.stats_tick_median,
                        mid.publish_heard,
                        mid.publish_unheard,
                        mid.published,
                        mid.dropped,
                        mid.read_min,
                        mid.read_median,
                        mid.read_max,
                        mid.reported,
                    );
                }
            }
        }
    }
}
