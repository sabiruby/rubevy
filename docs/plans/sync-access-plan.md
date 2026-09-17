# rubevy: コンポーネントの同期読み（tick の中でその場で読む）— 実装指示書

作成 2026-09-17。著者「同期の読み書きを考えるべきタイミングでは」。調査は `docs/worklog/2026-09-17-sync-access-survey.md`（行番号つき。**着手前に全部読む**）。
対象: rubevy main `bfa47ed`、sabiruby main `8faf26f`（0.5.1。rubevy の `Cargo.lock` は 0.5.0 の `e2470626` を指している）、Bevy 0.19.1。

**改訂（2026-09-17、著者「unsafe は増やさないこと」）**: 初版は sabiruby に `&World` を貸す口（unsafe 1 か所）を足す案だった。著者の指示で**unsafe ゼロ・sabiruby 無変更**の形に改めた。
鍵は sabiruby の `task_run_limits` が「実行できるタスクが無くなった」時点でも戻ること（`src/vm.rs:797-816`、`Step::Stuck`）。tick の中で「VM を回す → 止まったら溜まった読みの質問を `&World` で答えてタスクを起こす → また回す」を繰り返せば、World への参照はネイティブに一切渡さずに、読みは同じ tick の中で返る。

**この文書だけで着手できるように書いてある。** 読む順は 1 → 2 → 3 → 4。5 は着手前に必ず目を通す。

---

## 0. はじめの一歩

```bash
# rubevy だけ。sabiruby は触らない
cd /home/kishima/book/kishima/rubevy-wt-sync     # worktree はもうある（ブランチ sync-reads、main から）
cargo test                                        # 着手前に全部通ること（tests 14 本 53 件 + doctest 10）
```

作法は `/home/kishima/book/.claude/agents/implementer.md`。段階ごとに 1 コミット、push しない、main に触らない、過程は worklog に書きながら進める（捨てた案と理由も）。決めきれない点は止まって報告する。

---

## 1. 何を作るのか（30 秒版）

今の `e[:Hunger]` は `Rubevy.ask("component.get")` の往復で、**読み 1 回に 1 フレーム**かかる（N の Tick で訊き、N の Answer で `answer_components` が答え、N+1 の Tick で起きる）。
生き物が 0.2 秒に 1 回判断する分には効かなかったが、「毎フレーム全員を見る」世界の脚本（`rubevy_games/docs/plans/garden-world-plan.md`）はこれに正面からぶつかる。

これを**同じ tick の中で返る**形にする:

```ruby
hp = me[:Hunger]        # 今: 1 フレーム待つ。これから: 同じ tick の中で返る（このフレームの Tick 時点の値）
me[:Velocity] = [[vx, vz]]   # 変えない: Tick の末尾で反映され、次のフレームから見える
```

**読みだけ同期にする。書きは今のまま。** 理由は 3 つ:
1. 読みは `&World` で足りる（`ReflectComponent::reflect` / `contains`、調査 §2・§4）。書きは `&mut World` が要り、別名と順序の問題が一気に増える。
2. 書きを末尾反映のままにすると、ゲームが頼っている **wheel / last-writer-wins の取り決めが変わらない**（`garden/ruby/prelude.rb:97-107`、`sabibots/ruby/prelude.rb:250-254`）。
3. 前例: 往復を 2→1 フレームにしただけで Battle の反射テストが 5 回中 3 回落ちた（`rubevy_games/docs/worklog/2026-09-17-battle-followups.md:249-270`）。一度に動かす意味論は 1 つにする。

同期になるもの: `Rubevy::Entity#[]` / `get` / `has?` / `components`、`Rubevy.find`。
同期にならないもの: `Rubevy.ask` 一般と `Rubevy::Proxy`（答え手はゲームのシステム）、`answer_with`（Future）、`subscribe` の `pop`、`sleep`、`Rubevy.spawn` / `despawn`（構造変更は `Commands` 経由のまま）、**書き**。

---

