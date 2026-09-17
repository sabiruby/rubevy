# 2026-09-17 コンポーネントの同期読み（S1: 排他 tick と答えループ）

`docs/plans/sync-access-plan.md` の **S1** の記録。ブランチ `sync-reads`、main `989cd2a` から。
計画の前提になっている事実集めは `docs/worklog/2026-09-17-sync-access-survey.md`（行番号つき）にあるので、
ここに書くのは「その通りだったか」「違ったところをどうしたか」と、測った数字と、捨てた案。

**絶対の制約**: `unsafe` を書かない。World への参照をネイティブにも `HostState` にも渡さない。
sabiruby は 1 行も触らない（`Cargo.lock` も 0.5.0 の `e2470626` のまま）。
`grep -rn unsafe src/` は着手前 0 件、終わったあとも 0 件（当たるのは新しい rustdoc の
「**Why there is no `unsafe` in it.**」という文だけ）。

---

## 0. 着手前の状態

`cargo test` は 14 ファイル 53 件 ＋ doctest 10 件がすべて通る（計画書の数と一致）。
`cargo clippy --all-targets` の警告は 5 行（`type_complexity` が `examples/headless.rs:47` に 1 つ、
`collapsible_if` が `src/lib.rs` の `drain_commands` に 1 つ、あとは「N warnings generated」の集計行）。
`cargo run --example components / headless / two_vms` は 3 本とも 0 で終わる。
これを基準にして、終わったあとに同じものを取り直した（§6）。

---

## 1. 作ったもの

### 1.1 tick を排他システムにする

`tick_scripts::<M>` は `Res<Time>` / `Res<FrameCount>` / `ResMut<ScriptWorld<M>>` / `Query` /
`MessageWriter` / `Commands` の 6 引数だった（調査 §1）。これを

```rust
fn tick_scripts<M: 'static>(world: &mut World, tasks: &mut RunningTasks<M>)
```

にした。6 つの代わりにやることは:

* `Time` と `FrameCount` は `world.resource::<_>()` で**先に値を写す**（`delta` / `elapsed` / `frame`）。
  閉包の中で World を触るので、参照を持ち越さない。
* タスクの一覧は `RunningTasks<M>`＝`SystemState<Query<(Entity, &ScriptTask<M>), Without<ScriptDone<M>>>>`。
  **`world.query_filtered::<..>()` は使わなかった**: bevy 0.19.1 の `World::query_filtered` は
  `QueryState::new(self)` そのもの（`bevy_ecs/src/world/mod.rs:1827-1829`。キャッシュは無い）で、
  毎フレーム全アーキタイプの照合をやり直すことになる。排他システムは `&mut World` の隣に
  `&mut SystemState<P>` を取れる（`bevy_ecs/src/system/exclusive_system_param.rs:58`）ので、
  Query のときと同じようにアーキタイプの照合が持ち越される。
* `MessageWriter` の代わりに `world.write_message(ScriptEnded { .. })`、
  `Commands` の代わりに `world.get_entity_mut(e)` → `insert(ScriptDone)`。
  **どちらも `resource_scope` の閉包を抜けてから**やる（下記 1.4）。

`SystemState::get` は `Result<SystemParamItem, SystemParamValidationError>` を返す（同 `function_system.rs:370-373`）。
読み取り専用の `Query` は検証で落ちないので `Err` 側は到達しないが、`unwrap` でフレームごと落とすのは
割に合わないので `.map(..).unwrap_or_default()` にした。
（最初 `tasks.get(world).iter()` と書いて `Result::iter` に解決され、
「expected `Query<..>`, found tuple」という読みにくいエラーで 1 往復した。）

### 1.2 答えループ

`resource_scope::<ScriptWorld<M>, _>` の閉包の中身が計画 §3.1 の形:

```rust
loop {
    let left = budget - spent;                       // 予算切れなら抜ける（budget == 0 の一時停止もここ）
    let time_left = frame_time - started.elapsed();  // 時間切れなら抜ける
    spent += vm.task_run_limits(RunLimits { instructions: Some(left), time_ns: time_left, overrun_ns })?;
    if answer_reflect_requests(&*world, scripts) == 0 { break; }   // 誰も起きないなら回しても無駄
}
```

