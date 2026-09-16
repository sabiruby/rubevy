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

## 2. 止まった VM はスケジューラの時計を進めない

### 今どうなっていたか

`tick_scripts` は毎フレーム `task_advance_ticks` を呼ぶ。`budget` が 0 でも呼ぶ。
sabiruby の `task_run_limited` はループの頭で `if budget.is_some_and(|b| spent >= b) { return }`
を見る（`src/vm.rs:773`）ので、0 なら 1 命令も走らない——**走らないのに時計だけ進む**。
rubevy_games の `P`（`ScriptWorld::budget = 0`）はこの形で止めていて、
向こうの worklog が「再開した瞬間に寝ていたタスクが全部起きる。直すなら rubevy の `tick_scripts`」
と書いて残していた。

### 規則をどちらにするか

指示は「`budget` が 0 のときは進めない」か「`ScriptWorld::pause(bool)` を足す」の二択だった。
**前者にした。**

* `budget` は `pub` のフィールドで、止めるゲームは**もう `budget = 0` と書いている**。
  この規則なら向こうは 1 行も変えずに直る。
* `pause` フラグを足すと「同じことを言う道が 2 本」になり、食い違える
  （`paused == false` なのに `budget == 0`、その逆）。どちらが本当かを決める規則が要り、
  それは結局「budget が 0 なら走らない」という今の事実の言い直しになる。
* 0 という値に他の用途がない。0 は「このフレームはスクリプトにとって起こらなかった」以外の
  意味を持ちようがない（1 命令も走らないので）。

### どこまで止めるか

`tick_scripts` ごと `return` してしまう案も考えたが、**やめた**。止めたのは
ティックの加算と `task_advance_ticks` だけで、`set_frame_state`（`$rubevy` の更新）、
`task_run_limits`（0 なので即戻る）、出力の掃き出し、終わったタスクの回収はそのまま残した。
理由は rubevy_games の D2 で入った**窓の中の VM インスペクタ**で、あれは止めている間に
VM を覗くためのものである。`$rubevy` が凍ると、パネルが見せるフレーム数が止まった瞬間の値のまま
固まる。走っていないのはスクリプトであって、VM を見る側ではない。
`tick_remainder`（1 ティックに満たない端数）も止めている間は溜めない。溜めて再開時に吐くと、
結局まとめて起こすことになって同じ穴になる。

### 確認

`tests/pause.rs`。`sleep 0.1` を挟んで「始めた」「起きた」を**待たない質問**でゲームに送り、
ゲームが受け取ったフレームと `Time::elapsed_secs` を書き留める。
4 フレーム走らせて `sleep` に入らせ、`budget = 0` で 100 フレーム（1 フレーム 2 ms なので実時間 0.2 秒、
sleep の 2 倍）、そこで `budget` を戻す。

直す前の挙動も測ってある。`if world.budget > 0` を `if true` に書き換えて同じテストを回すと:

```
woke on frame 104, one after the resume at 104 — the clock ran on while it was paused
```

**再開したフレームそのもので起きている**。直したあとは 2 本とも通る。

```
test without_a_pause_it_sleeps_the_time_it_asked_for ... ok
test a_pause_does_not_spend_a_sleep ... ok
```

対照のテスト（止めない場合に 0.1 秒くらい寝る）を足したのは、
「再開後に 0.05 秒以上たってから起きた」という主張が、測っている量として正しいことを言うため。

## 3. 起動時にホストが VM に触る道

### 調べたら、もうあった

次のゲームは `sabiruby_serde::install_json(&mut vm)` と、自前の `define_fn` を
スクリプトが走り出す前に入れたい。指示は「`with_vm` / `vm_mut` があるか調べ、
無ければ最小の入り口を足す」だった。

**足すものは無かった。** `ScriptWorld` の `vm` は最初から `pub vm: Vm` である
（`src/lib.rs`、`ScriptWorld` の定義の 1 行目）。そして `RubevyPlugin::build` が
`insert_resource(world)` を呼ぶのは `App` を組んでいる最中なので、
リソースは `Startup` より前に存在する。最初のスクリプトが走るのは最初の `Update` の
`start_scripts` → `tick_scripts` なので、**`Startup` のシステムがちょうど間に入る**。

```rust
fn install_host_api(mut world: ResMut<ScriptWorld>) {
    let vm = &mut world.vm;
    sabiruby_serde::install_json(vm);
    let object = vm.core.object;
    vm.define_fn(object, "arena_size", |_vm: &mut Vm| -> f64 { 240.0 });
}
```

