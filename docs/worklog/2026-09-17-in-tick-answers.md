# 2026-09-17 ゲームが tick の中で答える口（S4: `answer_in_tick`）

`docs/plans/sync-access-plan.md` の **S4**（§3.6）の記録。ブランチ `in-tick-answers`、main `3540308` から。
S1〜S3 の記録は `docs/worklog/2026-09-17-sync-reads.md`（答えループそのもの）と rubevy_games 側にある。
ここに書くのは「S1 が作ったループに、ゲームの閉包をどう挿したか」「借用をどう解いたか」「捨てた案」「何を測っていないか」。

**絶対の制約**: `unsafe` を書かない。World への参照をネイティブにも `HostState` にも渡さない。
sabiruby と `Cargo.lock` は触らない。今回もそのとおりで、`grep -rn unsafe src/` は着手前も後も
コードは 0 件（当たるのは `tick_scripts` の rustdoc の「**Why there is no `unsafe` in it.**」の 1 行だけ）。

---

## 0. 着手前の状態

`cargo test` は 15 ファイル 56 件 ＋ doctest 10 件が通り、`tests/read_cost.rs` の 2 本は `#[ignore]`。
`cargo clippy --all-targets` の警告は 5 行（`collapsible_if` が `drain_commands` に 1 つ、
`type_complexity` が `examples/headless.rs:47` に 1 つ、あとは集計行）。
`cargo build --examples` は通る。これを基準にして §6 で取り直した。

実装の実物は先に読んだ。`tick_scripts`（`src/lib.rs`、S1 が排他システムにしたもの）の
`resource_scope` の中の答えループ、`answer_reflect_requests`（`&World` と `&mut ScriptWorld` を
別々の引数で持つ関数）、`take_reflect_asks`（VM のコマンドキューから予約 4 種の `Ask` だけを抜く）、
`push_answer`（`task_queue_push` ＋ `gc_unregister`）、`RESERVED_KINDS`、`drain_commands` の仕分け。
計画書 §3.6 が「`take_reflect_asks` と同じ要領で」と言っているのは、この 5 つがすでに
**ゲームの閉包を挿す場所を空けてある**という意味だった。実際、足したのは新しい仕組みではなく、
同じ形の 2 本目の道である。

---

## 1. 作ったもの

### 1.1 公開の口

```rust
pub type InTickAnswerer = Box<dyn Fn(&World, &Request) -> Answer + Send + Sync + 'static>;

impl<M: 'static> ScriptWorld<M> {
    pub fn answer_in_tick(&mut self, kind: impl Into<String>, f: InTickAnswerer);
}
```

計画書の署名そのまま。閉包は `&World` を**引数で**受けるだけなので unsafe は要らない。
`Send + Sync` は `ScriptWorld` が Bevy のリソースだから（`Vm` が `Send + Sync` を保っているのと同じ理由）。

`ScriptWorld` に足したフィールドは 2 つだけ:

* `in_tick_answerers: HashMap<String, InTickAnswerer>` — 登録そのもの。
* `in_tick_requests: Vec<Request>` — `reflect_requests` と同じ役の持ち越し。予算切れ・時間切れで
  tick が終わったとき、コマンドキューに残った分を `drain_commands` がここに仕分け、次の tick の
  答えループが最初に拾う。

### 1.2 答えループ

`tick_scripts` の中の 1 行が 2 行になっただけ。

```rust
let answered = answer_reflect_requests(&*world, scripts)
    + answer_in_tick_requests(&*world, scripts);
if answered == 0 { break; }
```

計画書 §3.6 の「答えた数を reflect の分と合わせて 0 なら終える」はこの `+` のこと。
**周回の上限は今回も置いていない**（S1 の理屈がそのまま効く: 質問を 1 つ出すには Ruby 側で
`Rubevy.ask` と `pop` が走って命令数を食うので、フレームの予算が周回を有限にする）。
新しい定数は 1 つも足していない。

順序は「reflect が先、ゲームの閉包が後、それぞれの中では積まれた順」。
2 つの集合をまたぐ順（同じ周に `e[:Hp]` と `garden.nearest` が積まれたとき）は、
キューの順ではなく「rubevy が先」になる。計画書は各集合の中の順しか決めていないので、
それに合わせて実装も docs もそう書いた（rustdoc: "in the order they were asked in"、
つまり 1 つの集合の中の話だと分かる書き方にした）。

