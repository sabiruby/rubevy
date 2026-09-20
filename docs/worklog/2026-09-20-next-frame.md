# 2026-09-20 スクリプトが「次の 1 フレームだけ待つ」と言える口（R11）

計画 `docs/plans/generalize-plan.md` の段階 **R11**。出どころは R9 の気づき
（`docs/worklog/2026-09-20-camera-layer.md` と計画書 7 章の「09-20 R9」の行）— **スクリプトから
「次の 1 フレームだけ待つ」と言えない**。本体が原則から決めた: フレームはゲームの基本の単位だから、
外の利用者が自作せずに言えるべきである。

到達点は `Rubevy.next_frame`（呼んだタスクが次のフレームの tick の頭で起きる）と、その上の
`Rubevy.each_frame { |dt| … }`。R10 が送った宿題 3 件は別コミット
（`docs/worklog/2026-09-20-r10-followups.md`）。

## 何が足りなかったのか

`sleep` は**時間**で、時計は 4 ms 単位である。プラグインは貯まったフレーム時間を tick 単位に
切り下げて mruby-task の時計に渡している（`src/lib.rs` の `tick_scripts` 冒頭、
`Vm::task_tick_unit_ms()` を毎回訊いて `task_advance_ticks`）。だから `sleep 0` は
「次のフレーム」ではなく「**時計が次に進むまで**」で、60 Hz なら 1 フレームに 4 tick 進むので
結果として次のフレームに起きるが、それは約束ではなく癖である。250 Hz を超えると時計が動かない
フレームが出る。

この穴は 2 か所で実害になっていた。

1. **`Rubevy.rejected_writes`**（R9 の前）が「直前のフレーム」ではなく「**直前に書きのあった
   フレーム**」という形にせざるを得なかった。毎フレーム空にする実装はテストで破綻していて、
   その原因がまさに「`sleep 0` は次のフレームとは限らない」だった
   （`docs/worklog/2026-09-20-rejected-writes.md`）。
2. **箱庭（rubevy_games の garden）が「1 パス = 1 フレーム」を自作していた。**
   `garden/ruby/world_prelude.rb` の `each_frame` は `Rubevy.ask("frame").pop` で待ち、
   `garden/src/main.rs` の `run_world`（`:4347` あたり）がそれに**毎フレーム 1 回だけ**答える
   システムを持っている。ゲーム側のコメントがそう言っている —
   「once is what makes one pass of `each_frame` one frame」。外の利用者は同じものを書くことになる。

## 実装の道を 3 つ比べた

計画が挙げた (a) 予約 kind、(b) VM のタスクの口（`Task#suspend` / `resume`）、(c) 1 本の
`Task::Queue` を毎フレーム push、を比べた。**(a) を選んだ。**

### (a) 予約 kind（採用）

`Rubevy.ask` は既に「タスクを queue に park し、ホストが後で push して起こす」道具で、
rubevy の予約 kind（`RESERVED_KINDS`）はその答えをホスト自身が tick の中で返す仕組みである。
`next_frame` はその**置き場所を 1 つずらすだけ**で書ける: 予約 kind に `"frame.next"` を足し、
答える代わりに `ScriptWorld::next_frame_waiters` に移し、**次の tick の頭**で全員に答える。

* **費用**（待っているタスク 1 本・1 フレームあたり）: 問い 1 回ぶんの命令（測って 62 命令、下記）と、
  ホスト側の `Vec` の要素 1 つ、`task_queue_push` 1 回。既存の同期読みと同じ道具で、新しい機構は無い。
* **順序**: `Vec` の順、すなわち**聞いた順**。フレーム時間で切られたら残りは先頭に残るので、
  次のフレームでいちばん先に起きる（飢えない）。
* **2 本目の VM**: `ScriptWorld<M>` のフィールドなので VM ごとに独立。`frame_time` と同じ扱い。
* **一時停止**: `budget == 0` のフレームでは起こさない（下記）。
* **フレーム番号**: 答えは `Value::Int(frame_no)` で、`$rubevy[:frame]` と**同じ数・同じ型**
  （`set_frame_state` が同じ `frame_no` を入れている）。

### (b) VM のタスクの口（捨てた）

sabiruby 0.5.2 の公開の口を確かめた。Ruby 側には `Task#suspend` / `#resume` があり
（`src/builtins/ext_task.rs:1030-1047`）、ホスト側には `Vm::funcall` があるので、
**VM の crate を触らずに**書けはする。捨てた理由は 3 つ。

