# rubevy: コンポーネントの同期読み（tick の中でその場で読む）— 実装指示書

作成 2026-09-17。著者「同期の読み書きを考えるべきタイミングでは」。調査は `docs/worklog/2026-09-17-sync-access-survey.md`（行番号つき。**着手前に全部読む**）。
対象: rubevy main `bfa47ed`、sabiruby main `8faf26f`（0.5.1。rubevy の `Cargo.lock` は 0.5.0 の `e2470626` を指している）、Bevy 0.19.1。

**この文書だけで着手できるように書いてある。** 読む順は 1 → 2 → 3 → 4。5 は着手前に必ず目を通す。

---

## 0. はじめの一歩

```bash
# sabiruby（段階 S0）
cd /home/kishima/book/kishima/sabiruby && git switch -c host-ref && cargo test && tools/check_no_std.sh
# rubevy（段階 S1 以降）
cd /home/kishima/book/kishima/rubevy && git worktree add -b sync-reads ../rubevy-wt-sync main && cd ../rubevy-wt-sync && cargo test
```

作法は `/home/kishima/book/.claude/agents/implementer.md`。段階ごとに 1 コミット、push しない、main に触らない、過程は worklog に書きながら進める（捨てた案と理由も）。決めきれない点は止まって報告する。

---

## 1. 何を作るのか（30 秒版）

今の `e[:Hunger]` は `Rubevy.ask("component.get")` の往復で、**読み 1 回に 1 フレーム**かかる（N の Tick で訊き、N の Answer で `answer_components` が答え、N+1 の Tick で起きる）。
生き物が 0.2 秒に 1 回判断する分には効かなかったが、「毎フレーム全員を見る」世界の脚本（`rubevy_games/docs/plans/garden-world-plan.md`）はこれに正面からぶつかる。

これを**その場で読む**形にする:

```ruby
hp = me[:Hunger]        # 今: 1 フレーム待つ。これから: その場で返る（このフレームの Tick 時点の値）
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
| 1 | World を VM に貸す口 | **sabiruby に足す**: `Vm::lend_host_ref::<T: Any>(&mut self, r: &T, f: impl FnOnce(&mut Vm) -> R) -> R` と `Vm::host_ref::<T: Any>(&self) -> Option<&T>`。`f` の間だけ `&T` を貸し、抜けたら（パニックでも）外す。**`unsafe` は sabiruby のこの 1 か所に閉じる**（ポインタの包みに `unsafe impl Send + Sync`、`host_ref` の deref）。rubevy は unsafe ゼロのまま（`docs/rust-bridge.ja.md` の「`unsafe` の数について」を守る）。代案 = rubevy に生ポインタの包みを置く（rubevy に初めて unsafe が入る）。sabiruby の作法「unsafe を増やさない」に反するので、**著者が違うと言えば代案に切り替える** |
| 2 | 貸すもの | `&World`（読み取り）。`&mut World` は貸さない |
| 3 | tick の形 | `tick_scripts::<M>` を排他システム（`fn(world: &mut World)`）に。`resource_scope` で `ScriptWorld<M>` を World から抜き、その閉包の中で `world.lend_host_ref(&*world, \|vm\| vm.task_run_limits(..))`。**`ScriptWorld<M>` が World に無い間しか貸さない**ので、ネイティブが World 経由で `Vm` に戻る道が無い（調査 §4）。2 本の VM の tick は直列になる（今は順序なし。`examples/two_vms.rs` に 1 行） |
| 4 | tick の外で同期の読みを呼んだとき | **例外**（`RuntimeError`、文言「component reads happen inside a frame; there is no world to read at Startup」）。`Startup` で `load_and_run` するスクリプト、`Task#terminate`、`unsubscribe` の `close` が該当（調査「引っかかりそうな点」2）。nil にはしない（静かな間違いになる） |
| 5 | 型名 → `ReflectComponent` の引き方 | 名前をキーにした**キャッシュ**を `HostState` に持つ（bevy 自身の助言 `bevy_ecs/src/reflect/component.rs:326-330`。`ReflectComponent` は clone が安い）。外れたら `AppTypeRegistry.read()` を引いて入れる。無い名前は毎回引く（後から登録される型のため） |
| 6 | `Rubevy.find` と `components` | 同期にするが、費用は今と同じ（全エンティティ / 全登録型の走査）。docs に「安くなったのはレイテンシで、走査は残る」と書く |
| 7 | `answer_components` と `RESERVED_KINDS` | **削除**。rubevy が自分で答える質問は無くなる。`reflect_requests` も消す |
| 8 | `Rubevy::Entity#[]` 等の実装 | `src/prelude.rb` の Ruby から **`define_fn` のネイティブ**に移す（park しないので Ruby で書く理由が消える。調査 §5）。`[]` がネイティブでも `OP_GETIDX` の send 経路は同じ |
| 9 | 書きの道 | 変えない（`Rubevy.set_component` → `component_writes` → `apply_component_writes` を Tick の末尾で）。読みが同期になっても「同じ tick で書いた値はまだ読めない」ことを docs に明記する |
| 10 | サンプルゲーム | rubevy 側の段階が終わってから（S3）。ゲームの selftest が閾値で落ちたら**閾値を動かす前に止まって報告** |

