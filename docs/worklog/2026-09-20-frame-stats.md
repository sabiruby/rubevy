# フレーム単位の統計と、常設の計測器 2 本（R5）

2026-09-20。計画書 `docs/plans/generalize-plan.md` の段階 R5、ブランチ `generalize`（`474112d` の上）。
材料は R1〜R4 の worklog（`2026-09-20-subscription-index.md`、`-one-irep-per-program.md`、
`-overflow-and-limits.md`、`-frame-time-as-a-limit.md`）と、調査の `2026-09-20-factory-survey.md`、
それに各段階がスクラッチパッドに残した計測器。前後の表は `docs/verification/scale.md` に置いた。

## 1. 何が取れて、何が取れないか（先に VM を読んだ）

計画書 3.5 が挙げた候補は 9 つ。**VM と rubevy が既に持っている数だけで組む**という条件なので、
sabiruby 0.5.2 の公開の口（`~/.cargo/registry/.../sabiruby-0.5.2/src/vm.rs:766-970`）と
`tick_scripts`（`src/lib.rs:2352` 付近）の中を突き合わせた。

| 候補 | 取れるか | どこから |
|---|---|---|
| 使った命令数 | 取れる | 答えループが `task_run_limits` の戻りを `spent` に足している。そのまま |
| 答えループの周回数 | 取れる | ループに数え上げを 1 つ足すだけ |
| 走ったタスク数 | **取れない** | VM に「この run で何本のタスクが走ったか」を言う口が無い。タスクごとに `task_instructions` の差を取れば分かるが、それはフレームごとに全タスクを歩くことで、統計のために O(N) を足すことになる |
| フレーム終わりに ready のまま残ったタスク数 | **数では取れない** | `Vm::task_pending()` は `bool`（`ext_task.rs:521`: ready キューが空でないか、期限つきで待っているタスクがあるか）。件数を返す口は無い。**`more_to_run: bool` として入れた** |
| tick の中で答えた問いの数（reflect と in-tick） | 取れる | 2 つの答え関数が既に件数を返している。足し込むだけ |
| 次のフレームへ持ち越した問いの数 | 取れる | R4 が作った `reflect_requests` / `in_tick_requests` の長さ |
| このフレームに落としたメッセージ数 | 取れる | R3 の `dropped()` の差。前フレームの値を 1 つ覚える |
| tick の実時間 | 取れる | `clock_ns`（VM に渡してある時計。`std::time::Instant` は wasm で panic する） |
| 読み込み済みのプログラムの本数 | 取れる | R2 の `programs.len()` |

「いちばん長かった答え 1 件の時間」（R4 の気づいた点）は、**時計を答えごとに既に見ているので
引き算 1 回で取れる**。取ったのが下の 2 節。

## 2. `AnswerClock` — 時計を 2 度読まないための小さな型

R4 の形はこうだった: 答えを 1 件作るごとに `out_of_time(deadline_ns)` を呼び、その中で
`clock_ns()` を読んで締切と比べる。読んだ値は比較に使って捨てていた。
「いちばん長かった答え」は**その捨てていた値の差**なので、読みを増やさずに取れる。

そこで `out_of_time` を `AnswerClock`（`src/lib.rs`、`answer_made`）に替えた。持っているのは
前回の読みの時刻 1 つと、これまでの最大の間隔 1 つ。`answer_made` は時計を 1 回読み、
前回との差を最大に入れ、締切を過ぎたかを返す。**時計を読む回数は R4 と同じ**。

足した読みは 1 か所だけ: 周回の答えが始まる直前（`tick_scripts` の `clock.at_ns = clock_ns()`）。
これが無いと、その周回の最初の 1 件に**その前の VM の実行**が乗ってしまう（答えではない時間を
「答え 1 件」と呼ぶことになる）。`frame_time` が `None` のときは読まない（`deadline_ns.is_some()` で囲った）ので、
時計を使わない app には 1 回も増えない。

`longest_answer_ns` が `Option<u64>` なのはここから決まる: **時計を読んでいないフレームは
「0 ns の答え」ではなく「言うことが無い」**。`0` を返すと「答えは一瞬だった」と読めてしまう。