## 2. 決まっていること・既定

### 決まっている（著者）

1. 同期の読み書きをやる（2026-09-17）。この計画では読みを同期に、書きは据え置き（上の理由。書きの同期は §4 の S4 として後で判断）。

### 既定（違和感があれば止めて報告する）

| # | 項目 | 既定 |
|---|---|---|
| 1 | **unsafe を増やさない**（著者の指示） | rubevy は unsafe ゼロのまま。sabiruby は**触らない**。World への参照はネイティブに渡さない |
| 2 | 読みの答え方 | **tick の中の答えループ**。ネイティブは今のまま `Request` を積んでタスクを park する（`Rubevy.ask` + Ruby 側の `pop`）。排他 tick が `task_run_limits` の戻りごとに `reflect_requests` を `&World` で答えて `push_answer` し、答えた質問が 1 つでもあれば残り予算でもう一度 `task_run_limits`。答えるものが無いか、予算・時間を使い切ったら tick を終える |
| 3 | tick の形 | `tick_scripts::<M>` を排他システム（`fn(world: &mut World)`）に。`resource_scope` で `ScriptWorld<M>` を抜き、閉包の中で `&*world` を使って答える。2 本の VM の tick は直列になる（今は順序なし。`examples/two_vms.rs` に 1 行） |
| 4 | tick の外で読みを呼んだとき | 今と同じ道（`Request` が積まれ、次の tick の答えループで答える）。**例外にはしない**（`Startup` の `load_and_run` で `e[:X]` を呼ぶと、そのタスクは最初の tick で答えを受け取る）。docs に「tick の外では待つ」と書く |
| 5 | 型名 → `ReflectComponent` の引き方 | 名前をキーにした**キャッシュ**を `ScriptWorld<M>` に持つ（bevy 自身の助言 `bevy_ecs/src/reflect/component.rs:326-330`。`ReflectComponent` は clone が安い）。外れたら `AppTypeRegistry.read()` を引いて入れる。無い名前は毎回引く（後から登録される型のため） |
| 6 | `Rubevy.find` と `components` | 同じ答えループで答える。費用は今と同じ（全エンティティ / 全登録型の走査）。docs に「安くなったのはレイテンシで、走査は残る」と書く |
| 7 | `answer_components` と `RESERVED_KINDS` | `answer_components` は**消す**（答えループがその中身を関数として使う）。`RESERVED_KINDS` の仕分け（`drain_commands`）は残るが、仕分け先は tick の中で消費される |
| 8 | `Rubevy::Entity#[]` 等の実装 | **変えない**（Ruby の `pop` で park する今の形が、ネイティブの中で park できない制約（`src/prelude.rb:5-11`）にちょうど合っている） |
| 9 | 書きの道 | 変えない（`Rubevy.set_component` → `component_writes` → `apply_component_writes` を Tick の末尾で）。同じ tick で書いた値はまだ読めないことを docs に明記する |
| 10 | 答えループの止まり方 | **予算（命令数）と `frame_time` だけ。** 周回数の上限は置かない（初版の「64」は根拠なし、著者の指摘で撤回）。1 周は答えた質問が 1 つ以上のときだけ回り、質問を出すには Ruby 側で `Rubevy.ask` と `pop` が走って命令数を消費するので、予算が周回を有限に抑える。ホスト側の 1 周の手間は毎周の `frame_time` の確認で抑える。**1 周の手間と、読みしかしないタスク 1 本が 1 フレームで回れる周回数を測って worklog に残す** |
| 11 | サンプルゲーム | rubevy 側の段階が終わってから（S3）。ゲームの selftest が閾値で落ちたら**閾値を動かす前に止まって報告** |

---

## 3. 設計

### 3.1 答えループ（S1 の核）