---

## 3. 設計

### 3.1 sabiruby（S0）

* `Vm` に `host_ref: HostRefSlot`（`TypeId` + 型消去したポインタ、`Option`）。`unsafe impl Send + Sync for HostRefSlot`（ポインタは `lend_host_ref` の閉包の中でしか有効でなく、その間 `&mut Vm` は閉包が独占している）。
* `lend_host_ref`: 入れる → `f(self)` → RAII ガードで抜く（`f` がパニックしても抜ける）。**入れ子は許さない**（既に貸している間に呼んだら `panic!`。仕様として書く）。
* `host_ref::<T>`: `TypeId` が一致すれば `Some(&T)`（`&self` の寿命に縛る）。不一致・未貸与は `None`。
* テスト: 貸している間だけ `Some`、閉包の後 `None`、`catch_unwind` でパニック後も `None`、`Vm: Send + Sync` が保たれる（`tests/send_sync.rs`）、`tools/check_no_std.sh`、本家テスト基準を下回らない。
* 版: `0.5.2`。`CHANGELOG` / `docs/` の該当（ホスト API の文書）に 1 節。

### 3.2 rubevy（S1）

* `tick_scripts::<M>(world: &mut World)`: `Time` / `FrameCount` は先に値を写す。タスクの列挙は `world.query_filtered::<(Entity, &ScriptTask<M>), Without<ScriptDone<M>>>()` で抜く前に集める。`resource_scope(|world, mut scripts: Mut<ScriptWorld<M>>| { ... scripts.vm.lend_host_ref(&*world, |vm| vm.task_run_limits(limits)) ... })`。終わったタスクの `ScriptEnded<M>` は `world.write_message`、`ScriptDone<M>` は `world.entity_mut(e).insert`。`Commands` は使わない。
* 同期ネイティブ（`src/lib.rs`、`define_fn`）: `component.get` / `has` / `components` / `find`。中身は今の `answer_components` の reflection をそのまま関数に切り出して呼ぶ（`reflect::reflect_to_ruby` は `&mut Vm` と `&dyn PartialReflect` だけで足りる）。`vm.host_ref::<World>()` が `None` なら既定 4 の例外。
* `HostState` に `reflect_cache: HashMap<String, ReflectComponent>`（既定 5）。
* 消すもの: `answer_components`、`RESERVED_KINDS`、`reflect_requests`、`src/prelude.rb` の該当 Ruby（`Rubevy::Entity#get` / `[]` / `has?` / `components`、`Rubevy.find`）とそのコメント（`:5-11`「park はネイティブの中ではできない」の理由は書きには残る）。
* テスト: `tests/scheduling.rs:83-104` の期待値 `[1,1,1,1,1,1]` → `[0,0,0,0,0,0]`。`tests/components.rs:215-236`（`rubevys_own_questions_never_reach_the_game`）は削除（理由をコミットに）。`tests/components.rs:109-132` の見出し（書きは末尾反映のまま、なので **変えない**、コメントだけ直す）。新テスト: 同じ tick の中で読み → 書き → 読み で古い値が返ること（既定 9 を固定する）、tick の外（`Startup` の `load_and_run`）で読むと例外、2 本の VM でそれぞれ自分の World の読みが通ること（`tests/two_vms.rs` に 1 本）。`tests/entity.rs` / `tests/proxy.rs` は無変更で通ること。
* `Cargo.lock`: sabiruby を main（S0 の後）に上げる（`cargo update -p sabiruby`）。

### 3.3 docs（S2）

`docs/host-api.md:329-362`（A read costs a frame / Why a read can wait inside `[]` — 理由ごと書き換え）、`:95-137`（RubevySet の表: Tick が排他で直列点になる）、`:583-586`（「ネイティブから World に触るな」の規則 → 「読みだけ、tick の間だけ、貸された `&World` を」）。`src/lib.rs:25-28`（crate doc）、`:682-700`、`:1646-1669`、`:1773-1777`。`docs/rust-bridge.ja.md:344-345,353` と「`unsafe` の数について」（sabiruby に 1 つ入った旨）。`docs/outlook.md:63-65,84-92,150-153`、`docs/outlook.ja.md:208-216`（`:211`「読み取りはその場の値が返ります」が本当になった）。`docs/plans/ecs-bridge-plan.md:5` の「遅さが問題になったときに足す」に「足した」と 1 行。`docs/README.md` の目次。

### 3.4 サンプルゲーム（S3、`rubevy_games`）