**この「読み始めの 1 回」を最初は間違った場所に置いていた。** 最初の版は `tick_scripts` の
周回の中（答え関数を呼ぶ直前）で読んでいたが、そこから 1 件目の答えまでの間には
**問いを VM のキューから取り出す仕事**（`take_reflect_asks` / `take_asks`。3000 本が問いを出していれば
数千件の `Request` を作る）と、レジストリの読みが入る。気づいたのは計測器の表で、
1000 台の `いちばん長い答え` が 371 µs、3000 台で 3.96 ms と出ていたからである
（この計測器の in-tick の閉包は `world.get::<Dial>()` 1 回で、µs の仕事しかしていない）。
`clock.start(deadline_ns)` を**2 つの答え関数の中の、取り出しが終わった直後**に移した。
周回ごとの読みが 1 回から最大 2 回になるが、`answer_in_tick_requests` は答え手が登録されていなければ
その前に返るので、in-tick を使わない app では 1 回のままである。
直した効果は前後のバイナリを交互に測って確かめた（1000 台で 326〜753 µs → 20〜70 µs、
tick の実時間は不変）。表は `docs/verification/scale.md` の 2 節。

フレームの終わりには時計をもう 1 回読む（`time_ns` のため）。これは R4 が読まなかった読みで、
`frame_time` が `None` の app にも乗る。1 回 16.7 ns（R4 の `clock_cost`）で、
**tick の実時間はその `frame_time` を選ぶために要る数**（R4 の気づいた点そのもの）なので払うことにした。

## 3. 統計を取ること自体の費用 — 測った

計画書の指示は「取る前後で `how_many_scripts` の定常のフレーム時間がぶれの中であること。
測れる費用があるなら、取る/取らないを設定にする」。

計測器は**前後の両方で建つもの**でなければならない（新しい example は `last_frame()` を使うので前の版では建たない）。
R4 がスクラッチパッドに残した `r4/factory_machines.rs` を両方の worktree の `examples/` に置いて使った。
前は `git worktree add --detach ../rubevy-wt-r5-before 474112d`、**別の `CARGO_TARGET_DIR`**、
`md5sum` で別物だと確認（`249d34c0…` 対 `b83cf992…`）。
走らせ方は R3 と同じ **前 → 後 → 前 → 後 の 2 巡**、毎回 `pgrep` で静けさを確かめて（4 回とも `QUIET`）、
`taskset -c 2`、90 フレーム × 3 回。出力は scratchpad の `r5/paired-1000.txt` と `r5/paired-3000.txt`。

tick の中央値（2 巡の平均、µs）:

| 台数 | limits | 前 | 後 | 差 |
|---|---|---|---|---|
| 1000 | default(200k/8ms) | 6776 | 6845 | +1.0% |
| 1000 | generous(100M/時計なし) | 6805 | 6883 | +1.1% |
| 1000 | tight(200k/1ms) | 1275 | 1257 | −1.4% |
| 1000 | clocked(100M/1s) | 6891 | 6955 | +0.9% |
| 1000 | default+bare | 6315 | 6316 | +0.0% |
| 3000 | default(200k/8ms) | 8772 | 8313 | **−5.2%** |
| 3000 | generous | 38849 | 39167 | +0.8% |
| 3000 | tight | 1325 | 1348 | +1.8% |
| 3000 | clocked | 38913 | 39087 | +0.4% |
| 3000 | default+bare | 8195 | 8219 | +0.3% |

**結論: 測れる費用は無い。設定にしない。** 根拠は 3 つ。

1. **散らばりが両方向で、同じ版の中のぶれより小さい。** 前の版の 2 巡どうしが 3000 台 default で
   8.644 対 8.901 ms（3.0% 違う）。後の版が「+1.0%」の行と「−5.2%」の行を同時に出すのは、
   差ではなくぶれの見え方である。
2. **台数を 3 倍にしても差が増えない。** もし `task_pending()` の待ちタスク走査のような O(N) が
   乗っているなら 3000 台で 3 倍に見えるはずだが、3000 台の default はむしろ速い側に出た。