```rust
// tick_scripts::<M>(world: &mut World) の中、resource_scope の閉包
let started = Instant::now();
let mut spent = 0u64;
loop {
    let left = budget.saturating_sub(spent);
    if left == 0 { break; }
    let time_left = frame_time.map(|t| t.saturating_sub(started.elapsed()));
    spent += scripts.vm.task_run_limits(RunLimits { instructions: Some(left), time_ns: time_left..., overrun_ns })?;
    // 止まった: 実行できるタスクが無い、か、予算・時間切れ
    let answered = answer_reflect_requests(&*world, &mut scripts);   // 旧 answer_components の中身。&World で足りる
    if answered == 0 || time_left == Some(0) { break; }               // 上限はこの 2 つと命令数の予算だけ
}
```

* `answer_reflect_requests` は今の `answer_components`（`src/lib.rs:1670-1776`）の中身を関数にしたもの。`world.resource_scope` の外側から `&World` を受け取るので `resource_scope` は要らない（`ScriptWorld<M>` はもう手元にある）。`reflect_to_ruby` は `&mut Vm` と `&dyn PartialReflect` だけで足りる（調査 §2）。
* 答えた質問のタスクは `push_answer` で READY に戻り、次の `task_run_limits` で続きを走る。**同じ tick の中で**。
* 読み → 書き → 読み: 書きは `component_writes` に積まれるだけなので、2 度目の読みは古い値（既定 9）。テストで固定する。
* `Request` の答えが tick の中で来るので、`deliver_answers`（Deliver の段）を通らない。ゲームが答える質問（`garden.*`）は今までどおり Answer の段 → 次の Deliver。

### 3.2 rubevy（S1）

* `tick_scripts::<M>(world: &mut World)`: `Time` / `FrameCount` は先に値を写す。タスクの列挙は `world.query_filtered::<(Entity, &ScriptTask<M>), Without<ScriptDone<M>>>()` で `resource_scope` の前に集める。閉包の中で 3.1 の答えループ。終わったタスクの `ScriptEnded<M>` は `world.write_message`、`ScriptDone<M>` は `world.entity_mut(e).insert`（**閉包を抜けてから**。閉包の中で構造変更をしない: `ScriptTask` の `on_remove` フックが `ScriptWorld` を見つけられず空振りする、調査「引っかかりそうな点」3）。`Commands` は使わない。
* `ScriptWorld<M>` に `reflect_cache: HashMap<String, ReflectComponent>`（既定 5）。
* 消すもの: `answer_components`（中身は関数へ）、その `add_systems`。`src/prelude.rb` は変えない。
* テスト: `tests/scheduling.rs:83-104` の期待値 `[1,1,1,1,1,1]` → `[0,0,0,0,0,0]`。`tests/components.rs:215-236`（`rubevys_own_questions_never_reach_the_game`）は「rubevy が答える質問はゲームの `take_requests` に出ない」という主張なので**そのまま通るはず**（通らなければ報告）。新テスト: 同じ tick の中で読み → 書き → 読み で古い値が返ること（既定 9）、`Startup` の `load_and_run` で読んだタスクが最初の tick で答えを受け取ること（既定 4）、2 本の VM でそれぞれ自分の読みが同じ tick で返ること（`tests/two_vms.rs` に 1 本）。`tests/entity.rs` / `tests/proxy.rs` は無変更で通ること。
* **性能**: `tests/components.rs:80-105` の形で「1 フレームに 24 タスクが 4 回ずつ読む」を組み、答えループの周回数と 1 フレームの所要を測って worklog に（同期化の前後で。読み 1 回の費用が µs の桁であることを確かめる）。
* `Cargo.lock` は**触らない**（sabiruby 0.5.0 のまま。同期読みに sabiruby の新しい API は要らない）。

### 3.3 docs（S2）