`task_run_limits` は「実行できるタスクが無い」「予算切れ」「時間切れ」のどれでも戻り、
**使った命令数を返す**（sabiruby `src/vm.rs:780-796` が `VmResult<u64>`）。調査 §5 の通りだった。

**周回上限は置かなかった。** 最初は計画どおり `ROUNDS_PER_TICK = 64` を書いたが、
著者から「64 の根拠は？」（＝根拠なし）という指摘があり、外した。外して困らない理屈はこうなる:
1 周は「前の周が誰かに答えた」ときだけ起きる。質問を 1 つ作るには Ruby 側で `Rubevy.ask` と `pop` が
走るので、**周回は必ず命令数を食う**（実測 1 読みあたり 75 命令、§5）。だから予算 200,000 が
そのまま周回の上限になり（実測 2667 周）、その前に `frame_time` が来ればそちらで止まる。
新しい定数を 1 つも足さずに、フレームが元から持っていた 2 つの数だけで閉じている。

### 1.3 `answer_components` を関数にする

`answer_components`（排他システム、`RubevySet::Answer`）は消して、中身を
`answer_reflect_requests(world: &World, scripts: &mut ScriptWorld<M>) -> usize` にした。
4 種（`component.get` / `component.has` / `components` / `entities.with`）の反射の中身は**そのまま**。
変えたのは 3 つだけ:

* 引数が `&World`。`ReflectComponent::reflect` / `contains` は `&World` で足りる（調査 §4）ので、
  `resource_scope` も `&mut World` も要らない。**World への参照はこの関数の引数として生き、
  ネイティブにも `HostState` にも渡らない**。これが unsafe が要らない理由のすべて。
* 質問の出どころが 2 つになった。まず `ScriptWorld::reflect_requests`（前のフレームの残り。
  下記 1.5）、次に VM のコマンドキューから `take_reflect_asks` で拾った今周の分。
* 型名の解決がキャッシュ経由（1.6）。

`drain_commands` の `RESERVED_KINDS` の仕分けは**残した**（計画 §2 既定 7）。
普段そこを通る質問はもう無いが、残り物（1.5）を `reflect_requests` に入れるのはこの道。

`RubevySet::Answer` に rubevy のシステムは 1 本も無くなった。ゲームだけの場になる。

### 1.4 閉包の中で構造変更をしない

調査「引っかかりそうな点」3 の通り、`resource_scope` の間は `ScriptWorld<M>` が World から
抜かれている（`bevy_ecs/src/world/mod.rs:2882` の `entity_mut.take::<R>()`）ので、
`ScriptTask` の `on_remove` フック（`src/lib.rs:237-250`）は `get_resource_mut` に失敗して
黙って戻る。だから終わったタスクの始末（`ScriptEnded` の write と `ScriptDone` の insert）は
**閉包を抜けてから**やる。

閉包を抜けたあとの順序は「(a) `ScriptWorld` を `resource_mut` で取り、終わったタスクを数えて
`gc_unregister` / `unsubscribe` してメッセージを組む → (b) 借りを返してから `write_message` と
`insert(ScriptDone)`」。(a) と (b) を分けているのは、(b) が World の構造を変える＝
その最中に `ScriptWorld` を借りていてはいけないから。

`ScriptDone` の挿入が `Commands` 経由（フレーム末尾）から即時に変わったが、
このフレームのうちに `ScriptDone` を見るシステムは rubevy には無い（`Without<ScriptDone<M>>` を
見るのは次のフレームの tick）ので、観測できる違いは無い。テストも全部そのまま通った。

### 1.5 残り物（持ち越し）

予算切れ・時間切れで tick が終わったとき、コマンドキューに答えていない読みが残ることがある。
その分は `drain_commands` が今までどおり `reflect_requests` に仕分け、
次のフレームの答えループが**最初に**拾う（`std::mem::take` してから今周の分を継ぎ足す）。
`Startup` の `load_and_run` で積まれた読みも同じ道を通る（tick の外では `drain_commands` も
まだ走っていないので、最初の tick の 1 周目が VM のキューから直接拾う）。
どちらも例外にはしない（計画 §2 既定 4）。テストは
`a_read_asked_before_the_first_tick_is_answered_by_it`。

