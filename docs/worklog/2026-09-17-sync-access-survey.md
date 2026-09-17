# 2026-09-17 同期アクセスのための調査（コードは 1 行も変えていない）

スクリプトからのコンポーネントの読み書きを**同期**（tick の最中にその場で読み書きする）にする計画を
書くための事実集め。rubevy main `bfa47ed`、sabiruby main `8faf26f`（`Cargo.lock` が固定しているのは
git rev `e2470626` = 0.5.0 なので、**checkout の 0.5.1 と rubevy がビルドしている VM は同じではない**）、
rubevy_games main `7f22fb1`、bevy 0.19.1。

調査だけなので実装も計測用のテストも足していない。**［確認］はファイルを読んで確かめたこと、
［推測］は根拠を添えた推論**として分けて書く。行番号はすべて調査時点のもの。

---

## 1. tick の形

### 引数と本体［確認］

`src/lib.rs:1486-1541`。

```rust
fn tick_scripts<M: 'static>(
    time: Res<Time>,                                                   // :1487
    frame: Res<FrameCount>,                                            // :1488
    mut world: ResMut<ScriptWorld<M>>,                                 // :1489
    tasks: Query<(Entity, &ScriptTask<M>), Without<ScriptDone<M>>>,    // :1490
    mut ended: MessageWriter<ScriptEnded<M>>,                          // :1491
    mut commands: Commands,                                            // :1492
)
```

つまり**今の tick は排他システムではない**（`&mut World` を取らない普通のシステム）。
排他なのは `answer_components`（`src/lib.rs:1670`）と `apply_component_writes`（`:1778`）の 2 本だけ。

本体の順序:

| 行 | 何をするか |
|---|---|
| `:1502-1509` | `budget > 0` のときだけ、フレーム時間をティックに直して `task_advance_ticks`。端数は `tick_remainder` に残す。**`budget == 0` は「スクリプトにとって起こらなかったフレーム」**（`ScriptWorld::budget` の rustdoc `:682-700`）。一時停止はフラグではなく `budget = 0` の 1 本道 |
| `:1512` | `set_frame_state`（定義 `:1544-1556`）— `$rubevy` を `hash_new` で毎フレーム作り直し `global_set` |
| `:1513-1518` | `RunLimits { instructions: Some(budget), time_ns: frame_time, overrun_ns: overrun }` |
| `:1519-1523` | **`world.vm.task_run_limits(limits)`** ← VM を回す唯一の呼び出し。`run_once` ではない |
| `:1524` | `flush_output`（`:1557-1566`）— `take_output` を Bevy のログへ |
| `:1526-1541` | `tasks` を舐めて終わったタスクを回収: `ScriptEnded` を write、`gc_unregister`、`unsubscribe`、`ScriptDone` を insert |

予算のフィールドは `ScriptWorld` の `budget: u64`（`:701`、既定 200,000）、
`frame_time: Option<Duration>`（`:704`、既定 8 ms）、`overrun: Option<Duration>`（`:708`、既定 50 ms）。
`task_run_limits` の実体は sabiruby `src/vm.rs:780-816`（`task_run_limited` のループが `:798-816`）。
予算・時間の判定はループの頭で行われるので、**`budget == 0` なら 1 命令も走らない**（`src/vm.rs:800`）。

### tick の中でネイティブが手にするもの［確認］

**`&mut Vm` だけ**。sabiruby のネイティブの型は
`Fn(&mut Vm, Value, &[Value], Value) -> VmResult<Value> + Send + Sync + 'static`
（`sabiruby/src/vm.rs:1343-1348`、`sabiruby/src/object.rs:18,31`）。
`&mut Vm` の外に触る口は 1 つだけで、

* `Vm::set_host_state` / `host_state::<T>()` / `host_state_mut::<T>()`
  （`sabiruby/src/vm.rs:1377,1381,1385`）。中身は
  **`host_state: Option<Box<dyn core::any::Any + Send + Sync>>`（`sabiruby/src/vm.rs:516`）で、VM ごとに 1 つだけ**。

rubevy がそこに入れているのは `HostState`（`src/lib.rs:1139-1150`）で、中身は 2 つ:

* `commands: Vec<HostCommand>` — ネイティブが書き、`drain_commands` が読むコマンドキュー
  （`push_command` `:1160`、`take_commands` `:1166`。`HostCommand` は `:362-376`）
* `subscriptions: Vec<Subscription>` — `Rubevy.subscribe` の一覧（`Subscription` は `:1152-1158`）

他に VM が持つホスト機構は `set_on_free`（1 本だけ。rubevy は `Rubevy::Entity` の数え上げに使用、`:766-771`）と
`HostStore`（型ごと。`TypeId` で引くのでゲームの型と衝突しない — `sabiruby/src/host_store.rs:21-28`、
経緯は `docs/worklog/2026-09-17-scheduling-and-vm-access.md` の §4）。

**ここが同期化の一番の壁**［確認］: `host_state` は `Any + Send + Sync + 'static` を要求するので、
`&mut World`（`'static` でも `Sync` でもない）をそのまま入れることはできない。

---

## 2. `component.get` / `set` / `has` / `components` の今の道

### 読み（`e[:Hunger]`）［確認］