1. **誰を起こすのかをホストが知る道が無い。** `suspend` したタスクの一覧は VM の中で、
   ホストから引ける公開の口は無い。だからスクリプトが「待ちます」と申告する道が要る —
   それは結局 `ask`（(a)）である。
2. **起こし方が send になる。** `funcall(task, :resume, …)` は tick の途中で Ruby を走らせる
   ことで、rubevy が答えに queue を使っているのはまさにそれを避けるためである
   （`ScriptWorld::answer` は `task_queue_push` だけで、VM の実行は次の `task_run_limits` に任せる）。
3. **答えを渡せない。** `resume` は値を運ばない。フレーム番号を返すには結局どこかに値を置くことになる。

`suspend`/`resume` は「スクリプトが自分で自分を止める」ための口で、ホストのスケジューリングの口ではない。
VM に口を足す必要は無いと判断した（計画の「VM に口が要ると分かったら止まって報告」には当たらない）。

### (c) 1 本の `Task::Queue`（捨てた）

毎フレーム 1 件 push しても、**pop できるのは 1 人**である。N 本の待ち手には N 件要り、
「誰が取るか」は VM のキューの起こし順（待ち行列の先頭）になるので、rubevy からは順序を約束できない。
待ち手ごとにキューを持つなら、それは `Rubevy.ask` が既にやっていることそのもの（＝(a)）。
この道には (a) に無い利点が 1 つも無かった。

## 置き場所を 2 回間違えかけた

**1 回目: 答える場所。** 素直に書くと `answer_reflect_requests` の `match` に `"frame.next"` の
腕を作り、そこで答えないで持ち越す形になる。ところが `answer_reflect_requests` は
`match` に入る前に「全部に nil を返す」経路を持っている（`AppTypeRegistry` が無い app）。そこに落ちると
`next_frame` が**聞いたフレームで nil を返して起きてしまう** — 唯一してはいけないことである。
なので `match` に入る前で拾い、registry の無い経路でも同じ分岐を通すようにした
（`src/lib.rs`、`answer_reflect_requests` の 2 か所のコメント）。

**2 回目: 遅れて届いた問い。** フレームが予算や時間で切れると、その周回の `ask` は VM の
コマンドキューに残り、フレーム末尾の `drain_commands` が `reflect_requests` に仕分ける。
`"frame.next"` をそのまま `RESERVED_KINDS` の仕分けに任せると、次の tick がそれを取り出して
「待ち行列に移す」ので、**待ちが 1 フレーム余計に**なる。`drain_commands` で先に
`next_frame_waiters` へ入れることで、聞いたフレームの次に起きる（`src/lib.rs` の
`drain_commands` のコメント）。

## 起こす場所は tick の頭

`wake_next_frame` は `tick_scripts` の中、`set_frame_state`（`$rubevy` の更新）の後・
**最初の VM の実行の前**に走る。だから起きたタスクは「そのフレームの `$rubevy`」を読み、
書いたものは「そのフレームの書き」に入る。1 フレーム遅れて読める `rejected_writes` が
`next_frame` の次のフレームで読めるのはこの順のおかげで、テストにした
（`tests/next_frame.rs::a_write_is_readable_on_the_next_frame`）。

### フレーム時間で切られたとき

答えと同じ扱いにした。`AnswerClock` を共有し、1 人起こすごとに締切を見て、切られたら残りは
`next_frame_waiters` に**そのまま**戻す（何も前に足されていないので先頭のまま）。
次のフレームの頭でいちばん先に起きるので、待ち行列は回る。

決定的なテストの作り方に少し困った。起こす処理は 1 件が `task_queue_push` 1 回なので、
ふつうの `frame_time` では何千本並べても切れない。`frame_time = 1 ns` にすると
**1 フレームに 1 人しか起きない**状態が機械に依らず作れる（最初の 1 人を起こした時点で
締切を過ぎている）。ただしこの設定では VM 自体も走らない（答えの前にループの頭の時間判定で
break する）ので、起きた順は `FrameStats::carried_reflect` の減り方で確かめ、
そのあと `frame_time` を既定に戻してから各タスクに「自分が起きたフレーム番号」を報告させた
（`a_wake_the_frame_time_cut_short_goes_on_at_the_head_of_the_next_frame`）。
3 本が 3 つの異なるフレームに、聞いた順に起きる。

