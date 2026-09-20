# 2026-09-20 `frame_time` を tick の上限にする（R4）

計画書 `docs/plans/generalize-plan.md` の R4（3.4）。ブランチ `generalize`、`d638f89` の上。
調査は `docs/worklog/2026-09-20-factory-survey.md`（実測 2 の 4 番、「`frame_time` 1 ms でも 3000 台のフレームは
3.58 ms、8 ms なら 16.1 ms。**設定値の 2〜4 倍**」）。

## 1. 今の形と、なぜ上限でなかったか

答えループは `src/lib.rs` の `tick_scripts` にある。着手時点（`d638f89`）の形はこう。

```rust
let started_ns = clock_ns();
loop {
    let left = scripts.budget.saturating_sub(spent);
    if left == 0 { break; }
    let time_left = scripts.frame_time
        .map(|t| (t.as_nanos() as u64).saturating_sub(clock_ns().saturating_sub(started_ns)));
    if time_left == Some(0) { break; }
    ...
    match scripts.vm.task_run_limits(limits) { ... }
    let answered = answer_reflect_requests(&*world, scripts)
                 + answer_in_tick_requests(&*world, scripts);
    if answered == 0 { break; }
}
```

時計は **周回の頭** にしかない。`task_run_limits` は渡された残り時間で戻ってくるが、その直後の
`answer_reflect_requests` / `answer_in_tick_requests` は `asked` を端から端まで答える。
`asked` の長さは「その周で読みに park したタスクの数」なので、3000 台のゲームでは 1 周に数千件になる。
1 件 2.2〜2.4 µs（`docs/worklog/2026-09-20-resources-by-name.md` の読み 1 回）× 数千件 = 数ミリ秒が、
時計を一度も見ないまま流れる。これが調査の「2〜4 倍」の中身である。

持ち越しの道はもともとあった。`drain_commands`（`src/lib.rs` の `HostCommand::Ask` の仕分け）が、
予算切れ・時間切れで VM のキューに残った問いを `reflect_requests` / `in_tick_requests` に入れ、
次のフレームの答えループが `std::mem::take` で**最初に**拾う（S1 の `docs/worklog/2026-09-17-sync-reads.md` 1.5）。
R4 がやったのは、この道に「時間切れで答え残した分」も流し込むこと、それだけである。

## 2. 直し

3 つの判断をした。

### 2.1 締切を 1 つの数にする

`tick_scripts` の頭で `deadline_ns: Option<u64>` を作り、周回の頭の判定もこれから引くようにした。

```rust
let started_ns = clock_ns();
let deadline_ns = scripts.frame_time.map(|t| started_ns.saturating_add(t.as_nanos() as u64));
...
let time_left = deadline_ns.map(|deadline| deadline.saturating_sub(clock_ns()));
```

意味は前と同じで、式が 1 つ減る。これを `answer_reflect_requests` / `answer_in_tick_requests` に
引数で渡す。時計は `clock_ns`、つまり **VM に `task_set_clock` で渡してあるのと同じ源**
（`bevy::platform::time::Instant` 起点の経過ナノ秒）。`std::time::Instant` は
`wasm32-unknown-unknown` で `time not implemented on this platform` と panic して最初のフレームで
ページごと落ちる — 2026-09-18 に公開版の庭が黒画面になったのがこれで、記録は
`docs/worklog/2026-09-18-wasm-instant.md`。答えループの中で時計を見る回数が 1 周 1 回から
1 件 1 回に増えた今は、なおさら同じ源であることに意味がある。

### 2.2 1 件答えてから時計を見る（必ず 1 件は答える）

両方の関数の中を、こう変えた。

```rust
let mut asked = asked.into_iter();
while let Some(request) = asked.next() {
    ...答える...
    answered += 1;
    if out_of_time(deadline_ns) {
        scripts.reflect_requests.extend(asked);   // 残りは次のフレームへ
        break;
    }
}
```

**判定を答えの前ではなく後に置いたのは、設計の要点で、思いつきではない。** 前に置くと
「締切を過ぎて入ってきた呼び出しは 1 件も答えない」になる。ところがこの関数に入る直前には
`task_run_limits` が走っており、それが `frame_time` を丸ごと使い切ることがある。そうなると
答えは 0 件、起きるタスクも 0 本 —— 全タスクが問いに park したまま、**永久に**答えをもらえない。