1. **Ruby**: `Rubevy::Entity#[]`（`src/prelude.rb:28-30`）→ `#get`（`:18-20`）→
   `Rubevy.ask("component.get", self, name.to_s).pop`
   `has?` は `:46-48`（kind `component.has`）、`components` は `:51-53`（kind `components`）、
   `Rubevy.find` は `:58-60`（kind `entities.with`）。
   **Ruby で書いてある理由**は `src/prelude.rb:5-11` に書かれている: どれも答えを待つ＝タスクを park する必要があり、
   park はネイティブの中でできない（sabiruby `src/builtins/ext_task.rs:934`
   「blocking pop cannot be called from within a C function boundary」）。
2. **ネイティブ**: `Rubevy.ask`（`src/lib.rs:1914-1953`）。引数を `Arg` に落とし（`Arg::Entity` / `Text` / `Num` /
   `Value`）、`vm.task_queue_new()`（`:1947`）＋`gc_register`（`:1948`）で**質問 1 回につき Queue オブジェクトを 1 個作り**、
   `HostCommand::Ask { entity, kind, args, queue }` をキューに置いて（`:1950`）Queue を返す。
   park は Ruby 側の `.pop` で起きる。
3. **`drain_commands`**（`:1596-1638`、`RubevySet::Tick` の 2 本目）。`Ask` を `Request` にし、
   kind が `RESERVED_KINDS`（`:1652` = `["component.get", "component.has", "components", "entities.with"]`）なら
   `world.reflect_requests`（フィールドは `:717`）へ、そうでなければ `world.requests`（`:712`）へ。
   → **rubevy 自身が答える 4 種はゲームの `take_requests` には決して現れない**。
4. **`answer_components`**（`:1670-1776`、排他システム、`RubevySet::Answer`）。
   `AppTypeRegistry` を `cloned()` して `read()`（`:1675-1681`）、
   `world.resource_scope::<ScriptWorld<M>, _>`（`:1682`）で `ScriptWorld` を World から**抜き出し**、
   `named(i)` が `registry.get_with_short_type_path(name).or_else(|| get_with_type_path(name))`（`:1688-1693`）。
   * `component.get`（`:1695-1712`）: `r.data::<ReflectComponent>()` → `world.get_entity(e)` → `rc.reflect(entity)` →
     `reflect::reflect_to_ruby` を `scripts.answer_value` の閉包の中で呼ぶ
   * `component.has`（`:1713-1720`）: `rc.contains(entity)` → `Answer::Bool`
   * `components`（`:1721-1744`）: **型登録を全部舐めて** `rc.contains(entity)` を試し、短い型名を集めてソート
   * `entities.with`（`:1745-1762`）: **`world.iter_entities()` で世界の全エンティティを舐めて** `rc.contains`
5. `answer_value` / `answer`（`:857`,`:887`）→ `push_answer`（`:892-900`）が `task_queue_push` と
   `gc_unregister(request.queue)`。**Queue はここで手放されるので、1 つの Request に答えるのは 1 回だけ**。
6. 次のフレームの `tick_scripts` でタスクが起きる。

### 書き（`e[:Hp] = {...}`）［確認］

`src/prelude.rb:41-43` → `#set`（`:36-39`）→ `Rubevy.set_component` ネイティブ（`src/lib.rs:1957-1966`）。
ここで **`reflect::read_ruby` が Ruby の値を Rust の `RubyData` に写し取る**（`:1963`。`&mut Vm` があるのはここだけ、
という理由が `src/reflect.rs:52-59` に書いてある）。`HostCommand::SetComponent` →
`drain_commands`（`:1612-1616`）で `ComponentWrite`（`:378-382`）→
`apply_component_writes`（`:1778-1820`、排他、`RubevySet::Tick` の 3 本目）が
`rc.reflect_mut(&mut entity)` ＋ `reflect::apply_ruby`。**書きは待たない**（質問ではなく命令）。

### 往復が何フレームか［確認］

`RubevySet` は `src/lib.rs:1261-1300`、並べているのは `:1413-1417`（`Deliver → Tick → Answer` を `chain()`）、
中身は `:1418-1433`。

| フレーム | 何が起きるか |
|---|---|
| N・Tick | `tick_scripts` でスクリプトが `Rubevy.ask` → コマンド。`drain_commands` が `Request` に。`apply_component_writes` がこのフレームの書きを適用 |
| N・Answer | `answer_components` が答えを Queue に push → タスクが ready |
| N+1・Tick | スクリプトが起きて答えを受け取る |

= **1 フレーム**。`tests/scheduling.rs:83-104`
（`a_component_read_still_costs_one_frame`、期待値 `[1,1,1,1,1,1]`）が測っている。
`answer_components` は元はフレームの先頭にいて「N-1 の質問を N の頭で答える」形だったが、
`RubevySet` 導入で末尾に移った。**起きるフレームは同じ**で、副作用として
「同じフレームの中で書いてから読むと書いた値が読める」が構造として閉じた
（`docs/worklog/2026-09-17-scheduling-and-vm-access.md` §1）。

### `reflect.rs` の役割［確認］

型ごとの接着コードは 1 行も無く、`ReflectComponent` と型登録が言うことをそのまま歩く。

| 関数 | 行 | 要るもの |
|---|---|---|
| `read_ruby(vm, Value, entity_tag) -> RubyData` | `src/reflect.rs:101` | **`&mut Vm`**（World は不要） |
| `reflect_to_ruby(vm, &dyn PartialReflect, entity_class, entity_tag) -> Value` | `:158` | **`&mut Vm` と `&dyn PartialReflect`**（World そのものは不要） |
| `apply_ruby(&mut dyn PartialReflect, &RubyData)` | `:314` | **`&mut dyn PartialReflect`**（Vm は不要） |