### 1.3 `answer_in_tick_requests`

`answer_reflect_requests` の隣に置いた同じ形の関数。違いは答えの出どころが反射ではなく閉包なこと。

```rust
let answerers = std::mem::take(&mut scripts.in_tick_answerers);
let mut asked = std::mem::take(&mut scripts.in_tick_requests);
asked.append(&mut take_asks(&mut scripts.vm, |kind| answerers.contains_key(kind)));
for request in asked {
    let answer = answerers[&request.kind](world, &request);   // 実際は match で None も畳む
    scripts.answer(&request, answer);                          // ここで `&mut Vm` が要る
}
scripts.in_tick_answerers = answerers;
```

**なぜ `mem::take` するのか**が、この段階でいちばん時間を使ったところなので書いておく。
閉包は `&ScriptWorld` の中に住んでいて、答えを返すには `scripts.answer(&request, answer)`＝
`&mut ScriptWorld` が要る。`answerers.get(&kind)` の借りが生きている間はそれができない。
借用検査に嘘をつかずに解くには、閉包を**一時的に `ScriptWorld` の外に出す**のがいちばん素直だった。

捨てた案は 2 つ。

1. **`Arc<dyn Fn ...>` にして clone する。** 1 件ごとに参照カウントを 1 回触るだけで済むし、
   `mem::take` の「戻し忘れ」の心配もない。捨てた理由は、公開型が計画どおり `Box` だからで、
   内部だけ `Arc` にすると登録のたびに `Box` → `Arc` の詰め替えが起きて、型が二重になる。
   `mem::take` は HashMap の move が 2 回（＝ポインタの付け替え）で、1 周につき 2 回しか起きない。
2. **答えを先に全部集めてから `answer` する**（`Vec<(Request, Answer)>` を作る）。借りは切れるが、
   質問の数だけ `Answer` を溜める分だけ余計に確保する。閉包を出し入れする方が安い。

戻すのを忘れると登録が消えるので、「なぜ安全に戻せるのか」は rustdoc に書いた:
閉包が受け取るのは `&World` と `&Request` だけで、その World には `ScriptWorld<M>` が
**入っていない**（`resource_scope` が抜いている）ので、閉包の中から登録を足す道が無い。
だから「出したものがそのまま戻る」と言い切れる。

`None` の腕（登録が無い kind がこの道に来る）は到達しない — この道に仕分けられるのは登録済みの
kind だけで、登録を取り消す口は無いから。それでも `Answer::Nil` で答えるようにしたのは、
万一そこに落ちたときに**タスクが永久に park する**方が悪いから。コメントでそう書いた。

### 1.4 `take_reflect_asks` を 2 人で使う

`take_reflect_asks` は「コマンドキューを頭から見て、`Ask` のうち予約 4 種だけを `remove` して
Request にし、他は置いていく」20 行だった。ゲームの kind にも同じものが要る。
述語だけを引数にした `take_asks(vm, wanted: impl Fn(&str) -> bool)` に括り出して、
`take_reflect_asks` はその 1 行の呼び出しにした。複製を 2 本持つより読みやすく、
「キューに残す方」の規則（`Rubevy.spawn` などは `drain_commands` のもの）が 1 か所で済む。

### 1.5 仕分け（`drain_commands`）

```rust
if RESERVED_KINDS.contains(&request.kind.as_str()) { ... reflect_requests }
else if world.in_tick_answerers.contains_key(&request.kind) { ... in_tick_requests }
else { ... requests }
```

これで「登録済みの kind は `take_requests` に出ない」が、予約 4 種とまったく同じ仕組みで成り立つ。
`tests/in_tick.rs` の `a_registered_kind_never_reaches_take_requests` がそれを固定していて、
`tests/components.rs` の `rubevys_own_questions_never_reach_the_game`（予約 4 種の同じ主張）と
並んで読めるようにした。

### 1.6 後勝ちと `warn!`