### 1.6 `ReflectComponent` のキャッシュ

`ScriptWorld` に `reflect_cache: HashMap<String, ReflectComponent>` を足した。
根拠は bevy 自身の rustdoc（`bevy_ecs/src/reflect/component.rs:326-330`、調査 §6 が引いている）。
名前が登録に無いときは**入れない**（あとから登録される型があるので）。
外したのは「登録が無い」の負のキャッシュで、外れたときの費用はハッシュ 1 回。

キャッシュにはもう 1 つ役目があった。元のコードは `named(i)` という閉包で
`&TypeRegistration` を借りていて、その借りが生きている間は `scripts`（＝`&mut ScriptWorld`）を
`answer_value` に渡せない。`ReflectComponent` を**クローンして持つ**形にすると借りが切れるので、
借用検査のための手当てが 1 つ減った。

---

## 2. 変えなかったもの（計画の既定の通り）

* `src/prelude.rb`（`Rubevy::Entity#[]` は Ruby のまま。ネイティブの中では park できないという
  制約が変わっていないので、`.pop` で待つ形が正しいまま）。`prelude.mrb` も再生成していない。
* 書きの道（`Rubevy.set_component` → `component_writes` → `apply_component_writes`）。
* 構造変更（`Rubevy.spawn` / `despawn`）は `Commands` のまま。
* `Cargo.lock`（sabiruby 0.5.0 の `e2470626` のまま。新しい API は 1 つも要らなかった）。

---

## 3. 意味論がどう動いたか

| いつ | 前 | 後 |
|---|---|---|
| `e[:Hp]` が返る | 次のフレームの Tick | **同じ tick の同じ行** |
| 読む値の時点 | N の Answer（このフレームの書きが済んだあと） | **N の Tick**（このフレームの書きの前） |
| 読み → 書き → 読み | 2 度目は書いた値 | **2 度目は古い値**（書きは末尾のまま） |
| ゲームが答える `Rubevy.ask` | 1 フレーム | 1 フレーム（変わらず） |
| 2 本の VM の tick | 順序なし | 直列（排他システムなので） |

2 番目と 3 番目が S3（サンプルゲーム）で効く。3 番目はテストで固定した
（`a_read_after_a_write_in_the_same_tick_is_still_the_old_value`）。

---

## 4. テスト

| ファイル | 何を足した／変えた |
|---|---|
| `tests/scheduling.rs` | `a_component_read_still_costs_one_frame` → **`a_component_read_costs_no_frame`**。期待値 `[1,1,1,1,1,1]` → `[0,0,0,0,0,0]`。名前も見出しも嘘になるので直した。ゲームが答える方の 1 フレームは同じファイルにそのまま残っているので、「答える人が誰かで費用が違う」が 1 つのファイルで読める |
| `tests/components.rs` | `a_read_after_a_write_in_the_same_tick_is_still_the_old_value`（既定 9）、`a_read_asked_before_the_first_tick_is_answered_by_it`（既定 4） |
| `tests/two_vms.rs` | `each_vm_answers_its_own_reads_inside_its_own_tick`（2 本の VM がそれぞれ自分の読みを自分の tick で返す） |
| `tests/read_cost.rs`（新規） | 計測用。2 本とも `#[ignore]`（§5） |

**無変更で通ったもの**: `tests/entity.rs`、`tests/proxy.rs`、`tests/components.rs` の既存 6 本
（`rubevys_own_questions_never_reach_the_game` を含む。計画が「通るはず」と書いていた通りで、
`RESERVED_KINDS` は残っているので主張も空虚になっていない）、`tests/events.rs`、`tests/pause.rs`、
`tests/replace.rs`、`tests/restart_burst.rs`、`tests/futures.rs`、`tests/host_data.rs`、
`tests/child_task.rs`、`tests/ask_value.rs`、`tests/vm_setup.rs`、doctest 10 本。