`RubyData`（`:61-74`）は「`&mut Vm` のあるネイティブと `&mut World` のあるシステムが別の時刻にいる」から要る中間表現。
`MAX_DEPTH = 16`（`:41`）、`AS_ARRAY`（`:50`、glam の 5 型を Array で見せる）。

---

## 3. 同期にする候補の全部

| 口 | 今どこで答えているか | 同期にできるか |
|---|---|---|
| `e[:X]` (`component.get`) | `answer_components` `src/lib.rs:1695-1712`（排他、Answer） | **できる**。必要なのは `&World`（読み取りだけ）と型登録。`reflect_to_ruby` はネイティブの `&mut Vm` でそのまま呼べる |
| `e.has?` (`component.has`) | `:1713-1720` | **できる**。`&World` だけ |
| `e.components` | `:1721-1744` | できるが**重い**（型登録を全部舐める。§6） |
| `Rubevy.find` (`entities.with`) | `:1745-1762` | できるが**重い**（`world.iter_entities()` で全エンティティ。rustdoc `:1751-1752` 自身が「毎フレーム向きではない」と書いている）。同期にすると「安いから毎フレーム呼ぶ」誘惑が生まれる方が危ない |
| `e[:X] = h` (`set_component`) | `apply_component_writes` `:1778-1820`（排他、Tick 末尾） | **できる**が `&mut World` が要る。`read_ruby` → `apply_ruby` を 1 つのネイティブでつなげば `RubyData` は残しても消しても良い。ただし**「書きは `Commands` と同じ約束」という今の意味論が変わる**（§7 C） |
| `Rubevy.entity` | ネイティブ `:1899-1904`。`current_entity`（`:2019-2024`）が走っているタスクの `@rubevy_entity` ivar を読むだけ | **すでに同期**。往復なし |
| `$rubevy[:frame]` / `[:delta]` / `[:time]` | `set_frame_state` `:1544-1556` が毎フレーム tick の頭で作る Hash | **すでに同期**。往復なし |
| `Rubevy.log` / `set_position` / `move_to` | コマンドキュー（`:1874`,`:2009`,`:1905`） | できる。ただし `set_position` / `move_to` は `Transform` への書きなので上と同じ話 |
| `Rubevy.spawn` / `despawn` | コマンドキュー → `drain_commands` `:1620-1631` が `Commands` 経由（`commands.spawn` / `commands.entity(e).despawn()`） | **すべきでない、少なくとも最後に**。構造変更。`&mut World` があれば直接できてしまうが、`ScriptTask` の `on_remove` フック（`:237-250`）が `resource_scope` の最中は `ScriptWorld` を見つけられず**黙って何もしない**（`:246` の `else { return }`）ので、タスクと購読が漏れる |
| `Rubevy::Proxy` | `assets/scripts/proxy.rb:19-21` → `Rubevy.ask` → ゲームの answering システム | **同期にできない**（答えるのはゲームのシステムで、tick の外にいる）。park は残る |
| `Rubevy.ask` 一般 | `drain_commands` → `take_requests` → ゲーム | 同上 |
| `answer_with`（Future） | `:932-941`、`deliver_answers` `:1576-1594` | 本質的に非同期。park は残る |
| `Rubevy.subscribe` | `:1970-2007`。Queue を作るところは同期、`pop` で待つ | 変える必要なし |

つまり**同期にできるのは rubevy が自分で答えている 4 種＋書きだけ**で、ゲームが答える `Rubevy.ask` は今のままになる。
ここが計画の分かれ目で、「park はなくならない、減るだけ」。

---

## 4. `&mut World` を tick に渡す形の検討材料

### 使える道具［確認、bevy 0.19.1 のソースを読んだ］

* `World::resource_scope<R, U>(&mut self, f: impl FnOnce(&mut World, Mut<R>) -> U) -> U`
  — `bevy_ecs-0.19.1/src/world/mod.rs:2851`。実体は `try_resource_scope`（`:2869-`）で、
  **`entity_mut.take::<R>()` でリソースを本当に World から抜き、`ReinsertGuard` の `Drop` で戻す**（`:2882-2940`）。
  → **閉包の間、`ScriptWorld<M>` は World の中に無い**。これが安全側の一番強い材料で、
  ネイティブが World 経由で `ScriptWorld`（＝`Vm`）にもう一度到達する道が**構造的に無い**ことを意味する。
  逆に言えば、閉包の中で `ScriptWorld` を要求するもの（コンポーネントフック、`answer_components`）は全部空振りする。
* `World::as_unsafe_world_cell(&mut self) -> UnsafeWorldCell<'_>`（`:204`）、
  `UnsafeWorldCell::world_mut`（`unsafe_world_cell.rs:196`）、`::get_entity -> UnsafeEntityCell`（`:386`）。
* `ReflectComponent`（`bevy_ecs-0.19.1/src/reflect/component.rs`）:
  * `reflect<'w,'s>(&self, entity: impl Into<FilteredEntityRef<'w,'s>>) -> Option<&'w dyn Reflect>` — `:205`。**`&World` で足りる**
  * `contains(&self, entity: impl Into<FilteredEntityRef>) -> bool` — `:199`
  * `reflect_mut<'w,'s>(&self, entity: impl Into<FilteredEntityMut<'w,'s>>) -> Option<Mut<'w, dyn Reflect>>` — `:212`。**`&mut World` が要る**
  * `unsafe reflect_unchecked_mut(&self, entity: UnsafeEntityCell) -> Option<Mut<dyn Reflect>>` — `:224`。
    安全条件は「同じスコープで同じコンポーネントに 2 回呼ばない」