`insert` が `Some` を返したら `warn!("rubevy: {kind} had an in-tick answerer already; ...")`。
テストは 1 本目（`a_kind_the_game_answers_in_the_tick_costs_no_frame`）に畳んだ:
同じ kind を 2 回登録して、返ってくるのが 2 本目の値であることを 6 周とも見ている。
テストの本数を計画どおり 6 本に収めるための畳み方で、主張は減っていない。

---

## 2. 決めた形と、その理由

### 2.1 答えは `Answer` だけ（`answer_value` 相当は作らない）

計画書 §3.6 の指示どおり。理由は計画書が書いているとおり（`&mut Vm` も渡すと
`Fn(&World, &mut Vm, &Request)` になり、VM を貸している最中に閉包が VM を触る形になる）だが、
実装してみて**もう一つの帰結**が見えた: 閉包は `Arg::Value`（Hash や Array の引数）を
「そこにある」とは言えるが**中を読めない**。中を読むには `Vm` が要るからである。
これは docs と rustdoc に明記し、テスト
（`the_closure_sees_an_arg_value_and_it_is_released_on_the_next_frame`）で固定した。
「中を見たいなら `RubevySet::Answer` のシステムで答える」が正しい案内になる。

### 2.2 予約 4 種は上書きできない

`take_reflect_asks` が先にキューから抜くので、`answer_in_tick("component.get", ...)` を
登録しても閉包は呼ばれない（静かに無視される）。計画書はここに触れていない。
**`warn!` は足さなかった**（計画に無い挙動を増やさない）が、rustdoc と host-api.md の
「The rules」に 1 行ずつ書いた。気になるなら 1 行で足せる、と報告に挙げる。

### 2.3 例の閉包は反射ではなく型で読む

計画書は example の閉包を「`world.iter_entities()` + `ReflectComponent` で距離を測る」と書いている。
実装は `world.iter_entities()` のまま、コンポーネントは `entity.get::<Transform>()` の**型付きの読み**にした。
理由: この閉包は**ゲームが書くコード**で、ゲームは自分の型を知っている。反射は「名前しか知らない側」
（＝ rubevy）の道具で、ゲームに反射を書かせると「閉包は `&World` を渡されるただの Bevy のコード」という
いちばん大事なことが霞む。走査の形（全エンティティを舐める）は計画どおり。
計画との差なので報告に挙げる。

---

## 3. テスト（`tests/in_tick.rs`、新規 6 本）

| テスト | 何を固定するか |
|---|---|
| `a_kind_the_game_answers_in_the_tick_costs_no_frame` | 登録した kind の往復が `$rubevy[:frame]` で 0 差、6 周とも。ついでに後勝ち（2 本目の閉包の値が返る） |
| `a_kind_nobody_registered_still_costs_one_frame` | 未登録の kind は今までどおり 1 フレーム。1 つ登録しても他の kind は変わらない |
| `a_registered_kind_never_reaches_take_requests` | ゲームの `take_requests` に出るのは自分の kind だけ（予約 4 種と同じ扱い） |
| `the_closure_sees_the_world_this_frame_left_it_in` | `.before(RubevySet::Tick)` のシステムが書いた値を閉包が読む＝見ているのは「このフレームの Tick 時点」 |
| `a_named_vm_registers_its_own` | `ScriptWorld<Mods>` にも登録でき、そのVMの tick で答える |
| `the_closure_sees_an_arg_value_and_it_is_released_on_the_next_frame` | 閉包が `Arg::Value` を「値として」見られること、その解放が次のフレームの Deliver であること |

測り方はどれも S1/S2 と同じ「スクリプト自身が計器」の形にした
（`f0 = $rubevy[:frame]` → `Rubevy.ask(...).pop` → 差を**誰も待たない質問**でゲームに送る）。
`tests/scheduling.rs` と同じ読み方ができる。

**4 本目が本当に効いているかを確かめた。** `.before(RubevySet::Tick)` を `.after(...)` に変えて
1 本だけ走らせると、期待どおり落ちる:

```
assertion `left == right` failed: the closure read what this frame's system had just written
  left: -1.0
 right: 0.0
```

（`-1.0` はコンポーネントの初期値。tick のあとに書くシステムでは、閉包は「まだ誰も書いていない世界」を見る。）
順序を戻して通ることを確かめた。この確認をしたのは、S3 で著者が
「`.before(RubevySet::Tick)` が初めて意味を持つ」と書いた主張の、rubevy 側の対になるからである。