3. **機構から見積もれる量が小さい。** 1 フレームに増えたのは、周回ごとの時計 1 回（既定の 1000 台で
   4 周前後）＋フレーム末の時計 1 回＝約 5 回 × 16.7 ns、`host_state` の downcast 1 回、
   `task_pending()` 1 回、構造体 1 つの書き込み。合計 100 ns 前後で、6.8 ms の tick の 0.002%。

設定にしなかったのは、その設定が「出どころの無い数と、それを読む分岐」を増やすからである
（CLAUDE.md の禁止の 1 項目め。R4 が「何問かに 1 回」を捨てたのと同じ判断）。

**測れていない最悪の場合は 1 つある**: `Vm::task_pending()` は ready キューが空なら待ちタスクを歩く。
歩いている途中で「期限つきで待っているタスク」に当たれば即座に返るので、`sleep` するスクリプトが
1 本でもいれば早く終わる。**全タスクが期限の無い `q.pop` で止まっている**ゲーム（購読だけの形）では
待ちタスク数ぶん歩く。これは計画書 5 章が sabiruby 側の宿題として挙げている O(待ちタスク数) と同じ根で、
rubevy からは短くできない。上の 2 本の計測はこの形ではないので、**この場合の費用は測っていない**。

## 4. 常設の計測器 2 本

調査の `factory_events.rs` / `factory_machines.rs`（worktree `rubevy-wt-factory-survey` の未コミット）と、
R3 の `r3_limits.rs`、R4 が足した版をまとめて 2 本にした。工場の語彙は入れていない
（`Machine` → `Dial`、`decide` → `lookup`）。冒頭に「ネイティブ専用の計測器、assert しない」と書いた
（`tests/read_cost.rs` と同じ流儀）。

* **`examples/how_many_scripts.rs`** — 原型は `factory_machines`。既定の表、`mem` モード。
* **`examples/how_many_subscribers.rs`** — 原型は `factory_events` に `r3_limits` の 2 モードを畳み込んだもの。
  既定の表（購読者 × 1 フレームの publish 件数）、`limit` モード（上限を動かして 1 本が読める件数）、`mem` モード。

**別の example にせず `limit` と `mem` をモードにしたのは**、外の利用者の入口が
「自分の機械で何本載るか」の 1 コマンドであるべきで、キューの上限の話と溜まった 1 件の重さの話は
どちらも「購読者がどれだけ載るか」の内訳だからである。example が 4 本あると、どれを先に走らせるかを
利用者が決めなければならなくなる。

### 前の担当が指摘した穴 3 つを直した

1. **P=1 の列に計器の値段が丸ごと乗る**（R1）。既定の列を 10・100・1000 にした（1 は引数で出せる）。
   加えて、`Instant::now()` のペア 1 回の値段をその場で 10 万回測って見出しに出すようにした
   （この機械で 32 ns）。減らせない費用なら、読む人が引けるように見せる。
2. **`rss_start_kb` / `rss_run_kb` が同じ設定でも暴れる**（R4）。表から外し、`mem` モードだけにした。
   理由は rustdoc に書いた（同じプロセスの 2 回の RSS は前の行が残したものを抱え、アロケータは行の間で返さない）。
3. **上限 4096 の行だけ中央値が跳ねた**（R3、繰り返し 1 回）。`limit` モードに繰り返しを足し、中央値を出す。

### 数は全部引数、既定値には出どころ

台数・sleep・フレーム数・繰り返し・上限の列・`mem` の既定は、すべて `const` としてファイルの頭に
まとめ、1 つずつ「調査と同じ値」「mruby-task の 1 tick が 4 ms だから」のように出どころを書いた。
引数で上書きできる（`how_many_scripts 90 3 1000 0.004` で 1 行だけ）。

### `FrameStats` を計測器が使う形にした

R4 の計測器は tick の実時間を「`RubevySet::Tick` の前後に置いた 2 つのシステム」で測っていた。
新しい計測器は**その数と `FrameStats::time_ns` の両方**を出す。突き合わせて分かったのは、
**2 つは同じものではない**ということである: `RubevySet::Tick` には 4 つのシステムが入っている
（`tick_scripts`、`drain_commands`、`apply_component_writes`、`apply_resource_writes`。`src/lib.rs:1924` の表）。
外から挟むと 4 つ全部を測る。1000 台で 6.8 ms 対 5.9 ms、100 台で 2.3 ms 対 1.5 ms のように 1 ms 前後ずれる。
**`frame_time` が縛るのは小さいほう**なので、R4 の表の「tick の実時間」はコマンドと書きを含んだ数だった
（R4 の結論は変わらない — 前後で同じ計器を使っているので差は差である）。`docs/verification/scale.md` に書いた。