* `AppTypeRegistry(pub TypeRegistryArc)` は `#[derive(Resource, Clone)]`（`bevy_ecs/src/reflect/mod.rs:35-36`）で、
  中身は `Arc<RwLock<TypeRegistry>>`（`bevy_reflect-0.19.1/src/type_registry.rs:42-45`）。
  **`clone()` は Arc のクローンなので安い**。今のコードもそうしている（`src/lib.rs:1675,1681`）。

### 再利用できる関数と、要る参照［確認］

| 呼びたいもの | `&World` で足りるか |
|---|---|
| `reflect::reflect_to_ruby`（`src/reflect.rs:158`） | **World は要らない**。`&dyn PartialReflect` と `&mut Vm` だけ。`&World` から `rc.reflect(entity)` で参照を取れば、そのままネイティブから呼べる |
| `reflect::read_ruby`（`:101`） | World 不要。`&mut Vm` だけ |
| `reflect::apply_ruby`（`:314`） | `&mut dyn PartialReflect` が要る ⇒ `rc.reflect_mut` ⇒ **`&mut World`** |
| `answer_components` の `named()`（`src/lib.rs:1688-1693`） | 型登録の読みロックだけ。World 不要 |

→ **読み・has・components・find は `&World` で足り、書きだけが `&mut World` を要求する。**

### 形の候補［推測］

1. **`tick_scripts` を排他システムにする**（`fn tick_scripts<M>(world: &mut World)`）。
   `resource_scope::<ScriptWorld<M>>` で VM を抜き、World の生ポインタを `HostState` に置いて
   `task_run_limits` を回し、終わったら外す（RAII で必ず外す）。
   *要るもの*: `host_state` が `Send + Sync + 'static` を要求するので、
   `struct WorldLease(*mut World)` に `unsafe impl Send + Sync` を付けた包みが要る。**`unsafe` が rubevy に初めて入る**
   （今は 0 か所。`docs/rust-bridge.ja.md` の「`unsafe` の数について」の節が主張していることが変わる）。
   *失うもの*: `Res<Time>` / `Res<FrameCount>` / `Query` / `MessageWriter` / `Commands` が使えなくなる
   （`world.write_message::<ScriptEnded<M>>`（`bevy_ecs/src/world/mod.rs:3015`）や
   `SystemState`（`bevy_ecs/src/system/function_system.rs:239`）で代替は効く）。
   *もう 1 つ失うもの*: 排他システムは他のシステムと並列に走れないので、**`RubevySet::Tick` がフレームの直列点になる**。
   VM が 2 本なら 2 本の tick が必ず直列化する（今は `answer_components` と `apply_component_writes` だけが排他）。
2. **sabiruby 側に「一時的なホスト参照」の口を足す**。
   たとえば `vm.with_host_ref(&mut dyn Any, |vm| …)` のような、`'static` を要求しないスコープ付きの置き場。
   VM の `host_state` が 1 つしか持てない（`sabiruby/src/host_store.rs:21-28` がまさにこの制約を書いている）ので
   別枠になる。*要るもの*: sabiruby への API 追加と、その中の `unsafe` は VM 側に寄る。
3. **読みだけ同期にする**（`&World` で足りる範囲）。書きは今のまま `apply_component_writes` に残す。
   *利点*: 別名の問題が「共有参照の寿命」だけになり、構造変更・フック・`Commands` の話が一切出てこない。
   *欠点*: 「書いた直後に読むと古い値」が残る（今は書きも読みも遅れているので順序が揃っていた）。

---

## 5. SabiRuby 側

* **ネイティブが VM の外に触る口**: `set_host_state` / `host_state::<T>()` / `host_state_mut::<T>()`
  （`sabiruby/src/vm.rs:1377,1381,1385`、フィールドは `:516`）。**VM ごとに 1 つ**、`Any + Send + Sync + 'static`。
  型ごとの置き場は `HostStore`（`sabiruby/src/host_store.rs`）で、`Vm::install_host_store` / `host_store`。
  `Host` トレイト（`sabiruby/src/host.rs:39`）は `compile` / `read_file` / `file_exists` だけで、`Send + Sync`。
* **park の仕組み**: `Task::Queue#pop` が `sabiruby/src/builtins/ext_task.rs:920-948`。
  待つタスクは `REASON_QUEUE` / `WAITING` にされ `vm.task.switching = true`（`:944`）。
* **C 関数の境界**: 3 か所で拒否される。
  * `Queue#pop`（ブロッキング）: `ext_task.rs:934`
  * `Task.pass`: `:702`
  * `Kernel#sleep`（引数なし）: `:1143`
  判定は `vm.fiber_check_native(vm.cur)`。設計の説明は `sabiruby/docs/design/fibers.md:45-71`
  （`OP_GETIDX` が Array/Hash/String 以外を**呼び出し元のフレームで send する**ので、Ruby で書いた `[]` の中で park できる、
  という今の `e[:X]` が成り立っている根拠が `:67-71`）。
  rubevy 側の対応する記述は `src/prelude.rb:5-11` と `docs/host-api.md:355-362`。
