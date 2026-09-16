# 2026-09-17 フレームの中の居場所と、ホストから VM に触る道

著者判断 3 つ（公開の `SystemSet`、止まった VM はスケジューラの時計を進めない、起動時に VM を触る道）と、
次のゲームが要るもの 1 つ（`Answer` で Data オブジェクトを返す）。ブランチ `scheduling-and-vm-access`、
rubevy main `9104f7c` から。

最初に `cargo update -p sabiruby -p sabiruby-compiler`（`b474f06`）。`1ae258f3` → `9c8ebf2d`、
これが公開された 0.5.0 のタグである。ロックだけを 1 コミットにしたのは、
このあとのコミットが「rubevy で何が変わったか」だけを読める差分であってほしいから。
更新した時点で `cargo test --workspace` は 41 件すべて通っている（10 ファイル）。

## 1. `RubevySet` — ゲームのシステムがフレームのどこに入るか（`c1e2…`）

### 今どうなっていたか

`RubevyPlugin::build` は `Update` に 6 本を 1 本の `.chain()` で並べていた。

```
start_scripts → deliver_answers → answer_components → tick_scripts → drain_commands → apply_component_writes
```

どれも非公開の `fn` なので、ゲームは `.before(tick_scripts)` と書けない。
rubevy_games の `docs/worklog/2026-09-16-showpieces-d2-d3.md` が D3 でこれを測っていて、
**ゲームが答える質問は 2 フレーム、rubevy が自分で答える質問（成分読み）は 1 フレーム**だった。
向こうは `answer_requests` を `PreUpdate` に逃がすと 1 フレームになることまで確かめたうえで、
「rubevy 側に公開の `SystemSet` があるほうが素直」と書いて戻している。

### どこに 2 フレームが生まれるのか（数えた）

質問の往復は 4 か所を通る。フレーム N の `tick_scripts` で `Rubevy.ask` がコマンドを置き、
同じフレーム N の `drain_commands` がそれを `Request` にし、ホストがそれに答えると
タスクが ready になり、次に `tick_scripts` が回ったときにスクリプトが起きる。
つまり**ホストが「次の `tick_scripts` より前」に答えれば 1 フレーム**で、
それは `PreUpdate` でも `PostUpdate` でも、鎖の前でも後ろでも成り立つ。

1 フレームにならない置き場所は 1 か所しかない。**`tick_scripts` と `drain_commands` の間**である。
そこに入ったシステムは、このフレームの質問をまだ見られず（`Request` になるのが数命令あとなので）、
前のフレームの質問には答えるが、そのフレームの `tick_scripts` はもう終わっている。
だから起きるのは N+2 で、**2 フレーム**になる。rubevy_games の `answer_requests` は
13 本の `.chain()` の 2 番目にあり、マルチスレッドの executor がそこに落としていた。

### 再現しようとして、再現しなかった

`tests/scheduling.rs` に一時的なテストを足して、
「`in_set` を付けないシステム」を同じ最小アプリで 5 回測った。**5 回とも 1 フレーム**だった。
`.in_set(RubevySet::Tick)`（集合の中だが鎖には入らない）でも 5 回とも 1 フレームだった。
このアプリは競合するシステムが少ないので、executor が毎回 `ScriptWorld` を要求する
rubevy の鎖を先に流し切ってしまう。つまり**2 フレームは executor のくじ引きの結果**であって、
「順序を指定しないと 2 になる」という規則ではない。
この一時テストは落としてある（残しても「1 と出る」としか言わないため）。

ここが今回いちばん書いておきたいことで、`RubevySet` の値打ちは
「2 を 1 にする」ことではなく、**どこに落ちるかがくじ引きでなくなる**ことにある。
ゲームの作者から見ると、往復の値段がゲームの他のシステムの本数で変わるのをやめさせる、という話になる。
文書にもその形で書いた（「2 フレームは規則ではなかった、運だった」）。

### 何にしたか

```rust
pub enum RubevySet { Deliver, Tick, Answer }
```

`configure_sets(Update, (Deliver, Tick, Answer).chain())` で並べ、中身は

| 集合 | 中身 |
|---|---|
| `Deliver` | `start_scripts`、`deliver_answers`、`release_values` |
| `Tick` | `tick_scripts`、`drain_commands`、`apply_component_writes` |
| `Answer` | rubevy の `answer_components`、**とゲームの答えるシステム** |

3 つ迷ったところがある。

**`release_dropped_values` をシステムに出した。** これまで `tick_scripts` の先頭で呼んでいた。
指示が `Deliver` の中身として名前を挙げているので、`release_values` という 1 行のシステムに出した。
実行される位置は変わっていない（どちらも `tick_scripts` の直前）ので、挙動は同じ。
集合の説明と実際の中身が一致するほうが、あとで読む人が `Deliver` の意味を取り違えない。

**`answer_components` を先頭から末尾に動かした。** 前は `tick_scripts` の**前**にあり、
フレーム N-1 の質問をフレーム N の頭で答えていた。今はフレーム N の質問をフレーム N の末尾で答える。
**起きるフレームは同じ**（どちらも次の `tick_scripts`）なので、成分読みの値段は 1 フレームのまま。
`tests/scheduling.rs` の 2 番目のテストがそれを測っている。
副作用として、`apply_component_writes` が先に回るようになったので、
**同じフレームの中で書いてから読むと、書いた値が読める**。前もそうだった（書きはフレーム末、
読みの答えは次のフレームの頭）が、今は 1 フレームの中で順番が閉じている。

**`Tick` は「入れてはいけない集合」になった。** 2 フレームを生む窓は `tick_scripts` と
`drain_commands` の間にあり、その 2 本はどちらも `Tick` の中にいる。
`Deliver` と `Answer` に置けば窓の外であることが構造から言えるが、`Tick` の中に置くと
（集合の中で順序が決まっていないので）また運になる。文書と rustdoc の両方に
「`Tick` には入れるな」と書いた。

**`Answer` を薦める理由は速さではない。** `Deliver` に置いても 1 フレームである
（3 番目のテストがそれを測っている）。違うのは見えるものの方で、`Answer` は
**質問されたそのフレームのうちに質問が見える**唯一の場所なので、答えがそのフレームの
他の結果に依存してよい（`move_robots` のあとの位置、このフレームのイベント）。
`Deliver` は常に前のフレームの質問を答えることになる。

### 確認

`tests/scheduling.rs` の 3 本。スクリプト自身を測定器にしてある——
`$rubevy[:frame]` を `Rubevy.ask(…).pop` の前後で読み、差を
**答えを待たない質問**（`Rubevy.ask("gap", …)`、pop しない）でホストに送る。
これは rubevy_games が「待たない質問は命令」と呼んでいる形で、
測ること自体がフレームを消費しない。

```
test a_host_answering_in_the_answer_set_costs_one_frame ... ok     [1, 1, 1, 1, 1, 1]
test a_component_read_still_costs_one_frame ... ok                 [1, 1, 1, 1, 1, 1]
test a_host_answering_in_the_deliver_set_sees_the_previous_frame ... ok
```

`examples/sensor.rs` と `examples/async.rs` の `answer_requests` を `.in_set(RubevySet::Answer)` にした。
`sensor.rs` はわざと 2 フレーム遅らせて答える例なので、数字は変わらない（遅らせているのは例の方）。