### 一時停止（`budget == 0`）

**起こさない。** 理由は時計と同じで、`tick_scripts` は `budget > 0` のときしか
`task_advance_ticks` を呼ばない（「一時停止はスクリプトが生きなかった時間」）。
起こしてしまうと、答え（フレーム番号）だけがキューに入り、スクリプトが実際にその行を走らせるのは
ずっと後のフレームなので、**自分が見ていないフレームの番号**を持つことになる。
テストは `a_paused_frame_is_not_the_next_frame`: 5 フレーム止めて、起きたのは予算を戻した
フレームで、番号もその数。

### `FrameStats` の数え方

* 起こしたぶんは `reflect_answers` に数える（rubevy 自身の kind の答えを tick の中で作ったので、
  そのとおりのもの）。
* `carried_reflect` には、**フレーム時間で起こしきれなかったぶんだけ**を足す。
  tick の頭で起こした直後の `next_frame_waiters.len()` がちょうどそれである
  （そのフレームの中で新しく待ちに入ったタスクは後から積まれる）。
  待っているタスクを全部数えてしまうと、「毎フレーム全員が待つ」ふつうのゲームで
  `carried_reflect` が常に台数ぶんになり、「tick が追いついていない」という本来の意味が消える。

## Ruby 側

```ruby
def self.next_frame
  ask("frame.next").pop
end

def self.each_frame
  loop do
    n = next_frame
    yield $rubevy[:delta], n
  end
end
```

`each_frame` を足すかどうかは迷ったが、足した。garden が自作していたのは `next_frame` ではなく
**`each_frame`** の形で、ゲームが書くのはこちらだからである。ブロックには delta とフレーム番号を
渡す（Ruby のブロックは余った引数を捨てるので `{ |dt| }` と書ける）。`break` で抜けられる。
`prelude.rb` は `tools/compile_scripts.sh` と同じ mrbc（Docker `kishima/mruby:4.1.0-rc`）で
`prelude.mrb` に焼き直した。

## 測った

機械: WSL2、2026-09-20。命令数は VM の数なので機械に依らない。

```
cargo test --release --test next_frame -- --ignored --nocapture
```

| 何 | 値 |
|---|---|
| `Rubevy.next_frame` 1 回 | **62 命令**（10 回 639、110 回 6839 の差から） |
| `sleep 0` 1 回（対照） | **14 命令** |
| 100 本が毎フレーム起きる | tick の中央値 **346.6 µs**、5,600 命令（1 本 56 命令） |
| 1000 本が毎フレーム起きる | tick の中央値 **2.50 ms**、56,000 命令 |

1 本 1 フレームあたり 2.5〜3.5 µs で、これはほとんど rubevy の側ではない — `frame_time` の
rustdoc が既に書いている「3000 本並んでいると、何も問いを出さなくても tick が 2.9 ms かかる」
（VM のスケジューラが push と wake のたびに待ち行列を歩く）と同じものである。
**時間の 2 つの行は静かな機械での値ではない**: 測った直後の `vmstat 1 5` で隣の担当（rubevy_games の
S7）の `docker run … garden` が走っており、idle は 20% だった。2 回の走行で 1000 本が
2.37 ms と 2.50 ms（5% 差）なので桁は信用してよいが、µs の桁は取り直しが要る。
命令数の 4 行は決定的な値で、そのまま使える。

この数から言えること: **1000 本のスクリプトが `each_frame` で毎フレーム起きるのは、既定の
予算 200,000 命令の 28% と、既定の `frame_time` 8 ms の 31% を、何もしないうちに使う。**
`host-api.md` の「Waiting for the next frame」にそう書いた。

## カメラ層の `follow` は載せ替えなかった

`Rubevy::Camera#follow` は子タスクで `sleep every`（既定 0）を回している。`next_frame` に
載せ替えるかどうかを比べて、**載せ替えない**ことにした。理由は 2 つで、どちらも測った数に基づく。

1. **`sleep 0` の方が安い。** 14 命令対 62 命令で、しかも `sleep 0` は VM の中で完結する
   （ホストの問い・答えが要らない）。follow はカメラが追っている間ずっと毎フレーム走る。
2. **`every` の意味が 1 つで済む。** 0 のときだけフレーム、それ以外は秒、とすると引数が
   2 つのものになる。