* **同期にすると park が要らなくなるところ**［確認＋推測］:
  `component.get` / `component.has` / `components` / `entities.with` の 4 種。
  これらが同期になると `.pop` が消え、**この 4 つを Ruby で書いていた理由（park できるフレームが要る）が消える**ので、
  `Rubevy::Entity#[]` / `#get` / `#has?` / `#components` / `Rubevy.find` は
  `Vm::define_fn` のネイティブにできる。`OP_GETIDX` の send は `[]` がネイティブでも同じように働く
  （`Rubevy::Entity` は Array/Hash/String ではないので send 経路、`fibers.md:67-71`）。
  park が残るのは `Rubevy.ask`（ゲームが答える）、`Rubevy::Proxy`、`answer_with`、`subscribe` の `pop`、`sleep`。
* **`task_run_once` / `task_run_limits` の実体**: `sabiruby/src/vm.rs:766`（`run_once`、rubevy は使っていない）と
  `:780-816`（`task_run_limits` → `task_run_limited`）。
  rubevy が呼ぶのは `task_run_limits` の方 1 本だけ（`src/lib.rs:1519`）。
  **`&mut Vm` はこの呼び出しの間ずっと排他に借りられている**（`&mut self`）ので、
  ネイティブが World に触るなら `Vm` を経由しない別の道（生ポインタ or 新 API）でなければならない。

---

## 6. 性能の目安

### 今の 1 件あたり［確認：コードを読んで数えたもの］

`answer_components` で `e[:Transform]` 1 件が通る道:

1. `AppTypeRegistry` の `clone()`（Arc 1 本）＋ `read()`（`RwLock` の read guard）— **1 フレームに 1 回**（`src/lib.rs:1675,1681`）、
   1 件あたりではない
2. 型名 → `TypeId`: `get_with_short_type_path`（`bevy_reflect/src/type_registry.rs:467`）= ハッシュ 1 回、
   外れたら `get_with_type_path`（`:441`）でもう 1 回
3. `TypeId` → `TypeRegistration`: ハッシュ 1 回（`:423`）
4. `TypeRegistration::data::<ReflectComponent>()`: ハッシュ 1 回 ＋ ダウンキャスト（`:677-681`）
5. `world.get_entity(e)` → `rc.reflect(entity)`（関数ポインタ 1 段 → `entity.get::<C>()`）
6. `reflect_to_ruby` で Ruby の値を組む。`Transform` なら **Ruby ヒープの確保は 4 個**（Hash 1 ＋ Array 3）だけで、
   数値は `Value::Float(f64)` の即値（`sabiruby/src/value.rs:13-21`）なので確保しない。Symbol 3 個は intern（既存なら引くだけ）

**bevy 自身が 2〜4 について書いている**［確認］: `bevy_ecs/src/reflect/component.rs:326-330`
「`TypeRegistry::get` に続く `TypeRegistration::data::<ReflectComponent>` は 1 フレームに何度もやると高くつきうる。
`ReflectComponent` を clone して frame 間で保持することを考えよ。clone はとても安い」。
→ **名前 → `ReflectComponent` のキャッシュを持てば 2〜4 はハッシュ 1 回に落ちる**、というのが公式の助言。

これに今は**質問 1 回あたり**次が乗る:

* `vm.task_queue_new()` で Ruby の Queue オブジェクト 1 個 ＋ `gc_register` / `gc_unregister` の対
  （`src/lib.rs:1947-1948`、`:899`）
* `Request` の `String kind`（`"component.get"`）と `Arg::Text`（型名）の **Rust の String 2 本**、`Vec<Arg>` 1 本
  （`:1917-1946`）。`HostCommand` から `Request` への move が 1 回（`:1606`）
* タスクの park と wake（`ext_task.rs:920-948`）とスケジューラの往復
* **そして 1 フレーム（60 fps なら 16.7 ms）の待ち**

### 同期にしたときの 1 アクセス［推測］

やることは「キャッシュ引き 1 回 → `rc.reflect` → `reflect_to_ruby`」だけになる。
消えるのは Queue オブジェクト・String 2 本・`Vec`・gc 登録解除・park/wake・1 フレーム。
**桁で言えば、1 件の費用は µs 未満のまま（Ruby ヒープ 4 個の確保が支配的）で変わらないが、
遅延が 16.7 ms → 0 になる**。つまり効くのはスループットではなくレイテンシで、
「1 フレームに 1 回しか読まない」という書き方の制約が外れることが本体。

**測っていない。** 指示が「コードを 1 行も変えるな」なので、
`tests/components.rs` を元にした計測用テストを足すこと自体を避けた。数字が要るなら
`tests/components.rs:80-105`（`a_script_reads_a_transform_as_a_hash`）の形で
「N 回読んでフレーム数と `ScriptStats::instructions` を比べる」テストを 1 本足すのが最小。

### 重いのは読みではない［確認］

* `components`（`src/lib.rs:1721-1744`）は**型登録を全部舐めて** `rc.contains` を試す。
  `DefaultPlugins` の登録数は数百なので、1 回で数百の関数ポインタ呼び出し。
* `entities.with`（`:1745-1762`）は `world.iter_entities()`（`bevy_ecs/src/world/mod.rs:1010-1027`、
  全アーキタイプを歩く。**0.19 ではリソースもエンティティなのでそれも含む**）× `rc.contains`。
* 同期にするとこの 2 つが「安く見える」ようになるので、**呼ばれ方が変わって遅くなる**のが現実的な危険。

---

## 7. サンプルゲームへの影響

rubevy_games main `7f22fb1`。以下はサンプル側を読んだ結果（行番号はそちらのリポジトリ）。