`docs/host-api.md:329-362`（A read costs a frame / Why a read can wait inside `[]` — 理由ごと書き換え）、`:95-137`（RubevySet の表: Tick が排他で直列点になる）、`:583-586`（「ネイティブから World に触るな」の規則は**そのまま**。答えループの形ではネイティブは今までどおり `&mut Vm` しか受け取らない。初版の名残で書き換え対象にしていたが不要）。`assets/scripts/components.rb:12` のコメント（"parked here until the host answers, next frame"）は嘘になったので直す。`src/lib.rs:25-28`（crate doc）、`:682-700`、`:1646-1669`、`:1773-1777`。`docs/rust-bridge.ja.md:344-345,353`（「`unsafe` の数について」は**そのまま**: 今回も 0）。`docs/outlook.md:63-65,84-92,150-153`、`docs/outlook.ja.md:208-216`（`:211`「読み取りはその場の値が返ります」が本当になった）。`docs/plans/ecs-bridge-plan.md:5` の「遅さが問題になったときに足す」に「足した」と 1 行。`docs/README.md` の目次。

### 3.4 サンプルゲーム（S3、`rubevy_games`）

* `Cargo.lock` を rubevy の新しい rev に（sabiruby はそのまま）。
* **順序**: 読みが「N の Tick 時点」を見るので、スクリプトが読むコンポーネントを書くシステムは **`.before(RubevySet::Tick)`** に置く（garden の規則 chain `main.rs:1301-1325` は今 `RubevySet` に対して順序が無い。Battle も同様に確認）。
* **計測**: garden の「1 判断あたりのフレーム数」（`Mind::asked_frame`、`frames_per_decision`、`watch_minds`、HUD）は読みが park しなくなるので測れない → **「1 判断あたりの命令数」に置き換える**（`ScriptStats::instructions` の差分）。`SHORTEST_SLEEP` は `restore_memory` の都合なので残す。
* **VM パネル**: `Waiting::Component`（`crates/rubevy-arena/src/inspect.rs:399-403`）は到達不能になるので削除、単体テスト `:755-767` も。docs（`garden.md:920-944` のスクショの文、`sabiruby-battle.md:525`）を直す。
* **selftest**: 両ゲーム ×3 で通ること。落ちたら閾値を動かさず報告（既定 10）。
* `docs/garden.md` の「The questions」「The components」、`docs/sabiruby-battle.md` の該当、`docs/plans/garden-plan.md` の状況表に 1 行。

### 3.5 書きの同期（S5、**後で判断**）

`&mut World` を貸す形。順序の意味論（フレーム内のタスク実行順で勝敗が決まる）とコンポーネントフック（`resource_scope` の間 `ScriptTask` の `on_remove` が空振りする。調査「引っかかりそうな点」3）を先に解く必要がある。S3 でゲームが動いてから、世界の脚本（`garden-world-plan.md` の書き直し）に本当に要るかで決める。

---

## 4. 段階

| 段階 | repo | 内容 | 確認 |
|---|---|---|---|
| S1 | rubevy | 排他 tick + 答えループ + `answer_components` の吸収 + キャッシュ + テスト + 計測（3.1、3.2） | `cargo test`、`cargo build --examples`、`cargo run --example components` / `headless` / `two_vms`、clippy 増減なし、**サンプルゲームは S1 では触らない**（`Cargo.lock` が古い rev を指すので影響なし） |
| S2 | rubevy | docs（3.3） | doctest、リンク切れなし |
| S3 | rubevy_games | 順序・計測・パネル・selftest（3.4） | 両ゲーム selftest ×3、窓ビルド、`web/build.sh garden` |
| S4 | rubevy | **ゲームが tick の中で答える口**（任意、後で判断）: `ScriptWorld::answer_in_tick(kind, Box<dyn Fn(&World, &Request) -> Answer + Send + Sync>)`。答えループが `reflect_requests` の次にこれを見る。`garden.nearest` / `count` がその場で返る。閉包は `&World` を引数で受けるだけなので unsafe は要らない | — |
| S5 | — | 書きの同期。**後で判断**（3.5） | — |

---

## 5. 分かっている罠（調査「引っかかりそうな点」から）

