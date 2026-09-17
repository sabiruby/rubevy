# rubevy: 1 つのアプリに VM を複数（実装指示書）

作成 2026-09-17。著者の要望「rubevy はサンプルゲームだけでなく色々な使い方を想定したいので、VM を複数起動することもサポートしたい」。
調査（コード変更なし、実測つき）を implementer が行い、本体が計画に起こし、**別のセッションが文脈なしで実行できる形に書き直した**のがこの版。

**この文書だけで着手できるように書いてある。** 読む順は 1 → 2 → 4 → 5。3 と 6 は着手前に必ず目を通すこと。

---

## 0. はじめの一歩

```bash
# worktree はもうある（無ければ作る）。main の checkout には触らない
git -C /home/kishima/book/kishima/rubevy worktree list
#   /home/kishima/book/kishima/rubevy             [main]        ← 触らない
#   /home/kishima/book/kishima/rubevy-wt-multivm  [multi-vm]    ← ここで作業

cd /home/kishima/book/kishima/rubevy-wt-multivm
cargo test          # 着手前に全部通ることを確かめる（tests 13 本、examples 5 本）
```

作法は `/home/kishima/book/.claude/agents/implementer.md`。要点だけ再掲:

* **段階ごとに 1 コミット。** 各段階の終わりで必ずビルドとテストが通ること。
* **push しない。** `main` に触らない。
* 過程は `docs/worklog/2026-09-17-multi-vm.md` に書きながら進める（結果だけでなく、試したこと・測ったこと・**捨てた案とその理由**）。コミットに含める。
* 決めきれない点は**自分で決めずに止まって報告**する。

---

## 1. 何を作るのか（30 秒版）

今の rubevy は **1 アプリに VM が 1 本**。`ScriptWorld` が Bevy の `Resource` なので、型に対して 1 つしか置けない。
そこに**名札（型引数）**を付けて、名札ごとに別の VM を立てられるようにする。

```rust
// ── 今も、これからも動く（名札を省くと 1 本目）
RubevyPlugin::default()
fn my_system(w: ResMut<ScriptWorld>) { … }

// ── 2 本目が欲しいアプリだけ、これを足す
struct Mods;                                            // ただの名札。中身は空でいい
RubevyPlugin::<Mods>::with_asset_root("assets/mods")    // 2 本目の VM が立つ
fn mod_system(w: ResMut<ScriptWorld<Mods>>) { … }
```

名札が違えば、**ヒープ・グローバル変数・定数・クラス・シンボル・GC・タスクのスケジューラ・購読・予算・`require` のロードパス**がすべて別になる。
そして `ScriptTask<Mods>` を 1 本目の `ScriptWorld` に渡そうとすると**コンパイルエラー**になる。

**サンプルゲーム（sabibots / garden）は多 VM 化しない。** 1 行も変えずにビルドと selftest が通ることが受け入れ条件。

---

## 2. 前提と、調べ済みの事実

対象: rubevy `ec6f0ce`（`main`）。sabiruby 0.5.1（`8faf26f`）。rubevy_games `ccaf2f3`。

### 2.1 sabiruby 側は何も要らない（確認済み）

`Vm` は完全に自己中心。**`sabiruby/src/` にグローバル可変状態は 1 つも無い**（`static mut`・`thread_local!`・`OnceLock`・`lazy_static` すべてゼロ。唯一の `static` は `src/builtins/str_alnum.rs:22` の定数テーブル）。
`tests/send_sync.rs` が `Vm: Send + Sync` を保証。`src/mrbtest.rs:99` にも "the hook state lives in globals of this VM rather than in C statics, **so two VMs stay independent**"。

実測（1 プロセスで確認済み）:

| | |
|---|---|
| 64 本の VM の独立 | グローバル変数・定数・シンボル・例外・GC・タスクキューまで完全に別 |
| 4 スレッド × 各自の VM の同時実行 | 問題なし |
| VM 1 本の生成 | **約 0.6 ms**（128 本で 81 ms。ほぼ mrblib の読み込み） |
| VM 1 本の常駐 | **約 0.5 MB RSS**（128 本で 63 MB） |
| 何もしない VM を 1 フレーム回す | **約 5 ns**。32 本でも 174 ns/フレーム |

つまり **本数を増やしてもフレーム時間はほぼ増えない。効くのはメモリと起動時間だけ。**