ゲームが**約束**の方を要るときは自分のタスクに `Rubevy.each_frame` を書けばよく、それを
`camera.rb` の `follow` の rustdoc に 1 段落書いた（`.mrb` はコメントだけの変更なので
md5 が変わらないことを確かめた）。

## `rejected_writes` は変えていない

R9 の「直前に書きのあったフレーム」はそのまま。`next_frame` があっても `sleep` で待つ
スクリプトはいるので、毎フレーム空にすると相変わらずそちらが読めなくなる。
`host-api.md` と `src/prelude.rb` の該当箇所の**理由の文**だけを直した
（「起きる口が無いから」→「`next_frame` なら次のフレームに読めるが、`sleep` で待つ方は 2〜3
フレーム後になりうるから」）。

## 捨てた案・書かなかったもの

* **`Rubevy.next_frame(n)`（n フレーム待つ）**: 欲しい利用者が出てから。`n.times { next_frame }`
  で書けるし、ホスト側は「何フレーム目か」を待ち行列に持つことになって順序の話が増える。
* **`FrameStats` に「待っているタスクの数」を足す**: 足さなかった。待っているのは backlog では
  ないし、`carried_reflect` に混ぜない判断（上）と同じ理由で、HUD が見て意味のある数ではない。
  必要になったら `ScriptWorld` に読み出しを足す方が正しい。
* **起こす場所を `answer_reflect_requests` の中（周回の答えと同じ場所）にする**: 検討して、
  tick の頭に出した。中にあると起こすのが「最初の VM の実行のあと」になり、待っていたタスクは
  そのフレームの 1 周目に走れない。
* **`$rubevy` に「このフレームで起きた人」を載せる**: 全スクリプトが見る値に待ち行列の話を
  持ち込むことになるので考えただけ。`rejected_writes` のときと同じ判断。

## 確認

* `cargo test --workspace`: 185 passed / 6 ignored（着手前 178 / 4。足したのはテスト 7 本と
  計測器 2 本）。
* `cargo clippy --workspace --all-targets`: 新しい警告 0。
* `cargo build --target wasm32-unknown-unknown --lib`、`cargo test --test no_wasm_unsupported`、
  `cargo doc --no-deps` 警告 0。
* `.mrb`: `src/prelude.mrb` を焼き直し（Docker の mrbc 4.1.0-rc）。`src/layers/camera.mrb` は
  コメントだけの変更なので md5 が変わらないことを確かめて、そのまま。

## 気づいた点

1. **待ちタスク 1 本あたり 2.5 µs は rubevy ではなく VM の側**（`task_queue_push` / wake が
   待ち行列を歩く）。`frame_time` の rustdoc が 3000 本で 2.9 ms と書いているのと同じ現象で、
   `each_frame` が広まるとこれが「1000 体のゲームが載らない」の主因になる。属する先: **VM
   （sabiruby）**。sabiruby 側の計画に「待ち行列を O(1) で起こす」を置く価値がある。
2. **`frame_time` を極端に小さくすると VM が 1 命令も走らない。** ループの頭で
   `time_left == Some(0)` を見て break するので、答えを作る側の「遅れていても 1 件は答える」
   規則が効かない（そこまで到達しない）。テストではこれを道具として使ったが、ゲームが
   `frame_time = 1µs` と書くとスクリプトが完全に止まる。属する先: **rubevy（口の設計）**。
   `frame_time` に下限を置くか、rustdoc に 1 文書くか。今回は触っていない。
3. **`garden` の `Rubevy.ask("frame")` は `next_frame` に置き換えられる。** ゲーム側の
   `run_world` の答え手（`main.rs:4347` 付近）と `world_prelude.rb` の `each_frame` が
   まるごと消える。属する先: **rubevy_games（S7 / 共有 crate の計画）**。ただし garden の
   `each_frame` は `being` ごとの呼び出しも兼ねているので、そのまま 1 対 1 ではない。
4. **`Rubevy.each_frame` を 2 つのタスクが回すと、答えは 2 つとも同じフレーム番号になる**
   （テスト `two_tasks_wait_for_the_same_frame`）。当たり前だが、`next_frame` の戻り値を
   「自分だけの通し番号」と勘違いする利用者はいそうなので、`prelude.rb` の rustdoc は
   「`$rubevy[:frame]` が持っているのと同じ Integer」と書いてある。
