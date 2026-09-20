# 2026-09-20 イベント大量・スクリプト数百本の実測（工場ゲームの下調べ）

調査のみ。`src/` は 1 行も変えていない。rubevy_games の 3 本目（Factorio 風の 2D 工場）の計画の前に、
rubevy に新しくかかる 2 つの負荷 — **大量のイベント**と**スクリプト付きエンティティ数百台** — を測った。
ゲーム側のまとめは rubevy_games `docs/worklog/2026-09-20-factory-survey.md`。

## 測った条件

- 13th Gen Intel Core i7-13700、WSL2、rustc 1.97.0、`--release`、`taskset -c 2`（P コア固定）。ほかのベンチは走っていない。
- **sabiruby 0.5.2（crates.io）/ bevy 0.19.1**。main に `.cargo/config.toml` は無く、隣の checkout は使っていない。
  worktree で patch を使うなら config のコピーが要る（cargo の config は cwd から上へ辿るので `rubevy/.cargo/` は `rubevy-wt-*/` に効かない）。
- 計測コードは worktree `rubevy-wt-factory-survey`（ブランチ `factory-survey`、`fb4f398` から）の
  `examples/factory_events.rs` と `examples/factory_machines.rs`。**未コミット**。ヘッドレス（`MinimalPlugins`）、ネイティブ専用。
- 再現: `CARGO_TARGET_DIR=../rubevy/target cargo build --release --example factory_events --example factory_machines`、
  `taskset -c 2 …/factory_events 60 3`（約 4 分）、`taskset -c 2 …/factory_machines 90 3`（約 6 分）、
  `…/factory_machines mem <台数> 60`（メモリは台数ごとに別プロセス）。

## コードから分かったこと

| 事実 | 場所 |
|---|---|
| 購読キューの上限は `const QUEUE_LIMIT: usize = 64`、全 VM 共通、変えられない。**測って決めた数ではない**（rustdoc と 2026-09-15 の worklog の根拠は「数フレームに 1 回読むスクリプトには十分、読まないスクリプトのコストは小さい」という定性だけ） | `src/lib.rs:1195`, `:1187-1194` |
| あふれたら**古い方から黙って捨てる**。ログ・カウンタ・エラー無し。スクリプトが見られるのは `q.size` まで、何件落ちたかは分からない | `make_room` `src/lib.rs:1123-1139`、`src/prelude.rb:86-95` |
| `publish` は毎回、購読リスト全体を線形走査する（誰も聞いていない名前でも） | `src/lib.rs:1099-1107` |
| `publish` は購読者ごとに値を作り直す（`payload.clone()` → `build(&mut vm)` → push） | `src/lib.rs:1086-1089`, `:1108-1115` |
| push 1 回ごとに VM が `Q_WAITING` を丸ごと clone して走査する（待ち手 0 でも） | sabiruby `ext_task.rs:481-490`, `:814-822`, `:84-90` |
| 命令予算 `budget` / `frame_time` / `overrun` は **VM に 1 組**（既定 200,000 / 8 ms / 50 ms）。タスク間では分けず、早い者勝ちの共有の池 | `src/lib.rs:714-721`, `:818-820`, `:1686-1721` |
| ready キューは優先度順、同じ優先度の中は FIFO のラウンドロビン。**同じ優先度なら飢えない**。優先度を分ければ番号の大きい側は飢えうる | sabiruby `ext_task.rs:71-79`, `:380-383` |
| 眠っているタスクは命令を食わないが、`wake_sleepers` が `Q_WAITING` を clone して走査するので**スケジューラの費用は待ちタスク数に比例** | sabiruby `ext_task.rs:277-300` |
| **`frame_time` は上限ではない**。時計は答えループの周回の頭でしか見ず、`answer_reflect_requests` / `answer_in_tick_requests` は時間で区切られない | `src/lib.rs:1691-1696` |
| **同じ `.mrb` を N 台に載せると irep は N 部**。`start_scripts` がエンティティごとに `vm.load` する | `src/lib.rs:1560-1588`、sabiruby `src/vm.rs:1972-1995` |
| `vm.ireps` から消す箇所は sabiruby に 1 つも無い（grep で確認）。**スクリプトを差し替えるたびに irep が積み上がる** | sabiruby `src/` |

## 実測 1: イベント

購読者 S 本（`Rubevy.subscribe(:belt)` して `pop` して数えるだけ）× 1 フレームに publish P 件（`.before(RubevySet::Tick)`）。
60 フレーム、3 回の中央値。`read` は 1 本が 60 フレームで読めた件数の min/med/max。