**注意**: `sabiruby-compiler`（`ruby-source` feature、C の mrbc）は `compiler/src/lib.rs:104` の `static LOCK: Mutex<()>` で**直列化**される（`mrc_presym.c` がグローバルを持つため）。安全だが並列には走らない。VM の本数とは無関係。

### 2.2 rubevy 側で「1 本前提」なのは 6 か所

| # | 場所 | 何が問題か |
|---|---|---|
| 1 | `ScriptWorld` が `#[derive(Resource)]`（`src/lib.rs:467`） | 1 型 1 インスタンス。**これが本丸** |
| 2 | `stop_removed_task`（`src/lib.rs:144-153`） | コンポーネント削除フックが `world.get_resource_mut::<ScriptWorld>()` で型引き。どの VM のタスクか分からない |
| 3 | `ScriptTask { task: ObjId }`（`src/lib.rs:131-135`） | `ObjId` はヒープの添字。**別の VM に渡すとエラーにならず別物を指す**（一番たちの悪い壊れ方） |
| 4 | `Script`（`src/lib.rs:104`） | どの VM で起動するか言えない |
| 5 | システム群 | `start_scripts` 1130 / `release_values` 1164 / `tick_scripts` 1173 / `deliver_answers` 1260 / `drain_commands` 1283 が `ResMut<ScriptWorld>` 固定。排他 2 本 `answer_components` 1355 / `apply_component_writes` 1463 |
| 6 | `RubevySet`（`src/lib.rs:1065`） | Deliver / Tick / Answer が 1 組しかない |

### 2.3 逆に、**もう VM ごと**になっているもの（ここが大きい）

| もの | どこ |
|---|---|
| ヒープ・シンボル表・irep・スタック・グローバル変数・定数 | `Vm` が所有 |
| mruby-task のスケジューラ（ready/waiting、tick、優先度） | `Vm::task` |
| ホスト状態（コマンドキュー＋購読リスト） | `vm.set_host_state(HostState)` — `src/lib.rs:954`。コメントに「two `App`s in one process each have their own」と既にある |
| `Rubevy::Entity` クラスの ObjId | `ScriptWorld::entity_class` |
| `Host`（`require` / `read_file`）とロードパス | `vm.set_host()` / `vm.set_load_path()` — **VM ごとに差し替え可能。隔離の肝** |
| 予算・`frame_time`・`overrun` | `ScriptWorld` のフィールド |

README・`src/lib.rs:46`・`docs/outlook.md:100` に「2 本目の VM はまだ作っていない」と書いてある＝**予定済み・未着手**の項目。

---

## 3. 決まっていること・決まっていないこと

### 決まっている（著者の指示）

1. **サンプルゲームは多 VM 化しない。** 今のまま 1 VM で、**1 行も変えずに**動くこと。
2. **本数はコンパイル時に決まってよい。** 実行時に VM を増やす形（段階 4）は**やらない**。

### 既定（違和感があれば止めて報告する）

| # | 項目 | 既定 |
|---|---|---|
| 1 | 予算の合算 | **VM ごとの取り分のまま。** `frame_time` の意味は変えない。「N 本なら最悪 N×`frame_time`」は docs に明記する。フレーム全体の上限という新しい概念は入れない |
| 2 | `publish` の宛先 | **今の意味を保つ＝その VM の購読者に配る。** 全 VM に配りたいアプリは両方に呼ぶ。新 API は足さない |
| 3 | `ask` の答え手 | **VM ごと**（`RubevySet::<M>::Answer` に置く）。この設計だとこれが自然 |
| 4 | 同じ `.mrb` を 2 本に載せる | **割り切る**（VM ごとに irep。メモリも 2 倍）。共有は sabiruby 側の話なので触らない |
| 5 | sabiruby | **触らない。** 今回は rubevy だけ |
| 6 | 同じエンティティに `Script<A>` と `Script<B>` | **許す**（別のコンポーネントなので自然にそうなる）。テストを 1 本書いて意図であることを示す |

---

## 4. 設計 — 型マーカー

3 案を比べて **B（型マーカー）** を採る。理由は「**ゲームを 1 行も変えない**」を満たせるのが B だけだから（A も C もゲーム側の `Res<ScriptWorld>` / `world.vm` に手が入る）。