書きながら踏んだ穴が 1 つ。`a_read_after_a_write_...` を最初 `frames(&mut app, 12)` で書いたら
スクリプトが `Rubevy.ask` に届かなかった。原因は `sleep 0.05` が**実時間**で、
このテストの 1 フレームは `frames` が入れている 2 ms ＋ 更新の実費しかないため、
12 フレームでは 50 ms に届いていなかった。40 フレームにした。

---

## 5. 計測

`tests/read_cost.rs`、`cargo test --release --test read_cost -- --ignored --nocapture`。
同期化の前の数字は、`src/lib.rs` だけ `git stash` して同じテストを走らせて取った
（テストは公開 API しか使っていないので、前の実装でもそのまま動く）。
機械は WSL2、他に `cargo` / `sabiruby` のプロセスは動いていない（`pgrep -af` で確認）。
フレームの実時間は ±300 µs 揺れるので、**5 回走らせて median の最小**を採った。

### 5.1 読みしかしないタスク 1 本が 1 フレームで何周できるか

スクリプトは `loop { e[:Hp]; n += 1; break if $rubevy[:frame] != f0 }`。
`$rubevy` は tick の頭で作り直されるので、スクリプト自身が「最初のフレームのうちに何回読めたか」を数えられる。
**この形では 1 読み＝答えループ 1 周**（読むたびに park し、答えが来て次の周で再開する）。

| | 読めた回数 | そのフレームの実時間 | 1 周あたり |
|---|---|---|---|
| 前（読み＝1 フレーム） | **1** | 0.66–1.23 ms | — |
| 後、既定（予算 200,000 命令・`frame_time` 8 ms） | **2667** | 6.10 ms | **2.29 µs** |
| 後、`frame_time = None`（予算だけ） | **2667** | 5.89 ms | **2.21 µs** |

2 行目と 3 行目が**同じ 2667** なのが要点で、止めたのは 8 ms の時計ではなく**命令数の予算**だと分かる
（6 ms しか使っていない）。200,000 / 2667 ＝ **1 読みあたり約 75 命令**。
この 75 命令は Ruby 側の `Rubevy.ask` ＋ `Queue#pop` ＋ `Rubevy::Entity#[]` の呼び出しで、
周回上限を置かなくても予算が周回を有限にする（1.2）ことの実測でもある。

1 周あたり 2.2 µs の内訳は「`task_run_limits` への再入 ＋ `take_reflect_asks` ＋ 型名の解決（キャッシュ）＋
`rc.reflect` ＋ `reflect_to_ruby`（Ruby ヒープの確保 3 個）＋ `push_answer` ＋ Ruby の 75 命令」。
調査 §6 の推測（「1 件は µs 未満のまま、効くのはレイテンシ」）はだいたい当たっていて、
1 件の実費は µs の桁の頭（2 µs 台）、消えたのは 16.7 ms の待ちの方だった。

### 5.2 1 フレームに 24 タスクが 4 回ずつ読む

`loop { 4.times { e[:Hp] }; sleep 0.004 }` を 24 体。対照は同じスクリプトから読みだけ抜いたもの
（`4.times { e }`）。200 フレームの median、5 回の最小:

| | 読みありのフレーム | 対照（読みなし） | 差 | 1 フレームの読み |
|---|---|---|---|---|
| 前 | 520 µs | 485 µs | 35 µs | 24 回（タスクごとに 1 回で park する） |
| 後 | 637 µs | 326 µs | 311 µs | **96 回** |

差 ÷ 読みの回数は前が 1.5 µs、後が 3.2 µs で、5.1 の 2.2 µs と桁は合う（この測り方は揺れに弱い。
実際 1 回だけの測定では対照の方が遅く出たことがあり、5 回の最小を採る理由になった）。
**読める回数が 4 倍になって、フレームは 0.3 ms 伸びた**というのがこの表の言っていることで、
「毎フレーム全員を見る」世界の脚本（`rubevy_games/docs/plans/garden-world-plan.md`）には十分に見える。
対照が前より速くなっている（485 → 326 µs）のは、排他 tick がフレームを遅くしていないことの傍証。
ただし対照の差は揺れの幅と同じくらいなので、「遅くなっていない」以上のことは言わない。