### A. 「1 判断あたりのフレーム数」の計測が測れなくなる［確認］

garden は**タスクが park して空白ができる**ことで往復を数えている。

* `garden/src/main.rs:464-475` — `ran_frame` / `asked_frame` / `ask_trips` / `ask_frames` / `read_trips` / `read_frames`。
  `:466-470` のコメントが「そのフレームから始まる空白は `Rubevy.ask` の往復、そうでない短い空白はコンポーネント読み」
* `:493-496` `frames_per_decision()`
* `:3121-3203` `watch_minds`（登録は `:1351`、`.after(RubevySet::Answer)`）。
  `:3160-3172` が判定本体で、`mind.asked_frame == Some(was)` なら ask、`gap < sleep_floor` なら read
* `:3113-3119` `SHORTEST_SLEEP`（`ruby/` の中で一番短い `sleep`。`sleep_floor` は `:3152` で毎フレーム再計算）
* 表示: `garden/src/window.rs:496`、`:595`、`:635-647`（`f/dec` の列と hover）、`:966-977`（窓ありセルフテスト）、
  `garden/src/main.rs:3970-3982`（headless の総計）

**同期にすると `read_trips` は永久に 0 になり、`gap < sleep_floor` は
残った短い空白（タイムスライス切れなど）を読みと誤認する。** `watch_minds` は
`mind.at` / `own_line` / `heat`（`:3189-3202`）と `spent` / `frames`（`:3157-3159`）も作っているので、
関数ごと消すことはできず、往復の勘定だけを抜く形になる。

### B. VM パネルの「待っている理由」が 1 種類到達不能になる［確認］

`crates/rubevy-arena/src/inspect.rs`:

* `:83-94` `Waiting` のドキュメント（「`Rubevy::Entity` はコンポーネント読み」）
* `:96-113` `enum Waiting { Nothing, Event, Ask, Component, Queue, Sleep }`
* `:115-131` `text()`（`:123-126` が `"waiting for a component read — [{name}]"`）
* `:133-142` `how()`（`:138` が Component の根拠文、`:137` が Ask の「答えは次のフレーム」）
* **`:383-431` `fn why(frames)`。`:399-403` が `if f.class == "Rubevy::Entity"` → `Waiting::Component`**
* `:289-290` 呼び出し、`:344-345` headless 出力
* `:755-767` 単体テスト `a_component_read_names_the_component`（`Task::Queue#pop` / `Rubevy::Entity#get` /
  `[]` / `Creature#hunger` の 4 段スタックを仕様として固定している）

同期読みではこのスタックがそもそも積まれないので、`Waiting::Component` は到達不能、`:755-767` は仕様として死ぬ。
文書も連動: `rubevy_games/docs/garden.md:920-944`（スクショ `docs/garden-vm.png` 付き）、`:1017`、
`docs/sabiruby-battle.md:525`、`docs/worklog/2026-09-17-garden-G9.md:129,137,161,180,222`
（wasm に含まれる文字列の本数まで数えている）。

### C. 「書きは遅れる」に乗っている取り決めが意味を変える［確認された前提＋推測］

* `garden/ruby/prelude.rb:97-107` — `act` のコメント。「書きは遅延され、このフレームのスクリプトが走り終わってから着地する」
  → last-writer-wins → **だから wheel（舵の取り合い）がある**。「ハンドラは 1〜2 フレーム、脳は 4〜5 フレーム
  （`me[:Hunger]` を読み、`garden.nearest` を聞き、草の `[:Transform]` を読み、やっと act するから）」
* `sabibots/ruby/prelude.rb:104-107,250-254` — 操作は last-writer-wins。
  **ハンドラを脳より低い優先度にして「フレーム内で最後に聞かれる」ようにしている**
* `sabibots/ruby/robots/scout.rb:51-55` —「取り決めは `act` の行で確認する。ループの頭だけでは 1 フレーム早すぎる」

［推測］脳の 1 パスが同一フレーム内で閉じると、「遅れて届く act が新しい act を上書きする」という力学が
「**同一フレーム内のタスク実行順**の勝負」に変わる。wheel が不要になるか、別の形の取り決めが要るかは、
どちらもありうる。

### D. 前例がある: 2→1 フレームにしただけでセルフテストが落ちた［確認］

`rubevy_games/docs/worklog/2026-09-17-battle-followups.md:249-270` —
往復を 2→1 にしたら scout の反射テストが `--headless 20` 5 回中 3 回落ちた。
原因は「被弾前に決めた `act` が 6 フレーム後でなく 3 フレーム後に届くようになり、舵の入る時間が 100 ms → 50 ms に減った」。
同 `:194-210` と `sabibots/src/main.rs:987-991`（「質問が 2 フレームから 1 フレームになったのでロボットは 1.5 倍の頻度で判断する」）も同じ話。
**1→0 は同じ種類の変更**で、同じ種類の落ち方をすると見てよい。

### E. 時間で切ってあるセルフテストの閾値［確認＋推測］

`garden/src/main.rs:249-252` `TOUCH_SETTLE = 1.5`、`:240-247` `NEWBORN_GRACE = 2.0`、
`:3813-3851` `watch_turning`（0.5 秒後の向き、`a.dot(b) < 0.7`。判定は `:4058-4064`）、
`sabibots/src/main.rs:1050-1060`（ポーズ 2 秒＝脳の `sleep 0.05` の 40 回ぶん、という数え方）。
［推測］判断頻度が上がると窓の意味が変わる。特に `watch_turning` は D と同じ形で落ちうる。