* `Cargo.lock` を rubevy / sabiruby の新しい rev に。
* **順序**: 読みが「N の Tick 時点」を見るので、スクリプトが読むコンポーネントを書くシステムは **`.before(RubevySet::Tick)`** に置く（garden の規則 chain `main.rs:1301-1325` は今 `RubevySet` に対して順序が無い。Battle も同様に確認）。
* **計測**: garden の「1 判断あたりのフレーム数」（`Mind::asked_frame`、`frames_per_decision`、`watch_minds`、HUD）は読みが park しなくなるので測れない → **「1 判断あたりの命令数」に置き換える**（`ScriptStats::instructions` の差分）。`SHORTEST_SLEEP` は `restore_memory` の都合なので残す。
* **VM パネル**: `Waiting::Component`（`crates/rubevy-arena/src/inspect.rs:399-403`）は到達不能になるので削除、単体テスト `:755-767` も。docs（`garden.md:920-944` のスクショの文、`sabiruby-battle.md:525`）を直す。
* **selftest**: 両ゲーム ×3 で通ること。落ちたら閾値を動かさず報告（既定 10）。
* `docs/garden.md` の「The questions」「The components」、`docs/sabiruby-battle.md` の該当、`docs/plans/garden-plan.md` の状況表に 1 行。

### 3.5 書きの同期（S4、**後で判断**）

`&mut World` を貸す形。順序の意味論（フレーム内のタスク実行順で勝敗が決まる）とコンポーネントフック（`resource_scope` の間 `ScriptTask` の `on_remove` が空振りする。調査「引っかかりそうな点」3）を先に解く必要がある。S3 でゲームが動いてから、世界の脚本（`garden-world-plan.md` の書き直し）に本当に要るかで決める。

---

## 4. 段階

| 段階 | repo | 内容 | 確認 |
|---|---|---|---|
| S0 | sabiruby | `lend_host_ref` / `host_ref`（3.1） | `cargo test`、`tests/send_sync.rs`、`tools/check_no_std.sh`、本家テスト基準（`tests/mrbtest/baseline*.txt`）、`tools/bench.sh` で性能がぶれの範囲 |
| S1 | rubevy | 排他 tick + `&World` の貸し出し + 同期ネイティブ 4 種 + `answer_components` の削除 + テスト（3.2） | `cargo test`、`cargo build --examples`、`cargo run --example components` / `headless` / `two_vms`、clippy 増減なし、**サンプルゲームは S1 では触らない**（`Cargo.lock` が古い rev を指すので影響なし） |
| S2 | rubevy | docs（3.3） | doctest、リンク切れなし |
| S3 | rubevy_games | 順序・計測・パネル・selftest（3.4） | 両ゲーム selftest ×3、窓ビルド、`web/build.sh garden` |
| S4 | — | 書きの同期。**後で判断** | — |

---

## 5. 分かっている罠（調査「引っかかりそうな点」から）

1. **別名**: 安全の根拠は 2 つ。`resource_scope` が `ScriptWorld<M>` を World から**本当に抜く**（`bevy_ecs/src/world/mod.rs:2851-2940`）ので閉包の中で World から `Vm` へ戻れないこと、貸すのが `&World` だけなこと。貸し出しは RAII でパニック時も外す。
2. **tick の外**: `Startup` の `load_and_run`（`tests/vm_setup.rs` が実例）、`stop_task` の `funcall(:terminate)`、`unsubscribe` の `funcall(:close)`。同期ネイティブは貸されていない状態で呼ばれうる → 例外（既定 4）。
3. **構造変更**: 同期にしない。`resource_scope` の間はコンポーネントフックが `ScriptWorld` を見つけられず空振りする（`src/lib.rs:237-250`）ので、閉包の中で despawn / remove を**絶対にしない**。終了タスクの `ScriptDone` 挿入は閉包を抜けてから。
4. **2 本の VM**: tick が排他なので直列。`examples/two_vms.rs:77-78` と host-api の "Two VMs" に 1 行。
5. **順序の意味論**: 読みが「N の Tick 時点」になるので、ゲームのシステムの `.before(RubevySet::Tick)` が初めて意味を持つ（S3）。書きは据え置きなので wheel / last-writer-wins は変わらない。
6. **sabiruby の rev**: rubevy と rubevy_games の `Cargo.lock` は 0.5.0 を指している。S1 で rubevy を、S3 で games を、同じ rev に上げる（`rubevy_games/Cargo.toml:27` のコメント: 同じソースであること）。
7. **`Vm: Send + Sync`**: `HostRefSlot` の `unsafe impl` で保つ。`tests/send_sync.rs` が守る。

---

## 6. 状況

| 段階 | 状態 |
|---|---|
| S0 | 未着手 |
| S1 | 未着手 |
| S2 | 未着手 |
| S3 | 未着手 |
| S4 | 後で判断 |