---

## 6. 確認（すべて実際に走らせた）

* `cargo test`: **66 件通過**（14 ファイル 53 件 → 15 ファイル 56 件 ＋ doctest 10）。失敗 0。
  `tests/read_cost.rs` の 2 本は `#[ignore]` なので既定では走らない。
* `cargo build --examples`: 通る。
* `cargo run --example components` / `headless` / `two_vms`: 3 本とも前と同じ結果で 0 終了。
  `components` は**中身が同じで時刻だけ変わった**: 先頭の 4 行（`I am` / `I have` / `a Waypoint?` /
  `step 0`）が前は 1 行ずつ別のフレームだったのが、同じミリ秒に並ぶ。
  最後の `Transform` も前と同じ `Vec3(5.0, 2.0, 3.0)`。
* `cargo clippy --all-targets`: 警告 5 行で着手前と同じ。
  一度 6 行に増えた（`type_complexity` が `tick_scripts` の `&mut SystemState<Query<...>>` に付いた）ので、
  `type RunningTasks<M>` に括り出して戻した。
* `grep -rn unsafe src/`: rustdoc の 1 行だけ。コードは 0 件のまま。

---

## 7. 迷った点・置いていくもの

1. **`assets/scripts/components.rb:12` のコメント**が「parked here until the host answers, next frame」と
   言っていて、もう嘘。サンプルの Ruby は S2/S3 の範囲なので触っていない。
2. **`docs/host-api.md:583-586`（「ネイティブから World に触るな」）は、この形なら書き換える必要がない。**
   計画 §3.3 は初版（unsafe で World を貸す案）の名残でここを「読みだけ、tick の間だけ貸す」に
   直すと書いているが、答えループの形ではネイティブは今までどおり `&mut Vm` しか受け取らない。
   S2 で直すのは「読みが 1 フレームかかる」の側だけでよいはず。`src/lib.rs:672-674` の同じ規則は
   そのままにしてある。
3. **`overrun_ns` は毎周そのまま渡している**（経過で減らしていない）。`overrun` は
   「切り替えられないタスクが 1 本で居座ったら `Task::Overrun`」の閾値で、1 回の
   `task_run_limits` の中の話だから、周ごとにリセットされるのが自然だと判断した。
   結果として「1 フレーム全体では overrun ×周回数まで居座れる」形にはなるが、
   居座るタスクは命令も時間も食うので `frame_time` が先に来る。
4. **答えループは `deliver_answers`（Future）と `Rubevy.ask`（ゲームの質問）には触らない。**
   これらは tick の外で答えが来るので、park は今までどおり残る（計画 §1 の「park はなくならない、減るだけ」）。
5. **計測をテストとして残すかどうか。** `#[ignore]` で残した（`tests/read_cost.rs`）。
   数字は機械のものなので assert は 1 つも書いていない。要らなければ消してよい。

---

# S2: 文書（同じブランチ、`d0e9b85` の上）

`docs/plans/sync-access-plan.md` の **S2**。コードは変えない段階なので、やったことは
「S1 で嘘になった文を探して、答えループの説明に書き換える」だけ。ただし探すのは骨が折れたので、
何をどう探して、どれを直し、どれをわざと残したかを書いておく。

## 1. まず「今」を確かめた

計画書 §3.3 は行番号で直す場所を挙げているが、S1 が `src/lib.rs` を大きく動かしているので、
行番号は当てにせず `grep -rn "one frame\|next frame\|1 フレーム\|answer_components"` で拾い直した。
その結果、**rustdoc は直すものが残っていなかった**: S1 が crate doc（`src/lib.rs:25-31`）、
`RubevySet` の表（`:1233`）、`reflect_requests` / `reflect_cache` のフィールド（`:722-737`）、
`tick_scripts`（`:1517-1545`）、`answer_reflect_requests`（`:1818-1830`）を書き換え済みで、
残っている「1 フレーム」の言及（`:1250-1253`、`:854`、`:1946`）は
**ゲームが答える質問と、書きと、`Arg::Value` の解放**の話で、どれも今も本当だった。
計画書が挙げていた `:682-700`（`ScriptWorld::vm` の「World に触るな」）も、
S1 の worklog §7-2 の判断どおりそのままでよい（ネイティブが受け取るのは今も `&mut Vm` だけ）。
だからこの段階で触ったコードは、計画書とプロンプトが名指しした 2 か所だけになった。