1. **別名**: 無い。World への参照はネイティブに渡らず、答えループは `&mut Vm` と `&World` を**同じ関数の中で別々の引数として**持つだけ。`resource_scope` が `ScriptWorld<M>` を World から抜いている（`bevy_ecs/src/world/mod.rs:2851-2940`）ので、`&World` から `Vm` に届く道も無い。
2. **tick の外**: `Startup` の `load_and_run`（`tests/vm_setup.rs` が実例）で読むと `Request` が積まれ、最初の tick の答えループで答える（既定 4）。例外にしない。
3. **構造変更**: 同期にしない。`resource_scope` の間はコンポーネントフックが `ScriptWorld` を見つけられず空振りする（`src/lib.rs:237-250`）ので、閉包の中で despawn / remove を**絶対にしない**。終了タスクの `ScriptDone` 挿入は閉包を抜けてから。
4. **2 本の VM**: tick が排他なので直列。`examples/two_vms.rs:77-78` と host-api の "Two VMs" に 1 行。
5. **順序の意味論**: 読みが「N の Tick 時点」になるので、ゲームのシステムの `.before(RubevySet::Tick)` が初めて意味を持つ（S3）。書きは据え置きなので wheel / last-writer-wins は変わらない。
6. **sabiruby の rev**: 触らない。S3 で games の `Cargo.lock` を rubevy の新しい rev に上げるだけ。
7. **答えループの止まり方**: `task_run_limits` は「実行できるタスクが無い」「予算切れ」「時間切れ」のどれでも戻る（`src/vm.rs:797-816`）。答えた質問が 0 なら、残り予算があっても tick を終える（誰も起きないので回しても無駄）。読みだけを繰り返すタスクも 1 周ごとに `ask` と `pop` の命令を使うので、予算で止まる。周回の上限は置かない。
8. **`Request` の寿命**: 今は `deliver_answers` が Deliver で `answering` を配る。答えループは `push_answer` を直接呼ぶので `answering` を通らない。`release_values`（Deliver）で解放される `Arg::Value` の道は変わらない — 答えループの中で `Request` を落とすと、その値の解放は次の Deliver。

---

## 6. 状況

| 段階 | 状態 |
|---|---|
| S1 | **済み** `d0e9b85`（2026-09-17）。tests 66 件（新 3 本 + 計測 2 本 `#[ignore]`）、examples 3 本同じ結果、clippy 増減なし、unsafe 0。**実測**: 読み 1 回（= 答えループ 1 周）2.2 µs。読みしかしないタスク 1 本は 1 フレームに 2,667 回読め、止めたのは `frame_time` でなく命令数の予算（1 読み ≈ 75 命令、6 ms しか使っていない）。24 タスク × 4 読み/フレームで 96 読みがフレームを 0.3 ms 伸ばす（前は 24 読みしかできない）。記録 `docs/worklog/2026-09-17-sync-reads.md` |
| S2 | **済み** `95328d6` + `129fb77`（2026-09-17）。host-api「A read costs no frame」、rust-bridge.ja、outlook 英日、README、`prelude.rb` の嘘 2 か所。副産物: `tools/compile_scripts.sh` は `-g` 無しなので `.mrb` にデバッグ情報が無い（`ScriptStats::location` がこれらの `.mrb` には効かない可能性、未確認） |
| S3 | **進行中**（rubevy_games ブランチ `sync-reads`）。計測の置き換え・パネル・docs は済み（`9465633` `5a726d7` `94d4802`）。**分かったこと**: 同期読みはゲーム無変更で何も壊さない（selftest 3 回全通過）。規則の chain は tick をまたいで割れ、割れ方がフレームごとに変わる。`.before(RubevySet::Tick)` を入れると selftest 6 番（触られた甲虫の向き）が 12 回中 6 回落ちる — 原因は `flee_from` が距離ほぼ 0 の位置差から向きを出すこと。**著者判断（2026-09-17）**: 順序を入れ、`flee_from` は距離が小さいとき相手の進行方向の反対に逃げる形に直す。閾値は動かさない |
| S4 | 後で判断 |
| S5 | 後で判断 |