**6 本目の解放の測り方。** 閉包の中で `Request` は落ちる（ループがその場で drop する）ので、
`Arg::Value` の ObjId は解放キューに積まれ、実際に `gc_unregister` されるのは
**次のフレームの `RubevySet::Deliver`** の掃き出しである。外から見るために、テストが
`.before(RubevySet::Deliver)` に自前の掃き出し（`release_dropped_values` を呼んで
`(frame, n)` を記録するだけ）を挿した。プラグインの掃き出しより 1 つ早いので、取るのはこちらになる。
結果は「質問したフレーム＋1 で 1 件」で、`Arg::Value` の寿命は今までの質問とまったく同じだと分かる
（計画書 §5-8 の「答えループの中で `Request` を落とすと、その値の解放は次の Deliver」の実測）。

書きながら踏んだ穴は 1 つだけ。4 本目でコンポーネント `Hp` をスクリプトの entity に足すのを
`run`（`Script` を挿す）より先に書いていて、`Rubevy.entity` を渡した閉包が `None` を返した。
`Script` を挿した entity を受け取ってから `insert(Hp(-1.0))` する順に直した。

---

## 4. example（`examples/nearest.rs` ＋ `assets/scripts/nearest.rb`、どちらも新規）

計画書は「`examples/sensor.rs` か `components.rs` に 1 つ」と書いているが、**新しい example を作った**。
既存の 2 本はそれぞれ「システムが 2 フレーム後に答える」「コンポーネントの読み書き」を見せる筋書きで、
そこに `nearest` を足すと出力が混ざって、S1/S2 が「前と同じ結果」で確かめている 3 本の基準も動く。
新しい 1 本なら、対比（閉包が答える `nearest` は 0 フレーム、システムが答える `weather` は 1 フレーム）を
**同じ出力の中に**並べられる。

中身: 草 5 本（`Plant` ＋ `Transform`）が生き物のまわりを回り（`.before(RubevySet::Tick)` のシステム）、
スクリプトが 5 回「いちばん近い草」を訊いて、その草の `Transform` も読む。

```
[script] nearest: frame 0: the nearest plant is #<Rubevy::Entity 32v0> at 6.0, 0.0 (frames waited: 0)
[script] nearest: frame 0: the weather is clear (frames waited: 1)
[script] nearest: frame 5: the nearest plant is #<Rubevy::Entity 34v0> at -4.6, 3.9 (frames waited: 0)
[script] nearest: frame 5: the weather is rain (frames waited: 1)
[script] nearest: frame 10: the nearest plant is #<Rubevy::Entity 34v0> at -4.2, 4.3 (frames waited: 0)
[script] nearest: frame 10: the weather is clear (frames waited: 1)
[script] nearest: frame 15: the nearest plant is #<Rubevy::Entity 34v0> at -3.9, 4.6 (frames waited: 0)
[script] nearest: frame 15: the weather is rain (frames waited: 1)
[script] nearest: frame 20: the nearest plant is #<Rubevy::Entity 33v0> at 3.6, 4.8 (frames waited: 0)
[script] nearest: frame 20: the weather is clear (frames waited: 1)
[nearest] host: script on 37v0 ended: Finished :nearest_done
```

`frames waited: 0` が 5 回、同じフレーム番号のまま `weather` が `1`。
最初 `plant.to_i`（＝`Entity::to_bits`）を出していたら `4294967263` という読めない数になったので
`plant.inspect` に変えて `.mrb` を取り直した。近い草が途中で変わっている（32 → 34 → 33）のは、
回転のシステムが毎フレーム動いていて、閉包が**そのフレームの**世界を見ている証拠でもある。

`.mrb` は `tools/compile_scripts.sh` と同じ docker のコマンドを 1 ファイルだけに絞って回した
（`docker info` は通った。全部回すと他の `.mrb` も上書きしてしまうため）。

---

## 5. docs