## 4a. 途中で 1 回、機械に騙された（罠 4）

新しい `how_many_scripts` の表を取ったら、1000 台の `generous` の行が **15.5 ms** で、
同じ設定を原型の `factory_machines` で測ったとき（上の A/B）の **6.8 ms** の 2.3 倍だった。
別プロセスで 1 行だけ走らせても 15.5 ms で、再現する。

計測器の差を疑って、まず 2 本のソースを `diff` した。測られる側の差は無い（語彙と、`FrameStats` の列と、
`rss` の 2 列を外したことだけ）。次に、**`tick_ended` が `Res<ScriptWorld>` を取って `last_frame()` を
読むようになった**ことを疑い、その引数を外した版を建てて測った → **18.7 ms**（むしろ遅い）。
原因ではない。

そこで **2 本を交互に 2 巡**（`compare.sh`）走らせた。結果:

| 巡 | `factory_machines` 1000 generous | `how_many_scripts` 1000 generous |
|---|---|---|
| 1 | 18.39 ms | 6.62 ms |
| 2 | 6.76 ms | 6.85 ms |

**遅いのは計測器ではなく機械の状態だった。** 1 巡目に走ったほうが遅い（どちらの向きでも）。
`pgrep` はどちらの巡でも空である。直前まで別の測定や `cargo build` が走っていた後の 1 回目が遅い、
という形に見える（WSL2 の CPU 周波数か、スケジューラか。**原因は突き止めていない**）。

だから最初の「15.5 ms」は、その前に 10 分走っていた表の続きで取った数字であって、
`how_many_scripts` の数字ではない。**表は取り直した**（`docs/verification/scale.md` の数字は取り直したほう）。
この罠は `docs/verification/scale.md` の再現手順に「罠 4」として書いた。
前の担当が見つけた罠 1〜3（同じ target で 2 つ目が建たない、共有 target の古い成果物、隣の cargo/docker/ブラウザ）と
同じ場所に置いてある。

## 5. 捨てた案

* **`last_frame` を `Option<FrameStats>` にする。** 「まだ tick していない」を型で言える。採らなかったのは、
  HUD が毎フレーム `unwrap_or_default()` を書くことになるのと、既定値（全部 0、`frame: 0`）が
  「何もしていないフレーム」として正しく読めるため。テストに 1 本書いた
  （`before_the_first_tick_it_is_all_zeroes`）。
* **`tasks_run` を `task_instructions` の差で作る。** フレームごとに全タスクを歩くことになる。
  統計のために O(N) を足すのは、統計が「既にある数だけで組む」という条件に反する。報告に書く（VM の口の不足）。
* **`FrameStats` を Ruby から見えるようにする**（`$rubevy` にキーを足す）。計画書が「足さない」と決めている。
  足すなら「何を見せるか」を決める必要があり、それは使う人が出てから。
* **統計の on/off をフィールドにする。** 上の 3 節。測れる費用が無いので足さない。
* **`limit` と `mem` を別の example にする。** 上の 4 節。
* **`more_to_run` を外す。** `task_pending()` の走査が怖かったが、3000 台でも差が見えない（3 節）。
  bool で入れ、数で言えない理由を rustdoc に書いた。

## 6. 確かめたこと

| 確認 | 結果 |
|---|---|
| `cargo test --workspace` | **150 passed / 3 ignored**（着手前 140/3。新しい 9 本と、`last_frame` の rustdoc の例 1 本） |
| `cargo clippy --workspace --all-targets` | 警告 2 件、どちらも既存（`src/lib.rs` の `collapsible_if`、`examples/headless.rs` の `type_complexity`）。新しい警告 0 |
| `cargo build --target wasm32-unknown-unknown --lib` | 通る（example は wasm のビルドに入らない） |
| `cargo test --test no_wasm_unsupported` | 2 passed |
| `cargo doc --no-deps` | 警告 0 |
| `cargo package --list` | 111 ファイル、1.4 MiB（圧縮 464.0 KiB）。増えたのは新しい example 2 本とテスト 1 本 |
| 公開 API | 足しただけ（`FrameStats` と `ScriptWorld::last_frame`）。既存の名前・意味は不変。依存の追加なし、`unsafe` 0 |