これは推測で捨てたのではなく、実際に書いて測った。判定を前に置いた版を作り（scratchpad の
`r4/before` に私の `src/lib.rs` を写して 1 か所だけ変えたもの）、新しいテストの
`a_script_that_never_parks_does_not_stop_the_others_being_answered` を回すと:

```
thread '...' panicked at tests/frame_time.rs:225:9:
reader 0 got its turns: []
```

`[]` — 眠りも問いもしないスクリプトが 1 本いるだけで、ほかの 2 本は 20 フレームのあいだ
**1 件も**答えをもらえない。採った版では 1 フレームに 1 件ずつ進む。
ついでに分かったこともある: 3000 台の計測器（後述）では、判定を前に置いた版でも飢えは起きなかった
（`tight` で 30/60/60 フレームに 1 回、採った版は 45/90/90）。park したタスクばかりになると
次のフレームの VM の実行がすぐ戻るので、答える時間が空く — 自分で釣り合う形になっている。
**飢えるのは「止まらないタスク」と「読むタスク」が混じったとき**で、それは珍しい形ではない。

`out_of_time` は 3 行の関数にした。`deadline_ns` が `None`（`frame_time` を設定していない app）なら
`is_some_and` が短絡するので **時計を 1 回も読まない**。時計なしの利用者の動きも費用も変わらない。

### 2.3 reflect と in-tick を別々に数える

`answer_reflect_requests` と `answer_in_tick_requests` はそれぞれ 1 件は答える。つまり遅れた tick は
「合わせて 1 件」ではなく「各 1 件」を答える。ゲームの in-tick の問いが rubevy 自身の読みに
（あるいはその逆に）飢やされないため。粒度は「その 2 つの最大値」になるが、どちらも 1 件である。

持ち越しの順序は保つ。`reflect_requests` は `mem::take` した直後なので空で、そこに残りを
`extend` すれば聞かれた順のまま。次のフレームはそれを先に取り、そのあとに今周の分を継ぎ足す。
**先入れ先出しがフレームをまたいで保たれる**ので、同じ問いが後回しにされ続けることがない。

## 3. 時計を見る頻度 — 測って決めた（そして数は足さなかった）

計画書は「毎問か、何問かに 1 回かは**測って決める**（時計 1 回の費用と答え 1 件の費用 2.2 µs の比から）」
と書いている。決め方を 2 本の計測で挟んだ。

**(a) 時計 1 回の直接の費用。** `clock_ns` と同じ形（`OnceLock::get_or_init` +
`elapsed().as_nanos() as u64`）を 500 万回、release、`taskset -c 2`（scratchpad の
`r4/before/examples/clock_cost.rs`）:

```
round 0: 5000000 reads in 83.337972ms — 16ns each
round 1: 5000000 reads in 83.541837ms — 16ns each
round 2: 5000000 reads in 84.478758ms — 16ns each
round 3: 5000000 reads in 83.469672ms — 16ns each
round 4: 5000000 reads in 84.084743ms — 16ns each
round 0: 5000000 empty turns in 626.698µs      ← 空回しのループ自体は 0.12 ns/回
```

**16.7 ns。** 答え 1 件は 2.2〜2.4 µs（component の読み、`tests/read_cost.rs` の流儀で R8 が 3 回測った値）。
比は **0.7%**。

**(b) フレーム全体で見えるか。** 計測器に `clocked(100M/1s)` という行を足した。`generous(100M/none)` と
まったく同じ仕事で、違いは `frame_time` を `Some(1 秒)`（絶対に来ない締切）にしてあること — つまり
**毎問の時計読みだけ**が差である。tick の中央値:

| 台数 | 版 | generous（時計なし） | clocked（毎問読む） | 差 |
|---|---|---|---|---|
| 1000 | 後 (1 回目) | 6.807 ms | 7.018 ms | +3.1% |
| 1000 | 後 (2 回目) | 6.873 ms | 6.860 ms | −0.2% |
| 3000 | 後 | 38.48 ms | 39.02 ms | +1.4% |
| 1000 | 前 | 6.769 ms | 6.800 ms | +0.5% |
| 3000 | 前 | 38.37 ms | 38.82 ms | +1.2% |