* `docs/host-api.md` に節「Answering inside the tick (`answer_in_tick`)」を新設。`answer_with` の節の
  次、「Components by name」の前に置いた（答えの出どころ 3 つ — システム、Future、tick の中の閉包 —
  が並んで読める）。いつ使うか（毎フレーム・全個体・答えが無いと進めない空間の問い。数字は計画書 §3.6 と
  S1 の実測にあるものだけ: 1 読み ≈ 75 命令、24 × 90 = 2,160）、いつ使わないか（フレームの結果が要る答え →
  `Answer` のシステム、work な答え → `answer_with`）、閉包に渡るもの・渡らないもの（`ScriptWorld` は
  その World にいない。だから `Vm` も無く、答えは平たい `Answer` だけ）、規則 6 つ。
* 同じ文書の `RubevySet` の表の `Tick` の欄に「ゲームが `answer_in_tick` で登録した kind も」を追記し、
  節の末尾に「no frame at all」の段落。`Rubevy.ask` の節の規則にも「答えの出どころは 3 つ」を 1 行。
* `src/lib.rs` の rustdoc: `RubevySet` の表と説明、`take_requests`（2 種類の質問はここに出ない）、
  `RESERVED_KINDS`（`answer_in_tick` でも上書きできない）、`InTickAnswerer`、`answer_in_tick`（doctest 付き）、
  `answer_in_tick_requests`、`take_asks`。doctest は 10 → 11 本になった。
* `README.md` に箇条書き 1 つと「Try it」に `cargo run --example nearest` の 2 行。
* `docs/README.md` の `host-api.md` の説明に `answer_in_tick` を挿し、worklog の目次にこの文書の行。

---

## 6. 確認（すべて実際に走らせた）

* `cargo test`: **73 件通過**（16 ファイル 62 件 ＋ doctest 11。着手前は 56 ＋ 10）。失敗 0。
  `tests/read_cost.rs` の 2 本は `#[ignore]` のまま。
* `cargo build --examples`: 通る（7 本）。
* `cargo run --example nearest`: §4 の出力、0 で終了。
* `cargo clippy --all-targets`: 警告 **5 行**で着手前と同じ。一度 7 行に増えた
  （`manual implementation of .is_multiple_of()` が example の `frame.0 % 2 == 0` に付いた）ので
  `frame.0.is_multiple_of(2)` に直して戻した。
* `grep -rn unsafe src/`: rustdoc の 1 行だけ。コードは 0 件のまま。
* `Cargo.lock` と sabiruby: 触っていない（`git status` に出ない）。
* docs の相対リンク: 変えた 3 つの文書（`docs/host-api.md`、`docs/README.md`、`README.md`）から
  `grep -o '](\S*\.md[^)]*)'` で拾って存在を確かめた。切れは 0。この worklog へのリンクも含む。

---

## 7. 迷った点・置いていくもの

1. **計測をしていない。** 閉包 1 回の費用（＝答えループ 1 周のうちゲームの分）は測っていない。
   計画書 §3.6 は S4 に計測を要求しておらず、指示にも無かったので、**数字を 1 つも作らなかった**。
   docs に書いた数字は全部 S1 の実測か計画書のものである。世界の脚本が本当に使い始めるとき
   （rubevy_games 側）に、`tests/read_cost.rs` と同じ形で「閉包つきの 1 周」を測るのが自然だと思う。
2. **予約 4 種を `answer_in_tick` で登録したときに黙って無視される**（§2.2）。docs には書いたが、
   `warn!` は足していない。足すなら `answer_in_tick` の中の 1 行。
3. **2 つの集合をまたぐ順**（§1.2）。同じ周に rubevy の読みとゲームの質問が積まれたとき、
   rubevy が先になる。計画書が決めているのは各集合の中の順だけなので従ったが、
   「積まれた順で厳密に」にしたいなら `take_asks` を 1 回にして kind で振り分ける形になる
   （そのときは `answer_reflect_requests` と `answer_in_tick_requests` を 1 本にする必要がある）。
4. **`Rubevy::Proxy` は触っていない。** 計画書のとおり `Rubevy.ask` の糖衣なので、
   `garden.nearest(:Plant)` は `"garden.nearest"` を登録すればそのまま同期になる。
   docs に 1 行書いただけで、テストはしていない（proxy 自体のテストは `tests/proxy.rs` にある）。
5. **サンプルゲームは触っていない**（指示のとおり）。計画書 §6 の状況表も触っていない。