### F. 命令が 1 フレームに集中する［推測］

park が減ると `rabbit.rb:53-88` / `beetle.rb:94-126` の `loop` は次の `sleep` まで一気に走る。
今は読みごとにフレームをまたぐことで自然に分散していた命令が 1 フレームに集まるので、
`insn/f`（`garden/src/window.rs:495`）と `VmClock`（`docs/garden.md:1005-1008` の「1.3 / 8.0 ms」）は上がる。
`ScriptWorld::frame_time = 8 ms` のタイムスライス切れが常態化すると、
`inspect.rs:140` が「隠していない」と書いている**「タイムスライス切れと sleep を見分けられない」問題が主役になる**。

### G. `SHORTEST_SLEEP` の根拠は読みではなく書き［確認］

`garden/ruby/prelude.rb:410-422` の `sleep 0.05` は
「`restore_memory` がそのフレームの末尾に `@memory` を戻すので、次の行で `memory` を読むと空の Hash を読む」ための待ち
（コメント `:414-419`）。**読みだけ同期にしても消えない**。書き／復元の同期化とセットで考える必要がある。

### H. 計画が 1 本、これ待ちで止まっている［確認］

`rubevy_games/docs/plans/garden-world-plan.md:139-142` —
「保留（2026-09-17、著者判断）。W1 に着手した直後に『同期の読み書きを先に考えるべき』となり止めた。
この計画の境界（数値と周期だけ Ruby）は『コンポーネントの読みが 1 回 1 フレーム』を前提にした妥協で、
rubevy に同期アクセス（`rubevy/docs/plans/sync-access-plan.md`）が入れば `world.rb` は毎フレームの規則そのものを書ける」。
**向こうはこちらの計画のファイル名まで書いて待っている。**
元になった調査は `rubevy_games/docs/worklog/2026-09-17-garden-world-survey.md`
（`:424` 読み＝1 フレーム、`:425` 書き＝次フレーム、`:471-479` が引っかかる点 3、`:364-382` が §7 のループ回数表）。

### I. 影響が小さいもの［推測］

`@me ||=` / `@garden ||=` / `@my_genome ||=` / `memory["trees"]` のキャッシュは同期化後もそのまま正しい。
ただしそれを正当化しているコメント（`garden/ruby/prelude.rb:21-24,136,175,225-233,249-251`、
`creatures/beetle.rb:113-117`、`creatures/rabbit.rb:40-44,74-77`）は全部「往復を節約するため」「答えは 1 フレーム古い」と
書いてあるので、文面の更新対象。ゲーム側の answering システムの置き場
（`garden/src/main.rs:1346-1350`、`sabibots/src/main.rs:378-384,411`）は `Rubevy.ask` のままなので変わらない。

---

## 8. 同期にすると書き換わる既存のテストと文書

### テスト

| ファイル | どうなるか |
|---|---|
| `tests/scheduling.rs:83-104` | `a_component_read_still_costs_one_frame`。期待値 `[1,1,1,1,1,1]` が **`[0,0,0,0,0,0]` になる**。`COUNTING_COMPONENTS`（`:53-61`）はそのまま使える |
| `tests/components.rs` 全体 | 読みのテスト（`:80-105`, `:134-153`, `:156-184`, `:186-213`）は同期でも通るはず。**`a_script_writes_one_field_and_leaves_the_others`（`:109-132`）** は書きを同期にすると「いつ見えるか」が変わるので、見出しの `writes it back with the frame's other commands`（`:1-2`）が嘘になる |
| `tests/components.rs:215-236` | `rubevys_own_questions_never_reach_the_game`。4 種が `Rubevy.ask` でなくなれば `RESERVED_KINDS`（`src/lib.rs:1652`）ごと消え、**このテストは空虚になる**（消すか、「ネイティブなのでそもそもコマンドにならない」を測る形に変える） |
| `tests/entity.rs` | **変わらない**。`Rubevy.ask` と `Rubevy.despawn` しか使っていない（`:79`, `:110-117`, `:139-146`） |
| `tests/proxy.rs` | **変わらない**。`Rubevy::Proxy` は `Rubevy.ask` のまま park する |
| `tests/two_vms.rs` / `examples/two_vms.rs` | 2 つの tick が排他システムになると並列に走れなくなる。テストの主張は変わらないが、`RubevySet` の rustdoc の「2 VM の frame は互いに順序付かない」の実態が変わる |

### 文書