`tests/frame_stats.rs` の 9 本が見ているもの:

| テスト | 何を固定するか |
|---|---|
| `what_a_working_frame_counts` | 読み 3 回 = 答え 3 件・周回 4 回、命令数は 0 より上で予算より下、持ち越し 0、次のフレームは 0 件（累計ではない） |
| `before_the_first_tick_it_is_all_zeroes` | tick 前は既定値。`Option` を返さない判断の裏 |
| `what_a_full_queue_dropped_is_in_the_frame_that_dropped_it` | 上限 4 に 10 件で 6、次のフレームは 0、VM の累計は 6 のまま |
| `a_frame_that_ran_out_of_time_says_what_it_put_off` | 20 ms の閉包 1 件で `in_tick_answers` 1・`carried_in_tick` 2、`longest_answer_ns` ≧ 20 ms、順番どおり答えられて最後は 0 |
| `with_no_frame_time_the_longest_answer_is_not_measured` | `frame_time = None` で `longest_answer_ns` は `None`、ほかは数える |
| `a_paused_frame_ran_nothing_and_still_has_something_to_run` | `budget = 0` で命令 0・周回 0・`more_to_run` true、戻すと再開 |
| `a_vm_whose_scripts_have_ended_has_nothing_more_to_run` | 一時停止と終了の違いが `more_to_run` に出る |
| `the_second_vm_has_its_own_frame` | 2 本目の VM の数は別（プログラム数も命令数も）、同じ bevy フレーム、片方の一時停止は片方に効かない |
| `loaded_programs_counts_programs_and_not_entities` | 3 エンティティ・2 プログラムで 2 |

## 7. 気づいた点

* **`RubevySet::Tick` を外から挟んで測ると 4 つのシステムを測る。** `docs/host-api.md` の表は
  4 つ入っていることを書いているが、「tick の実時間」を測りたい人がまず書くのは
  `.before(Tick)` / `.after(Tick)` の 2 システムで、それはコマンドの実行とフレーム末尾の書きを含む。
  今は `FrameStats::time_ns` がある。rubevy（文書と計測の作法）。`host-api.md` と
  `docs/verification/scale.md` に 1 段落ずつ書いた。
* **VM に「この run で走ったタスク数」「ready のまま残ったタスク数」を言う口が無い。**
  どちらも `FrameStats` に入れたかった数で、前者はタスクを全部歩けば作れる（が O(N)）、
  後者は `task_pending()` が `bool` に潰している（`ext_task.rs:521`、`vm.task.queues[Q_READY].len()` は
  内部からは 1 命令）。VM（口の不足）。sabiruby の計画に入れるなら、irep の数え口（R2 の気づいた点）と同じ場所。
* **`ScriptWorld::dropped()` は毎回 `host_state` を downcast する。** フレームに 1 回なら費用ではないが、
  HUD が `dropped()` と `last_frame().dropped` の両方を読む形は downcast 2 回になる。
  今のところ実害は無い（downcast は ns）。rubevy（小）。見送り。
* **`examples/how_many_subscribers.rs` の `mem` モードの `list` の行が、上限 1000 と 10000 で
  521 B と 362 B に割れる**（R3 の表）。溜まった配列がヒープのどこに落ちるかで変わっているように見えるが、
  確かめていない。計測器の限界として注記にとどめた。rubevy（計測、小）。
* **`tests/frame_stats.rs::what_a_working_frame_counts` は `rounds == 4` を固定している。**
  読み 3 件のスクリプトで周回が 4 回なのは「1 件答えるごとに 1 周」という今の形の帰結で、
  答えループの形を変える段階（もしあれば）はここが落ちる。落ちたら形が変わった印なので、これは意図した固定。
  rubevy（テストの性質）。