## 2. `docs/host-api.md`

三つ。

**`RubevySet` の表。** `Answer` の欄から `answer_components` が消えた（システムごと無くなったので、
`Answer` はゲームだけの場になった）。`Tick` の欄に `(exclusive)` と「読みは走っている間に答える」を足した。
そのうえで表の下に段落を 1 つ足して、排他であることの**見えにくい方の帰結**を書いた:
2 本の VM の `Tick` は重なれないので直列になる。ほかは全部 VM ごとに別なのに、ここだけが共有の直列点になる。

**「A read costs a frame」→「A read costs no frame」。** 理由ごと書き換えた。
`> run the ready tasks → answer the component reads they parked on → run them again` の 1 行を軸に、
`task_run_limits` が「走れるタスクが無い」で戻ることがなぜ「この周の読みが全部ホスト待ちになった瞬間」なのか、
なぜループに自前の上限が要らないのか（質問には命令が要るので予算が周回を抑える）、
なぜ `unsafe` が要らないのか（World への参照はネイティブに渡らない）を書いた。
そのあとに「読みが何を見て、何が残っているか」を 4 つの箇条書きにした:
Tick 時点の世界（＝ゲームの `.before(RubevySet::Tick)` が初めて意味を持つ）、
同じ tick で書いた値はまだ読めない、**安くなったのは待ちであって走査ではない**（`find` と `components` は
今も世界と型登録を舐める）、tick の外の読みは最初の tick が答える。
数字は S1 の §5 にある実測だけを使った（1 読み 2.2 µs、読みしかしないタスクが 1 フレームに 2,667 回）。
新しい数は 1 つも作っていない。

**「Why a read can wait inside `[]`」は残した。** 読みは今も `[]` の中で park する
（待ちが 1 フレームからループ 1 周になっただけ）ので、`OP_GETIDX` を本家と同じ送信にした話は今も要る。
書き出しに「待ちは短くなったが、park しているのは本当だ」の 1 文を足しただけにした。
計画書 §3.3 が「理由ごと書き換え」と言っているのは、park が消える前提（初版の案）の名残だと判断した。

`:583-586`（「ネイティブから World に触るな」）は**手を付けていない**。プロンプトの指示どおりで、
S1 の worklog §7-2 と同じ理由。

## 3. `docs/rust-bridge.ja.md`

日本語の橋の解説。3 か所。

* §3.3 の「VM を動かしている最中に Bevy の `World` を借りる必要がない。…並列実行の妨げにもならない」——
  ゲームが答える質問については今も本当だが、**tick が排他システムになった以上、後半は嘘**。
  「ゲームが答える質問については」と限定し、コンポーネントの読みは別だと括弧で足した。
* §3.4 の `tick_scripts` の 3 行のコード片。計画書 §3.3 はここを挙げていないが、
  この文書は「橋を 1 点ずつ追う」体裁で、いちばん中心の絵が「3 行の直線」のままでは誤解を招く。
  コード片は残して、下の箇条書きに「2026-09-17 から、この 3 行はループになっている」を 1 つ足した。
  **計画に無い追記なので報告に挙げる。**
* 末尾の弱点表。「1 回の質問に 1 フレーム前後かかる」は**ゲームが答える質問に限る**と直し、
  「読みは 1 往復 = 1 フレーム」の行は取り消し線にして、同じフレームで返るようになったことと数字を書いた。
  「`unsafe` の数について」の補足は**触っていない**（今回も増えていない）。

## 4. `docs/outlook.md` / `docs/outlook.ja.md`