`vm_mut()` を足すことも考えたが、**やめた**。フィールドが `pub` なのだから
`vm_mut()` は同じものを指す 2 本目の綴りで、増えるのは名前だけである。
指示が `vm_mut` に期待していたのは入り口そのものではなく、
**そこに書かれるはずだった警告**（タスクを走らせるな、参照を持ち越すな）だと読んだので、
警告はフィールドの rustdoc に書いた。`pub` なフィールドの doc は rustdoc に出るので、
入り口を探した人が必ず通る場所ではある。

### 書いた警告

3 つ。どれも「これは自分の VM ではなく、スケジューラが回している VM だ」の言い換えである。

* **タスクを走らせない**（`task_run_limits` / `task_run_once` / `Task.run`）。
  フレームを与えるのは `tick_scripts` の仕事で、システムの中でもう一度回すと
  誰も設定していない予算を使い、他人のフレームの途中でスクリプトを再開させることになる。
  ただし `Startup` での `Vm::load_and_run` は別で、これはプログラムを走らせているのであって
  スケジューラを回しているのではない。
* **`Value` / `ObjId` を持ち越さない**（`gc_register` するなら別）。
  rubevy 自身が持ち越しているのは `Request` と `ScriptTask` の 2 つだけで、どちらも登録済み。
* **ネイティブから Bevy の `World` に触らない**。ネイティブが受け取るのは `&mut Vm` だけである。

### 確認

`tests/vm_setup.rs` の 2 本。`sabiruby-serde` と sabiruby の `macros` は
**dev-dependency にだけ**足した（`Cargo.toml` にその旨を書いた）。
rubevy 本体が JSON を知らないことは、テストがゲーム側で入れていること自体が示している。

```
test a_game_installs_json_and_a_native_of_its_own_at_startup ... ok
test the_vm_is_reachable_while_the_app_is_still_being_built ... ok
```

1 本目はスクリプトの**1 行目**で `JSON.generate` を呼ぶ。`defined?` で守る必要がない、
というのが `Startup` で入れることの意味なので、そこを測っている。
`JSON.parse(...).inspect` の期待値を `{"x"=>1.5, ...}` と書いて 1 回落ちた。
SabiRuby の `inspect` は `=>` の前後に空白を入れる（`{"x" => 1.5}`）。CRuby 3.4 の書式である。
2 本目は「`run()` の前にリソースを触る」道で、システムを 1 本も書かずに同じことができることの確認。

## 4. `Answer` にホストの Data オブジェクトを載せる

### 調べたら、これももうできた

次のゲームは `Rubevy.ask("genome")` の答えを `#[derive(RubyClass)]` のオブジェクトで返したい。
指示は「`answer_value(request, |vm| …)` で既にできるなら文書とテスト、
足りないもの（`HostStore` がプラグインの持つ VM 経由で設定できない、など）があれば最小限を足す」。

`answer_value` の閉包は `&mut Vm` を受け取る。`RubyClass::into_handle` は
`register_class` → `install_host_store` → `data_new` を全部その `&mut Vm` の上でやる。
つまり**閉包の中で `Genome { … }.into_ruby(vm)` と書けば終わり**で、足すものは無い。

心配していたのは衝突である。rubevy は VM のホスト機構を 2 つ使っている——
`Vm::set_host_state`（**1 つしか持てない**。rubevy はコマンドキューを入れている）と
`Vm::set_on_free`（**1 本しか持てない**。rubevy は `Rubevy::Entity` を数えている）。
`RubyClass` がこのどちらかを要求するなら、ゲームは自分の型を入れられない。

要求しない。`HostStore` は `TypeId` で引く**型ごとの**置き場で、`Vm::install_host_store` が
VM の中に作る（`src/host_store.rs` の冒頭がまさにこの設計判断を書いている——
「`set_host_state` は 1 つしか持てないので、複数種類のホストオブジェクトが分け合わねばならない」）。
解放も VM 自身がやり、`set_on_free` はそのあとに呼ばれる。**別の場所**である。

### 確認

`tests/host_data.rs` の 2 本。1 回で通った。

```
test a_game_answers_with_an_object_of_its_own ... ok
test the_value_is_dropped_when_the_object_is_collected ... ok
```

衝突しないことを主張で終わらせないために、1 本目で `world.freed_entities()` が 0 のままであること
（rubevy のフックが `Genome` に対して鳴っていない）と、`Genome::store(vm).len() == 1` を両方見ている。
2 本目はスクリプトが終わったあとに `gc_collect` して `len() == 0` を見る。
ここが 0 にならなければ、ゲームが答えを返すたびに Rust の値が漏れることになるので、
「できる」と書く前に測っておきたかったところ。

`#[ruby_methods]` の `register` は `Startup` で 1 回呼ぶ。クラスと置き場は最初の `into_ruby` で
勝手にできるので、`register` が足すのは**メソッドだけ**である。これは
sabiruby の `macros/src/expand.rs:184` を読んで確かめた（`register_class` → `tag` → メソッド群）。