| | A. VM をコンポーネントに | **B. 型マーカー** | C. 1 リソースに N 本 |
|---|---|---|---|
| 本数が実行時に決まる | ◎ | ✕（コンパイル時） | ◎ |
| ObjId 取り違えの防止 | △ 実行時 | **◎ コンパイルエラー** | △ 実行時 |
| 既存 API の互換 | ✕ | **◎ 既定型引数で 1 行も壊れない** | △ アクセサ化（rubevy 48 行＋ゲーム 13 行） |
| VM 間の並列 | ○ | **◎ 別リソースなので Bevy が並列化** | ✕ |

### 4.1 変わる型（before / after）

```rust
// before                              after（既定型引数つき）
pub struct ScriptWorld { … }           pub struct ScriptWorld<M = ()> { …, _m: PhantomData<fn() -> M> }
pub struct Script { … }                pub struct Script<M = ()> { …, _m: PhantomData<fn() -> M> }
pub struct ScriptTask { task: ObjId }  pub struct ScriptTask<M = ()> { task: ObjId, _m: PhantomData<fn() -> M> }
pub struct ScriptEnded { … }           pub struct ScriptEnded<M = ()> { … }
pub enum   RubevySet { … }             pub enum   RubevySet<M = ()> { Deliver, Tick, Answer, …PhantomData }
pub struct RubevyPlugin { … }          pub struct RubevyPlugin<M = ()> { …, _m: PhantomData<fn() -> M> }
```

`Request` / `Answer` / `Arg` / `RootedValue` / `MrbAsset` / `ScriptStats` / `ScriptStatus` / `ScriptDone` / `SpawnedByScript` は**そのまま**。
`Request` に VM の識別は要らない（`take_requests` を呼ぶのが `ScriptWorld<M>` 自身だから、答え手はもう自分がどの VM か知っている）。

### 4.2 `PhantomData<fn() -> M>` を使うこと（`PhantomData<M>` ではなく）

`PhantomData<M>` にすると `M: Send + Sync` が要求される。`PhantomData<fn() -> M>` なら **M が何であっても `Send + Sync`** になり、`struct Mods;` のような空の名札がそのまま使える。
同じ理由で `M` には `'static` 以外の境界を付けない。

### 4.3 隔離は設定 2 行で書ける（この設計の売り）

```rust
App::new()
    .add_plugins(RubevyPlugin::default())                             // 本体の VM
    .add_plugins(RubevyPlugin::<Mods>::with_asset_root("assets/mods")) // mod の VM
    .add_systems(Startup, |mut w: ResMut<ScriptWorld<Mods>>| {
        w.budget = 20_000;                    // 本体の 1/10 しか回らない
        w.frame_time = Some(Duration::from_millis(1));
    });
```

`Host` とロードパスが VM ごとなので、mod は `assets/mods` の外を `require` できない。

---

## 5. 段階

各段階の終わりで **必ず** §7 の確認を全部走らせ、通ってからコミットする。

> 調査時のメモでは段階 0 を「`ScriptTask` などに『どの VM か』を**フィールドで**持たせる下準備」と書いたが、
> この設計では「どの VM か」は**型引数**なのでフィールドは要らない。段階 0 は「型引数を機械的に通す（機能変更なし）」に読み替える。

### 段階 0 — 型引数を通す。**機能は 1 ミリも変えない**

やること: §4.1 の型に `M = ()` の型引数を足し、システム・関数・内部呼び出しを機械的に `<M>` にする。

* `impl<M: 'static + Send + Sync> ScriptWorld<M>` のように境界は最小限（`'static + Send + Sync` だけ）。
* `RubevyPlugin::default()` / `::with_asset_root(..)` は `RubevyPlugin<()>` を返す。既存の呼び出しがそのまま通ること。
* この段階では**まだ 2 本目を立てない**。プラグインの二重登録（§6.3）はまだ直さなくてよい。

終わったときの状態: **既存の tests 13 本・examples 5 本・サンプルゲーム 2 本が無変更で通る。** VM は実質 1 本。

コミット例: `rubevy: a type marker on the VM — the plumbing, with nothing yet behind it`

### 段階 1 — 2 本目が実際に立つ

やること:

1. **プラグインの二重登録を避ける**（§6.3）。`init_asset` / `init_asset_loader` は共有資源。
2. `RubevySet<M>` の `configure_sets` が名札ごとに独立していること。
3. 新テスト `tests/two_vms.rs`:
   * 片方の `$x` / `class Foo` / `CONST` が他方から見えない
   * 片方で例外が上がっても他方は動き続ける
   * 片方の予算を 0 にしても他方は走る（一時停止の独立）
   * 片方のタスクを終わらせても他方のタスクは残る
   * `ScriptStats` がそれぞれ自分の数を返す
   * 同じエンティティに `Script<A>` と `Script<B>` を付けられる（§3 既定 6）

終わったときの状態: `RubevyPlugin::<Mods>` を足すと本当に 2 本動く。

コミット例: `rubevy: a second VM in one app — the plugin twice, the tests that prove they do not mix`

### 段階 2 — 1 本前提の残りを潰す

やること（§2.2 の 2・5 の排他システム・`publish`）:

1. **`stop_removed_task`**（`src/lib.rs:144`）— 型付けする。フックの属性がジェネリクスを取れない場合は §6.2 の代案。
2. **`answer_components`**（1355）と **`apply_component_writes`**（1463）— `&mut World` の排他システム。`M` ごとに登録する形にする。
3. **`publish`** — §3 既定 2 のとおり「その VM の購読者に配る」を保つ。テストで 2 本目の購読者に 1 本目のイベントが届かないことを示す。

終わったときの状態: 2 本目でもイベント・コンポーネント読み書き・タスクの後始末が正しい。

コミット例: `rubevy: the hook, the two exclusive systems and publish, once per VM`

### 段階 3 — 使い方を見せて、書き残す

1. **`examples/two_vms.rs`** — ゲーム本体の VM と mod 用 VM。示すのは 3 つ:
   1. mod からゲーム本体のクラス・グローバル変数が**見えない**
   2. mod の `loop {}` がゲーム本体のフレームを**食わない**（予算が別）
   3. mod が `require` でゲーム本体のファイルを**読めない**（ロードパスが別）
   スクリプトは `assets/scripts/` と `assets/mods/` に置く。
2. **docs** — 「2 本目の VM はまだ作っていない」と書いてある 4 か所を書き換える:
   `README.md:50` / `src/lib.rs:46`（crate doc）/ `docs/host-api.md:3` / `docs/outlook.md:9,100`。
   `docs/host-api.md` に「VM を 2 本立てるには」の節を足し、**予算が VM ごとであること（N 本なら最悪 N×`frame_time`）** と **irep が VM ごとに複製されること**を明記する。
3. **worklog** `docs/worklog/2026-09-17-multi-vm.md` を仕上げる。
4. この指示書の段階表を「済み」にし、ハッシュを入れる。

コミット例: `docs: more than one VM in one app — the example, the host API, the outlook`

### 段階 4 — **やらない**

実行時に本数が決まる形（プレイヤーごと・mod ごとに VM を増やす）。名札方式ではできない。
やるなら案 A（VM を Bevy のエンティティに持たせる）か C（1 リソースに `Vec<VmSlot>`）で、**そのときはサンプルゲーム側にも手が入る**。用途が出てから別の指示書で。

---

## 6. 分かっている罠（着手前に読む）

### 6.1 `#[derive(SystemSet)]` などの derive がジェネリクスに境界を足す

`#[derive(Clone, PartialEq, Eq, Hash, Debug)]` はジェネリックな型に `M: Clone` 等の境界を**勝手に足す**。空の名札（`struct Mods;`）はこれらを実装していないので、そのままでは通らない。
**対策**: `RubevySet<M>` の `Clone` / `Copy` / `PartialEq` / `Eq` / `Hash` / `Debug` は derive せず**手で書く**（`M` に境界を付けない）。`ScriptTask<M>` も同様。

### 6.2 コンポーネントのフックとジェネリクス

`#[component(on_remove = stop_removed_task)]` が `stop_removed_task::<M>` を取れるかは Bevy 0.19 で要確認。
**通らなければ代案**: フックをやめて、プラグインが `M` ごとに `app.add_observer(on_remove_script_task::<M>)` を登録する（`On<Remove, ScriptTask<M>>`）。挙動は同じで、ジェネリクスとの相性はこちらのほうがよい。
**どちらを採ったか、なぜかを worklog に書くこと。**