この 2 つは「作る前に書いた見通し」と「今どうなったか」が同じ項目に同居している文書なので、
**見通しの側は書き換えず、`Now:` / 括弧の側に足す**形にした。

* `outlook.md` 項目 3（ECS の橋）の見通しは「排他システムが VM を回す間だけ `&mut World` をホストに貸す」。
  当たったところと外れたところが両方あるので、そう書いた: 排他システムは本当に来たが、
  **誰にも貸していない**（tick 自身が World を持ったまま、VM を走らせる合間に答える）。
* 項目 4 の「A read is one question and one frame」を、答えループの説明と数字に差し替えた。
  「1 フレームに 12 回読むような使い方には向かない」という結びも、もう向くので直した。
* 「Why it fits」の「A host call never re-enters the VM, so `&mut World` can be lent」も同じ直し方。
  実際に起きたのは「貸す」ではなく「同じシステムが両方を持てる」だった。
* `outlook.ja.md` は著者が**最初から同期前提で書いていた**ところ（`:211`「読み取りはその場の値が返ります」）で、
  S1 より前は実装の方が後退していた（調査 §「直す文書」もそう指摘している）。
  ここは「これは 2026-09-17 に本当になりました」と書けるようになったので、そう書いた。
  裏（同じ tick で書いた値は読めない）も同じ段落に添えた。
  末尾の「宣言時とイベント時に触る方針はそのまま守られています」は、もう性能上の制約ではなく
  ただの書き方の好みなので、そう書き換えた。

## 5. 残り

* `docs/plans/ecs-bridge-plan.md` の冒頭「排他システムの中で同期に読む形は、遅さが問題になったときに足す」に
  「**足した**（2026-09-17、sync-access-plan の S1）」の 1 行。この計画書自身の状況表は S1/S2 の対象ではないので触っていない。
* `README.md` の「Components by name」の箇条書きに 2 文（読みはフレームを食わない、2.2 µs / 2,667 回、書きは末尾のまま）。
* `docs/README.md` の目次の `host-api.md` の説明文に「a read is answered inside the tick that asked it」を挿入。
* `assets/scripts/components.rb:12` のコメント（S1 の worklog §7-1 が「嘘」と書いて置いていったもの）。
  `# parked here until the host answers, next frame` → `# answered inside this tick: the value is here, now`。
  **1 行のまま**に収めたのは、`.mrb` が checked-in の生成物で、行が増えるとデバッグ情報の行番号がずれるから
  （コメントだけなら再生成しても同じバイト列になる）。再生成は Docker が要るのでしていない。
* `examples/two_vms.rs` の `give_the_mods_less` の rustdoc に 1 行。「フレームの最悪値は和」に
  「本当に和になる（tick が排他なので 2 本は直列）」を足した。

## 6. 確認

* `cargo test --doc`: **10 件通過**、失敗 0。
* `cargo build --examples`: 通る。
* 相対リンク: 変えた 7 つの文書から `grep -o '\](\S*\.md[^)]*)'` で拾って存在を確かめた。切れは 0。
  バッククォートの中のパス（`docs/...`）も同じように確かめた。存在しないものはあるが、
  どれも他の repo（sabiruby、rubevy_games）の文書を指す前からの言及で、今回足したもの
  （`docs/plans/sync-access-plan.md`、`docs/worklog/2026-09-17-sync-reads.md`）は両方ある。
* 行幅は元の文書に合わせた（100 桁前後で折る）。表の行だけは元から長い。

## 7. 置いていくもの

`src/prelude.rb` のコメントに嘘が 2 つ残っている。冒頭の「work stays on the Rust side
(`answer_components` in src/lib.rs)」は関数名が変わった（`answer_reflect_requests`）し、
`Entity#get` の「The answer comes from the host on the next frame, and the task is parked
until it does」は 1 フレームを約束してしまっている。**直していない**: 今回の指示がコードを
`assets/scripts/components.rb` の 1 行に限っていること、`prelude.mrb` が checked-in の生成物で
再生成に Docker が要ることの 2 つが理由。次にこのファイルに触るとき（S3 でも別の段でも）に
一緒に直すのがよい。