| 場所 | 該当 |
|---|---|
| `docs/host-api.md:290-362` | 「Components by name」。特に `:339-346`（**A read costs a frame**）、`:347-350`（rubevy が自分で答える 4 種は `take_requests` に出ない）、`:352-362`（**Why a read can wait inside `[]`** — park が要らなくなれば理由ごと消える）、`:329-337`（**What a write does** — 書きは「このフレームのスクリプトが走り終わってから」） |
| `docs/host-api.md:95-137` | `RubevySet` の表の `Tick` 行・`Answer` 行から `answer_components` / `apply_component_writes` が消える |
| `docs/host-api.md:583-586` | 「Do not try to touch Bevy's `World` from a native: a native gets `&mut Vm` and nothing else」。**この規則そのものが変わる**（少なくとも「tick の最中は例外」が付く）。ゲームのネイティブにも World を渡すのか、は計画で決める要あり |
| `src/lib.rs:25-28`（crate doc） | 「A read waits for the host, which is one frame」 |
| `src/lib.rs:682-700`, `:1646-1669`（`RESERVED_KINDS` と `answer_components` の rustdoc）, `:1773-1777` | `answer_components` / `apply_component_writes` の rustdoc |
| `src/prelude.rb:5-11` | 「なぜ Ruby で書いてあるか」。4 種がネイティブになるなら、残るのは `Proxy` の説明だけ |
| `docs/rust-bridge.ja.md:344-345` | 「1 回の質問に 1 フレーム前後かかる」 |
| `docs/rust-bridge.ja.md:353` | 「読みは 1 往復 = 1 フレームなので、宣言時とイベント時に触る形で使う」 |
| `docs/rust-bridge.ja.md`「`unsafe` の数について」の節 | 案 1 を採ると rubevy に `unsafe` が入る |
| `docs/outlook.md:63-65` | 項目 3「`&mut World` is lent to the `Host` only while an exclusive system steps VMs」— **これが到達点になる** |
| `docs/outlook.md:84-92` | 項目 4「A read is one question and one frame … not for a dozen reads a frame」 |
| `docs/outlook.md:150-153` | 「No C stack, no longjmp ↔ borrows and `Result` … so `&mut World` can be lent」 |
| `docs/outlook.ja.md:208`, `:211`, `:216` | 「書き込みは step の後にまとめて反映」「読み取りはその場の値が返ります」「読みは 1 往復 = 1 フレーム」。**`:211` は最初から同期を前提に書かれていて、今の実装の方が後退している** |
| `docs/plans/ecs-bridge-plan.md:5` | 「排他システムの中で同期に読む形は、**遅さが問題になったときに足す**」。この計画がその「そのとき」になる |

---

## 同期にするときに引っかかりそうな点（5 つ）

1. **別名（aliasing）の安全性と `unsafe` の初出。** `host_state` は `Any + Send + Sync + 'static` を要求する
   （`sabiruby/src/vm.rs:516`）ので、`&mut World` をネイティブに届けるには生ポインタの包み（`unsafe impl Send + Sync`）か
   sabiruby 側の新しい口が要る。安全の根拠にできるのは
   **`resource_scope` が `ScriptWorld` を World から本当に抜く**こと（`bevy_ecs/src/world/mod.rs:2882`）で、
   ネイティブが World 経由で `Vm` に戻る道が構造的に無い。ただしこれは「tick の間だけ貸す」を
   RAII で絶対に守り、パニック時も外れることが前提（`resource_scope` 自身は `Drop` で戻している）。

2. **tick の外から Ruby が走る道。** 今 Ruby が走るのは tick だけではない:
   `Startup` の `Vm::load_and_run`（`docs/host-api.md:549-576`）、`ScriptWorld::stop_task` の
   `funcall(:terminate)`（`src/lib.rs:804-818`。**`DeferredWorld` しかないコンポーネントフック `:237-250` から呼ばれる**）、
   `unsubscribe` の `funcall(:close)`（`:1054-1084`）。
   幸い `Task#terminate` は `ensure` を走らせない（`sabiruby/src/builtins/ext_task.rs:1049-1065`）ので
   今はユーザの Ruby は走らないが、**同期アクセスのネイティブは「World を貸されていない」状態で呼ばれうる**。
   そのとき nil を返すのか raise するのかを決める必要がある（`Startup` の 1 行目で `e[:Hp]` と書かれる）。

3. **構造変更とコンポーネントフック。** `resource_scope` の最中は `ScriptWorld<M>` が World に無いので、
   `ScriptTask` の `on_remove` フック（`src/lib.rs:237-250`）は `:246` の `else { return }` で**黙って何もしない**
   ＝タスクが止まらず購読も解放されない。`Rubevy.despawn` を同期にするなら、ここを先に直すか、
   構造変更だけは今まで通り `Commands` に残すか、どちらかを決める要あり。
   同じ理由で、tick の中で `&mut World` に触ったあとの `tasks: Query`（`:1490`）は再取得が要る。

4. **2 本の VM とフレームの直列化。** `tick_scripts` を排他システムにすると、
   VM が 2 本あれば 2 本の tick は必ず直列に走る（`examples/two_vms.rs:77-78` は今は互いに順序付いていない）。
   1 フレームの最悪時間が「2 つの `frame_time` の和」であることは変わらないが、
   **他のシステムと並列に走れなくなる**分だけフレームが長くなる。
   ついでに `host_state` は VM ごとなので、World の貸し出しも VM ごとに 2 回になる。

5. **順序の意味論が動く。** 今スクリプトが読む値は「フレーム N の `RubevySet::Answer` 時点の世界」で、
   使うのはフレーム N+1。同期にすると「フレーム N の `Tick` 時点の世界」を N のうちに使う。
   ゲームのシステムを `.before(RubevySet::Tick)` に置くかどうかが**初めて意味を持つ**。
   書きも同期にすると `Commands` と同じ約束（`src/lib.rs:1773-1777`）が破れ、
   rubevy_games の wheel / last-writer-wins の取り決め（`garden/ruby/prelude.rb:97-107`、
   `sabibots/ruby/prelude.rb:104-107,250-254`）が「フレーム内のタスク実行順」の勝負に変わる。
   2→1 フレームにしただけでセルフテストが 5 回中 3 回落ちた前例があり
   （`rubevy_games/docs/worklog/2026-09-17-battle-followups.md:249-270`）、1→0 も同じ種類の変更である。