前の版は答えの中で時計を読まないので、前の行の差（+0.5%、+1.2%）は**ぶれそのもの**である。
後の版の差はその中に収まっている（1000 台は 2 回で +3.1% と −0.2% に割れた）。
計算でも合う: 1000 台のフレームは答えが約 2000 件（1 台あたり読み 1 + in-tick の問い 1）なので
2000 × 16.7 ns = **33 µs**、6.8 ms の tick の 0.5%。ぶれ（±1〜2%）の下である。

**結論: 毎問見る。新しい数は足さない。** 「何問かに 1 回」にすると、その「何問」は
`ScriptWorld` のフィールドになり、rustdoc に出どころを書き、`docs/numbers.md`（R10）に 1 行増える。
0.7% のために増やす価値がない。**上限の粒度が「答え 1 件」ちょうどになる**という副産物もある
（n 件おきなら粒度は n 件ぶんになる）。

## 4. 前後の表

同じ日・同じ機械・同じ計測器。13th Gen Intel Core i7-13700、WSL2、release、`taskset -c 2`、
90 フレーム（60 Hz にペース）、3 回の中央値。計測器は調査の `examples/factory_machines.rs` に
**tick そのものの実時間**（`RubevySet::Tick` の前後に置いた 2 つのシステムで測る）と
`clocked` の行を足したもの（scratchpad `r4/factory_machines.rs`。R5 で常設するときの材料）。
前の版は `d638f89` を `git worktree add --detach` した別ディレクトリで、**別の `CARGO_TARGET_DIR`** に建て、
`md5sum` が違うことを確かめてある（`12b46aed…` 対 `0d7afdb4…`）。
再現: `taskset -c 2 …/factory_machines 90 3 <台数> 0.004`。

**tick の実時間**（中央値 / p95 / 最大）:

| 台数 | limits | 前（`d638f89`） | 後 |
|---|---|---|---|
| 1000 | default(200k/**8 ms**) | 6.82 / 7.70 / 8.95 ms | 6.95 / 8.04 / 8.42 ms |
| 1000 | tight(200k/**1 ms**) | 2.08 / 3.12 / 3.95 ms | **1.25 / 1.76 / 2.79 ms** |
| 1000 | generous(100M/時計なし) | 6.77 / 7.92 / 10.58 ms | 6.81 / 8.48 / 9.07 ms |
| 3000 | default(200k/**8 ms**) | 14.91 / 21.20 / 22.71 ms | **8.28 / 13.99 / 15.49 ms** |
| 3000 | tight(200k/**1 ms**) | 3.27 / 5.41 / 6.24 ms | **1.38 / 1.92 / 6.12 ms** |
| 3000 | generous(100M/時計なし) | 38.37 / 43.52 / 49.32 ms | 38.48 / 41.32 / 47.55 ms |

* **設定値の 2〜4 倍が、中央値では設定値 + 0.3〜0.4 ms になった。** 3000 台 8 ms は 14.91 → 8.28 ms、
  3000 台 1 ms は 3.27 → 1.38 ms、1000 台 1 ms は 2.08 → 1.25 ms。
* **時計なし（generous）の行は動いていない**（38.37 → 38.48 ms、6.77 → 6.81 ms。上の (b) と同じぶれの中）。
  `frame_time = None` の利用者に払わせたものは無い。
* 1000 台 8 ms の行がほとんど動かないのは、もともと超えていなかったから（6.8 ms < 8 ms）。

**1 台が動けた間隔**（min/med/max フレーム、min ≒ med ≒ max なら飢えていない）:

| 台数 | limits | 前 | 後 |
|---|---|---|---|
| 1000 | default | 1.0/1.0/1.0（最悪 2） | 1.0/1.0/1.0（最悪 1） |
| 1000 | tight | 7.5/8.2/8.2（最悪 15） | 11.2/12.9/12.9（最悪 20） |
| 3000 | default | 3.0/3.0/3.0（最悪 3） | 4.1/4.1/4.3（最悪 7） |
| 3000 | tight | 18.0/18.0/18.0（最悪 26） | 45.0/90.0/90.0（最悪 71） |

**代償はここに出る。** 締切を守るぶん、1 フレームに答える数が減るので、きつい設定では 1 台が動ける間隔が
延びる（3000 台 1 ms で 18 → 45〜90 フレーム）。どの行も min ≒ med ≒ max で、**飢えているタスクは無い**
（`silent` = 一度も動けなかった台数は、計器を持つ行すべてで 0。計器を外した `+bare` の行は
そもそも報告しないので 3000 と出る）。「みんなで等しく遅くなる」という調査 1 の性質は保たれている。
きつい設定でスクリプトを動かしたい app が動かすべき数は `frame_time` であって、この挙動ではない。

## 5. 残っている超過はどこのものか（対照実験）

後の版でも p95 と最大は設定値を超えている（3000 台 8 ms で p95 13.99 ms、3000 台 1 ms で最大 6.12 ms）。
答えの側は締めたのだから、残りは別の場所のはずである。確かめるために、**問いを 1 つも出さないスクリプト**
だけの対照実験を書いた（scratchpad `r4/before/examples/tick_overshoot.rs`。
`loop { 12.times { x += 1 }; sleep 0.004 }` を 3000 本。答えループは 1 周目で「誰にも答えなかった」ので抜ける）:

```
3000 quiet tasks, frame_time Some(8ms)   tick median 4.58ms   p95 6.24ms   max 10.45ms
3000 quiet tasks, frame_time Some(1ms)   tick median 2.93ms   p95 4.15ms   max 4.53ms
3000 quiet tasks, frame_time None        tick median 4.71ms   p95 6.39ms   max 7.97ms
```

**答えが 1 件も無い tick が、1 ms の設定で 2.93 ms かかっている。** これは
`Vm::task_run_limits` が締切に気づく粒度で、待ちタスクが多いほど大きい（push 1 回・起床 1 回ごとに
`Q_WAITING` を走査する。調査の罠の 2 番目、sabiruby `ext_task.rs:814-822`）。rubevy から短くはできない。
計画書 5 章の「VM 側に残るもの」の 1 行目がこれである。数百台では出ない。

というわけで、`frame_time` が**上限と言える範囲**は正直にこう書いた（`ScriptWorld::frame_time` の
rustdoc と `docs/host-api.md` の「Time」）: 超えうるのは (1) 答え 1 件ぶん（必ず 1 件は答えるため。
一番高いのは全エンティティを歩く `Rubevy.find` と、ゲームが書いた in-tick の閉包）、
(2) VM が締切に気づく粒度（待ちタスク数しだい、3000 本で約 2 ms）、(3) 2 本目の VM（足し算されない）、
(4) tick でないシステム（スクリプトの起動、コマンドの実行、フレーム末尾の書き、publish）。

## 6. テスト

`tests/frame_time.rs` を新しく 5 本。**時間に依るテストは揺れやすい**ので、まず
「時計を差し替えられる形で決定的に書けるか」を調べた。

* VM に渡している時計は `vm.task_set_clock(Some(clock_ns))` の 1 行で、渡しているのは `clock_ns`
  **その関数そのもの**（`src/lib.rs:1014`）。テストから差し替えるには「rubevy が VM に渡す関数を
  外から決められる口」を公開フィールドで 1 つ増やし、さらに答えループ側（VM 経由ではなく
  `clock_ns()` を直接呼ぶ）も同じ口を見るように書き換えることになる。テストのためだけに公開 API を
  1 つ増やす形で、しかも増やした口を使う利用者は当面いない。
* 足さずに済ませられた。**決定的にする別の道は「1 件の答えが `frame_time` より確実に長い」状況を作ること**で、
  そうすればフレーム番号が整数として決まる。2 通り用意した: (i) 20 ms 眠る in-tick の閉包（`frame_time` は
  既定の 8 ms）、(ii) 2 万エンティティの世界での `Rubevy.find`（`frame_time` は 1 ms）。
  どちらも「速い機械では落ちる」向きの余裕が数倍あり、混んだ機械では**より確実に**成り立つ。

| テスト | 何を固定するか |
|---|---|
| `a_question_the_frame_time_did_not_reach_is_answered_on_the_next_frame` | 3 本が同じ周で park し、報告は 1/2/3 フレーム。前の版では 1/1/1（全部同じ tick で答えていた） |
| `a_question_put_off_is_not_put_off_again_while_later_ones_are_answered` | 13 フレーム回して、3 本の turn 数の差が 1 以内、順番が 0,1,2,0,1,2,… の輪番（飢えない） |
| `a_script_that_never_parks_does_not_stop_the_others_being_answered` | 止まらないスクリプトが 1 本いても、読む 2 本が進む（2.2 の「必ず 1 件」の床） |
| `with_no_frame_time_every_question_is_still_answered_in_the_tick_that_asked_it` | `frame_time = None` なら 20 ms の答え 3 件でも全部その tick（今までどおり） |
| `the_same_holds_for_the_questions_rubevy_answers_itself` | rubevy 自身の道（`Rubevy.find`）でも同じ 1/2/3 |

**前の版に当てて落ちることを確かめた**（`d638f89` の worktree に同じファイルを置いて実行）:

```
a_question_the_frame_time_did_not_reach…  left: [(0,1),(1,1),(2,1)]  right: [(0,1),(1,2),(2,3)]
the_same_holds_for_the_questions…         left: [(0,1),(1,2),(2,2)]  right: [(0,1),(1,2),(2,3)]
a_script_that_never_parks…                （前の版では通る。2.2 の変種で落ちる）
```

`with_no_frame_time…` と輪番のテストは前の版でも通る。前者は「変えていない」ことの主張なので、それでよい。

書きながら 1 つ直した。最初 3 本の報告を `0/1/2` と書いたら `1/2/3` で落ちた。スクリプトが見られるのは
「答えが作られたフレーム」ではなく「**続きを走れたフレーム**」で、時間切れの tick では答えを受け取っても
その周では走れない（周回の頭の判定で抜ける）から 1 つずれる。1 本目の `1` は前の版でも `1` であり、
R4 が動かしたのは 2 本目以降である — テストの rustdoc にそう書いた。

既存のテストで「時間切れの状況で同じ tick に返ること」を要求しているものは無かった（`frame_time` を触る
テストは `tests/read_cost.rs` の `#[ignore]` の計測 2 本だけ）。止まって報告する事態にはならなかった。

## 7. 文書

* `ScriptWorld::frame_time` の rustdoc — 「tick の上限」と言い切り、**超えうる 4 つ**を並べた（5 章）。
* `docs/host-api.md`「Components by name」— 「A read costs no frame, **except at the tail of a frame that
  ran out of time**」の節を足した（何が起きるか、順序が保たれること、前後の数字）。
* 同「Answering inside the tick」— 閉包も締切の内側にいること、閉包 1 件が `frame_time` より長ければ
  その分だけ tick が延びること、ゲームの種類と rubevy の種類が別々に 1 件ずつ答えられること。
* 同「Time」— 表の `frame_time` の行を書き直し、「何が上限に入らないか」の 4 点を本文に。
  ついでに R8 の気づき（`host-api.md` の Time 節が `Time<Virtual>` を名前で読めることを知らない）を
  1 段落で足した。
* `CHANGELOG.md` の Unreleased に R4 の項（merge の SHA は空けてある）。

## 8. 捨てた案

1. **`ScriptWorld` に「何件ごとに時計を見るか」のフィールド。** 3 章のとおり、0.7% のために数を 1 つ増やし、
   上限の粒度を 1 件から n 件に広げることになる。測った結果として捨てた。
2. **締切を過ぎていたら 1 件も答えない。** 2.2 のとおり、止まらないタスクが 1 本いるだけで読み手が
   永久に飢える。実際に書いて落ちるのを見た。
3. **答えループ全体に「1 周あたりの答えの上限」を置く。** 数が 1 つ増えるうえ、S1 が
   `ROUNDS_PER_TICK = 64` を著者の指摘で外したのと同じ話になる（`docs/worklog/2026-09-17-sync-reads.md` 1.2）。
   時間で切るのだから時間で切ればよい。
4. **時間切れの問いを VM のキューに戻して `drain_commands` に任せる。** `take_asks` が既に取り外した後なので
   戻す口が無く、戻せたとしても順序が今周の分の後ろになる。持ち越しの vector に入れるほうが素直。
5. **テストのために時計を差し替える口を公開する。** 6 章のとおり、足さずに決定的に書けた。

## 9. 確認

| 確認 | 結果 |
|---|---|
| `cargo test --workspace` | **140 passed / 3 ignored**（着手前 135/3） |
| `cargo clippy --workspace --all-targets` | 警告 2 件、どちらも既存（`src/lib.rs` の `collapsible_if`、`examples/headless.rs` の `type_complexity`）。増減なし |
| `cargo build --target wasm32-unknown-unknown --lib` | 通る |
| `cargo test --test no_wasm_unsupported` | 2 passed（`src/` に wasm で落ちる std の呼び出しは無い） |
| `cargo doc --no-deps` | 警告 0 |
| 公開 API | 足しても変えてもいない（変えたのは private 関数 2 つの引数）。依存の追加なし、`unsafe` 0、新しい数 0 |

計測のとき、`pgrep -af "cargo|rustc|docker run|chrome|wasm-opt|wasm-bindgen"` は空だった
（隣の担当の Playwright は計測の 10 分前に終わっていて、load average が 9 から 1 まで下がるのを待った）。

## 10. 気づいた点

* **`RubevySet::Tick` の実時間を外から測る口が無い。** 計測器は `.before(RubevySet::Tick)` と
  `.after(RubevySet::Tick)` に自前のシステムを置いて `Instant` で挟んだ。`frame_time` が守られている
  ことを app が自分で確かめる道が今は無い、ということでもある。R5 の `FrameStats` に「tick の実時間」を
  入れる案は計画書にあるので、そこで解ける。**属する先: rubevy（R5 の材料）**
* **持ち越した問いの数を誰も数えていない。** R4 は `reflect_requests` / `in_tick_requests` に積むが、
  その数は Rust からも Ruby からも見えない。「うちのゲームは毎フレーム何件が翌フレームに回っているのか」は
  `frame_time` を選ぶために知りたい数で、R5 の `FrameStats` の候補にも挙がっている。
  **属する先: rubevy（R5 の材料）**
* **`Vm::task_run_limits` が締切に気づく粒度が、待ちタスク数に比例して粗くなる**（5 章。3000 本で
  1 ms の設定に対し 2.93 ms）。`frame_time` を守りたい app にとっては、rubevy 側を締めても
  ここが残る。計画書 5 章の「キューごとの待ち手リスト・sleep 期限のヒープ」がそのまま効く。
  **属する先: VM（sabiruby の計画）**
* **`ScriptWorld::frame_time` の既定 8 ms の出どころが rustdoc に無い**（`budget` 200,000、
  `overrun` 50 ms も同じ）。R4 でこのフィールドの rustdoc を書き直したが、既定値の理由は
  書けるものが手元に無かったので触っていない。R10 の一覧（`docs/numbers.md`）の対象。
  **属する先: 計画書（R10 の範囲）**
* **計測器 `factory_machines` の `rss_start_kb` / `rss_run_kb` の列が 0 や 1448 と暴れる**（同じ設定の
  2 回の実行で 0 と 952）。アロケータが前の行の領域を使い回すからで、調査のときから分かっていた
  （だから台数ごとに別プロセスで測る `mem` モードがある）。R5 で常設するときは、この 2 列を表から
  外すか「別プロセスで測れ」と書いておかないと、読む人が意味のある数だと思う。
  **属する先: rubevy（R5 の材料）**
* **遅い in-tick の閉包を書いたゲームは、`frame_time` を守れない。** 閉包 1 件が `frame_time` より
  長ければ tick はその長さになる（R4 は「必ず 1 件」なので止められない）。文書には書いたが、
  「閉包が長すぎる」と気づける道は無い（warn も統計も無い）。要る利用者が出てからで良いと思うが、
  出どころの記録として残す。**属する先: rubevy（口の設計）**