### 6.3 プラグインを 2 回足すと共有資源を 2 回登録する

`RubevyPlugin::build`（`src/lib.rs:1101-1103`）の
`init_asset::<MrbAsset>()` / `init_asset_loader::<MrbLoader>()` は **名札に関係なく共有**。2 本目のプラグインが同じものをもう一度登録する。
`app.is_plugin_added::<RubevyPlugin<M>>()` は型が違うので役に立たない。
**対策**: `if !app.world().contains_resource::<Assets<MrbAsset>>() { … }` のような**実体での判定**にする。
`add_message::<ScriptEnded<M>>()` は `M` ごとに別の型なので、そのままでよい。

### 6.4 `ObjId` の取り違えは今でも起こりうる

段階 0 の型引数が入るまで、`ScriptTask` の `ObjId` を別の VM に渡すと**静かに別のオブジェクトを指す**。
段階 0 を最初に済ませるのはこのため。段階 0 の前に 2 本目を立てて試さないこと。

### 6.5 フレーム予算は合算されない

`frame_time` は VM ごと。N 本立てれば最悪 N×`frame_time` かかる。既定ではこれを直さない（§3 既定 1）。**docs に明記すること。**

### 6.6 サンプルゲームの `Cargo.lock` を汚さない

§7.2 の確認は `--config` でパッチを当てるが、`Cargo.lock` が書き換わる。**確認のあと必ず戻す。**

---

## 7. 確認手順（そのまま貼れる）

### 7.1 rubevy 本体

```bash
cd /home/kishima/book/kishima/rubevy-wt-multivm
cargo test                     # tests 13 本
cargo build --examples         # examples 5 本
cargo run --example headless   # 段階 3 のあとは two_vms も
cargo clippy --all-targets     # 警告を着手前より増やさない
```

### 7.2 サンプルゲームが**無変更で**通ること（この計画のいちばん大事な確認）

rubevy_games は rubevy を **git 依存**で見ているので、この枝に向けるには `--config` でパッチを当てる。
`.cargo/config.toml` は git-ignore されていないので**ファイルを作らない**こと。

```bash
cd /home/kishima/book/kishima/rubevy_games
PATCH='patch."https://github.com/sabiruby/rubevy".rubevy.path="/home/kishima/book/kishima/rubevy-wt-multivm"'

cargo build --release -p sabibots -p garden --config "$PATCH"

SABIBOTS_SELFTEST=1 ./target/release/sabibots --headless 25   # 2 回。FAIL が 0 であること
GARDEN_SELFTEST=1  ./target/release/garden   --headless 90    # 3 回。9 判定すべて ok

git checkout Cargo.lock      # ← 必ず戻す
git status --short           # 何も出ないこと
```

**`rubevy_games` のソースは 1 行も変えないこと。** 変えないと通らない箇所が出たら、それは設計が間違っているので**止めて報告**する。

### 7.3 窓が要る確認

無し。この計画に GPU は要らない。

---

## 8. やらないこと

* サンプルゲームを多 VM 化しない。
* 実行時に VM を増やす形（段階 4）を作らない。
* sabiruby に手を入れない。
* フレーム全体の予算という新しい概念を入れない。
* `publish` の宛先を変えない（全 VM 配信の新 API を足さない）。
* irep を VM 間で共有しない。

---

## 9. 影響範囲（参考の数）

rubevy 内: `ScriptWorld` 71 行、`self.vm` / `world.vm` 48 行、tests 13 ファイル、examples 5 本。
rubevy_games 側: `ScriptWorld` 39 行、`ScriptTask` 29 行、`.vm` 直接参照 13 行（5 ファイル）。
**この設計なら、既定型引数のおかげで rubevy_games 側は 0 行の変更で通る見込み。** そうならなければ設計の誤りなので止めて報告する。

## 10. 段階の状況

| 段階 | 内容 | 状態 |
|---|---|---|
| 0 | 型引数を通す（機能変更なし） | 未着手 |
| 1 | 2 本目が実際に立つ＋独立のテスト | 未着手 |
| 2 | フック・排他システム 2 本・`publish` を VM ごとに | 未着手 |
| 3 | `examples/two_vms.rs` と docs、worklog | 未着手 |
| 4 | 実行時生成 | **やらない**（用途が出てから別の指示書で） |