| subs | P/frame | frame 中央値 | frame p95 | publish（聞く人あり） | publish（誰も聞かない名前） | published | read min/med/max |
|---|---|---|---|---|---|---|---|
| 1 | 1 | 97.5 µs | 153.8 µs | 1.67 µs | 67 ns | 60 | 60/60/60 |
| 1 | 100 | 211.2 µs | 291.3 µs | 13.2 µs | 962 ns | 6000 | 3840/3840/3840 |
| 1 | 1000 | 292.5 µs | 626.2 µs | 133.9 µs | 7.02 µs | 60000 | 3840/3840/3840 |
| 10 | 100 | 655.2 µs | 771.8 µs | 71.4 µs | 802 ns | 6000 | 3840/3840/3840 |
| 10 | 1000 | 1.458 ms | 1.676 ms | 935.6 µs | 7.24 µs | 60000 | 3840/3840/3840 |
| 100 | 1 | 286.0 µs | 427.9 µs | 22.4 µs | 144 ns | 60 | 60/60/60 |
| 100 | 10 | 945.3 µs | 1.028 ms | 63.4 µs | 431 ns | 600 | 600/600/600 |
| 100 | 100 | 3.601 ms | 3.914 ms | 789.9 µs | 3.67 µs | 6000 | 1920/1983/3840 |
| 100 | 1000 | 11.98 ms | 12.71 ms | 9.118 ms | 33.7 µs | 60000 | 1920/1983/3840 |
| 1000 | 1 | 1.995 ms | 2.552 ms | 704.9 µs | 1.01 µs | 60 | 60/60/60 |
| 1000 | 10 | 4.020 ms | 4.631 ms | 922.0 µs | 2.32 µs | 600 | 255/272/292 |
| 1000 | 100 | 12.55 ms | 14.62 ms | 9.553 ms | 21.4 µs | 6000 | 255/255/319 |
| 1000 | 1000 | 97.22 ms | 100.7 ms | 94.12 ms | 219.3 µs | 60000 | 255/255/319 |

（1×10、10×1、10×10 は取りこぼし 0 で、費用は上下の行の間。）

読み取れること:

1. **購読 1 本が 1 フレームに受け取れるのは 64 件**（3840 = 64 × 60）。60 fps で 3,840 件/秒が天井で、予算を積んでも増えない。
2. push 1 回は読み手が park していなければ約 92〜96 ns、全員が park していると 1000 本で 705 ns（待ちタスク 1 本あたり約 0.53 ns — `Q_WAITING` の clone と走査）。
3. 誰も聞いていない名前への publish も購読 1 件あたり約 0.22 ns かかる（1000 購読で 219 ns/回）。
4. 取りこぼしの原因は 2 つ: `QUEUE_LIMIT`（1×100 で 64%）と**フレームの予算**（1000×10 は 1 本も 64 を超えないのに 272/600）。
5. 1000 購読 × 1000 件/フレームは成立しない（97 ms、うち 94 ms が publish）。

## 実測 2: スクリプト付きエンティティ

各台は `loop { e[:Machine]; Rubevy.ask("decide", e).pop（in-tick）; Rubevy.ask("woke", frame)（計器）; sleep S }`。
90 フレーム、60 Hz にペース、3 回。`limits` は default = 200k/8 ms、generous = 100M/時計なし、tight = 200k/1 ms。

| 台数 | sleep | limits | frame 中央値 | p95 | 1 台が動けた間隔（フレーム）min/med/max | 最悪の間隔 |
|---|---|---|---|---|---|---|
| 100 | .004 | default | 1.451 ms | 2.881 ms | 1.0/1.0/1.0 | 1 |
| 300 | .004 | default | 3.265 ms | 4.713 ms | 1.0/1.0/1.0 | 1 |
| 1000 | .004 | default | 8.308 ms | 9.457 ms | 1.0/1.0/1.0 | 1 |
| 1000 | .004 | generous | 8.279 ms | 9.560 ms | 1.0/1.0/1.0 | 1 |
| 1000 | .004 | tight | 2.374 ms | 3.069 ms | 6.4/6.9/6.9 | 12 |
| 1000 | .004 | default（計器なし） | 6.404 ms | 7.413 ms | — | — |
| 3000 | .004 | default | 16.11 ms | 22.08 ms | 3.0/3.0/3.0 | 4 |
| 3000 | .004 | generous | 49.95 ms | 55.16 ms | 1.0/1.0/1.0 | 1 |
| 3000 | .004 | tight | 3.585 ms | 5.391 ms | 18.0/22.5/22.5 | 26 |
| 1000 | .25 | default | 393.9 µs | **8.473 ms** | 15.0/15.0/15.0 | 16 |
| 3000 | .25 | default | 377.6 µs | **19.51 ms** | 18.0/18.0/18.0 | 18 |

1. **飢えは無い**。どの行も min ≒ med ≒ max。予算が足りなければ全員が等しく遅くなる。
2. **毎フレーム動く形の限界は約 1000 台**（8.3 ms。generous でも同じなので予算ではなく実費）。3000 台は 16.7 ms に収まらない。
3. 1 台 1 回の素の費用は 1000 台で約 6.4 µs（読み 1 + in-tick の問い 1 + sleep）。300 台 9.0 µs → 3000 台 16.8 µs（計器込み）と**台数とともに増える** — 上の O(待ちタスク数) のキュー操作。
4. `frame_time` 1 ms でも 3000 台のフレームは 3.58 ms、8 ms なら 16.1 ms。**設定値の 2〜4 倍**。
5. **全台が同じ長さ眠ると同じフレームに一斉に起きる**（sleep 0.25: 中央値 0.4 ms、p95 8〜20 ms、最大 26 ms）。sleep をばらすのが前提。
6. 起動（`Vm::load` + `task_spawn`）は 1 台約 2.5 µs。3000 台を 1 フレームで起こすと 8 ms。

メモリ（台数ごとに別プロセス）: VM 込みで約 17.4 MB、起動で 1 台 2.2 kB（irep のコピー）、走らせた後 1 台 8.9 kB。3000 台で 44.3 MB。制約にならない。
（訂正、同日 R2: 起動の 2.2 kB のうち irep は 0.75 kB で、残り 1.46 kB は `task_spawn` 側 — コンテキスト・スタック・Task オブジェクト。`docs/worklog/2026-09-20-one-irep-per-program.md`）

## 工場ゲームにとって

「搬送コアは Rust、Ruby は機械 1 台ごとの低頻度の判断」は今の rubevy でそのまま成立する。
避けることは 2 つ: 搬送物 1 個ごとのイベント、全機械が同じ周期で起きること。どちらもゲーム側の書き方の話。

## 汎用の不足と、足すとしたらの形（選択肢。未着手、著者判断待ち）

工場に限らず、外の利用者が同じ形で困るもの。ゲームの語彙は持ち込まない。

| # | 不足 | 足すとしたら |
|---|---|---|
| A | あふれの通知が無い | `ScriptWorld::dropped() -> u64`（累計）／キューの ivar を増やして `Rubevy::Subscription#dropped` |
| B | 64 に根拠が無く、変えられない | `Rubevy.subscribe(:hit, limit: n)` と `ScriptWorld` の既定値フィールド。既定を据え置くなら rustdoc に「測った数ではない」と書く |
| C | 購読の索引が無い | `HostState.subscriptions` を名前で引く map に。公開 API 不変。効果に対して中が小さい |
| D | publish が購読者ごとに値を作り直す | `publish_shared`（1 度作って同じ `Value` を配る。受け手が書き換えると全員に見えるので数値と凍った値向け） |
| E | `frame_time` が上限でない | (i) 答えループの中でも時計を見て残りを次フレームへ（持ち越しの道は既にある）／(ii) 上限ではないと文書に書き目安を載せる |
| F | 1 本の `.mrb` を N 台で共有できない | `ScriptWorld` に `AssetId → IrepId` の表。`task_spawn` は `IrepId` を取るので VM 側は不変。差し替えで外す |
| G | 待ちタスクの費用が O(N) | **sabiruby 側**: キューごとの待ち手リスト、sleep 期限のヒープ、タスクが自分のいるキューを覚える |
| H | フレーム単位の可視化が無い | `ScriptWorld::last_frame() -> FrameStats { instructions, rounds, tasks_run, tasks_ready_at_end, answered }` |
| — | irep が解放されない（差し替えで積み上がる） | **sabiruby 側**。F と合わせて考える |

計測の 2 本は、語彙を外して `examples/how_many_scripts.rs` / `how_many_subscribers.rs` として常設する価値がある
（外の利用者が「自分の機械で何本まで載るか」を測る道具は今 `tests/read_cost.rs` しか無い）。

## 測っていないこと

入れ子の payload（Hash / Array）の publish、エンティティ宛の publish、1 台が購読を複数本持つ形、優先度を分けたときの飢え、
走行中の大量の建設・撤去、wasm。
