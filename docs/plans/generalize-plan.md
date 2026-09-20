# rubevy: 規模の手当てと、ゲームが毎回手で書いていた口 — 実装指示書

作成 2026-09-20。著者「3 本目（Factorio 風の工場）は汎用化のためのサンプルでもある」「先に crate へ上げる」
「要は Ruby からカメラとかゲームに必要な IF を使える口があるとうれしい」。
調査は `docs/worklog/2026-09-20-factory-survey.md`（実測の表と ファイル:行。**着手前に全部読む**）と、
rubevy_games `docs/worklog/2026-09-20-factory-survey.md`（全体像、著者の返事）。
対象: rubevy main `fb4f398`（0.0.1 公開済み）、sabiruby main `08d8e78`（0.5.2）、Bevy 0.19.1。
行番号は調査時点のもの。着手前に実物と合っているか確かめる。

**この文書だけで着手できるように書いてある。** 読む順は 1 → 2 → 4 の自分の段階 → 3 の該当節 → 5。

---

## 0. はじめの一歩

```bash
cd /home/kishima/book/kishima
git -C rubevy worktree list          # rubevy-wt-factory-survey（ブランチ factory-survey）が残っている。
                                     # 未コミットの examples/factory_events.rs / factory_machines.rs は R5 の材料
git -C rubevy worktree add ../rubevy-wt-generalize -b generalize main
cd rubevy-wt-generalize && cargo test    # 着手前に全部通ること
```

作法は `/home/kishima/book/.claude/agents/implementer.md` と `/home/kishima/book/CLAUDE.md`。段階ごとに 1 コミット、push しない、
main に触らない、過程は `docs/worklog/` に書きながら進める。**ベンチは同時に 1 つだけ**（`pgrep -af "cargo|rustc|sabiruby"` で確かめ、`taskset -c 2`）。
main に `.cargo/config.toml` は無く、rubevy は crates.io の sabiruby 0.5.2 でビルドされる。隣の checkout を使う段階（R7）は
worktree 側に config を**コピー**する（cargo の config は cwd から上へ辿るので `rubevy/.cargo/` は効かない）。

---

## 1. 何を作るのか（30 秒版）

1. **規模の手当て**（R1〜R5）: 実測で見つかった 5 つ — 購読の全走査、irep が台数ぶん、あふれが見えない、`frame_time` が上限でない、
   フレーム単位の統計が無い — を直し、外の利用者が「自分の機械で何本載るか」を測れる example を常設する。
2. **ゲームが毎回手で書いていた口**（R6〜R7）: prelude つきプログラムと行番号補正、スクリプトの差し替え、埋め込みの `Host`、
   `.rb` の埋め込み、宣言を集める口（data stage）。garden と sabibots が同じものを 2 回書いている。3 本目に 3 回目を書かせない。
3. **Ruby から使う口**（R0、R8〜R9）: resource を名前で読む口と、find・component の上に載せる任意の Ruby 層（まずカメラ）。

rubevy はゲームの語彙を知らない。**工場・機械・レシピ・ロボット・庭という語を `src/` と公開 API に入れない。**

---

## 2. 決まっていること・既定

### 決まっている（著者、2026-09-20）

- 汎用化を先にやり、3 本目はそれを最初に使う客になる。既存 2 本も載せ替える（rubevy_games 側の計画 `shared-crate-plan.md`）。
- 工場の前に直すもの: 購読の索引、irep の共有、あふれの通知と上限の選択、`frame_time`、フレーム統計 + 計測 example の常設。
- 人が操作するパン・ズームのカメラは **rubevy に入れない**（共有 crate）。rubevy に入るのは Ruby からの口。
- unsafe を書かない。wasm32-unknown-unknown で動くこと（`tests/no_wasm_unsupported.rs` を通す。`std::time::Instant`・スレッド・`std::fs` を `src/` に足さない）。
- 根拠のない数を書かない。

### 既定（違和感があれば止めて報告する）

- **公開 API は足すだけ。** 既存の名前・意味を変えない。変えざるを得ないなら止まって報告。
- **rubevy の依存を増やさない**（今は `sabiruby`・`bevy`（`bevy_asset`, `bevy_log`）・`thiserror`）。カメラ・入力・窓の feature を足さない。
- **`pause(bool)` は足さない。** `budget = 0` が一時停止、という判断は済んでいる（`docs/host-api.md`「Pausing」）。
- **数は利用者が変えられる場所に置く**（著者、2026-09-20「マジックナンバーは基本的に禁止。ユーザが変えられるようにするべき」）。
  rubevy が持つ数（上限・閾値・既定値・刻み）は、`budget` / `frame_time` / `overrun` と同じく **`ScriptWorld<M>` の `pub` フィールド**
  （VM ごと）か、呼び出しの引数にする。`const` にしてよいのは、変えると壊れる不変量（形式の版、表の並び）だけ。
  既定値には出どころを rustdoc に 1 行。出どころを言えない既定値を新しく書かない — 測って決めるか、決められなければ止まって報告。
- `QUEUE_LIMIT`（`const 64`、測って決めた数ではなく、app からも変えられない）は**フィールドにする**（3.3）。
  既定値を 64 のままにするか測った数に替えるかは R3 で材料を揃えて報告し、著者が決める（既定を変えると既存のスクリプトの見え方が変わるため）。
- rubevy に既にあるほかの数も同じ扱いにする（著者、2026-09-20「基本は設定可能なように、理由があるなら理由も残して」）。R10（3.10）。
- 版は上げない。`CHANGELOG.md`（根にある。主張ごとにコミットを添える流儀）に、段階ごとに「Unreleased」の行を足す。公開は著者の指示があってから。
- sabiruby 本体（VM の crate）は触らない。VM 側の不足（待ちタスクの O(N)、irep が解放されない）は記録にとどめる（5 章）。

---

## 3. 設計

### 3.1 購読の索引（R1）

今: `HostState.subscriptions: Vec<Subscription>` を `publish` のたびに全走査（`src/lib.rs:1099-1107`）。
誰も聞いていない名前でも購読 1 件あたり約 0.22 ns、1000 購読で 219 ns/回。
→ 名前で引ける形にする（名前 → 購読者の列）。エンティティ宛と全員宛の区別、購読の解除（`Unsubscribed`、タスク終了・despawn 時の掃除）、
`subscriptions()` の数え方を保つ。公開 API は変えない。

### 3.2 同じ `.mrb` は 1 つの irep（R2）

今: `start_scripts` がエンティティごとに `vm.load(&asset.bytes)`（`src/lib.rs:1560-1588`）。N 台で irep が N 部（1 台 2.2 kB、2.5 µs）。
→ `ScriptWorld<M>` に「アセット → 読み込み済みの irep」の表を持ち、初回だけ `vm.load`。`Vm::task_spawn` は `IrepId` を取るので VM は不変。
アセットが差し替わったら（`AssetEvent::Modified` / `Removed`、ホットリロード、エディタの Apply が新しい `Handle` を作る場合）表から外す。
**古い irep は VM から消えない**（sabiruby に `ireps` を消す口が無い）。これはこの段階では直せない。rustdoc と worklog に書く。
表の鍵を `AssetId` にするか内容のハッシュにするかは、games の Apply が毎回新しいアセットを足している（同じ内容でも別 `AssetId`）かを見て決める。
決めきれなければ止まって報告。

### 3.3 あふれの通知と上限の選択（R3）

- Rust: `ScriptWorld::dropped() -> u64`（その VM が落とした累計）。`make_room`（`src/lib.rs:1123-1139`）で数える。
- Ruby: `Rubevy::Subscription#dropped`（その購読が落とした累計）。`make_room` がキューオブジェクトの ivar を増やし、`src/prelude.rb` が読む。
  `pop` の戻り値の型は変えない（番兵を流す案は既存スクリプトを壊すので採らない）。
- **上限は 3 段で選べる**: `ScriptWorld::queue_limit`（`pub` フィールド、その VM の既定。app が `budget` と同じ場所で変える）→
  `Rubevy.subscribe(name, limit: n)`（その購読だけ）。`n` は 1 以上の整数、それ以外は `ArgumentError`。
  公開の関連定数 `ScriptWorld::<()>::QUEUE_LIMIT` は消さずに「既定値」として残すか非推奨にするかを、公開 API を壊さない側で選ぶ。
  app が Ruby 側の `limit:` に天井を掛けたいか（信頼しないスクリプトが巨大な上限を頼む）は、要るなら同じ形のフィールドで。要否を報告に書く。
- **既定値の材料を測る**: 購読 1 本が 1 フレームに読み切れる件数（予算と `frame_time` のもとで）、1 件あたりのメモリ、読まない購読が抱える最悪のメモリ
  （上限 × 1 件 × 購読数）。計測器は R5 の `how_many_subscribers`。「既定値はこの 3 つからこう導ける」という案を 1 つ以上添えて報告する。
  捨てる向きの選択（`drop: :newest`）は**足さない**（要る利用者が出てから）。
- `prelude.mrb` は生成物。作り方は `tools/` と worklog で確かめ、手で直さない。

### 3.4 `frame_time` を上限にする（R4）

今: 時計は答えループの周回の頭でしか見ない（`src/lib.rs:1691-1696`）。`answer_reflect_requests` / `answer_in_tick_requests` は
時間で区切られず、3000 台で設定値の 2〜4 倍になった。
→ 答えを作るループの中でも時計を見て、時間切れなら残りの問いを**次のフレームへ持ち越す**（持ち越しの道 `reflect_requests` /
`in_tick_requests` は既にある。`src/lib.rs:736`、`drain_commands` の仕分け `:1827-1833`）。時計は VM に渡してあるのと同じ源（`clock_ns`）を使う
（`docs/worklog/2026-09-18-wasm-instant.md` の理由）。
代償: 「読みは同じ tick で返る」が、時間切れのフレームの末尾に並んだタスクについてだけ 1 フレーム遅れる。
これを `docs/host-api.md`「Components by name」「Time」に書く。時計を見る頻度（毎問か、何問かに 1 回か）は**測って決める**
（時計 1 回の費用と答え 1 件の費用 2.2 µs の比から）。既存のテストが「同じ tick で返る」を時間切れの状況で要求していたら止まって報告。

### 3.5 フレーム統計と計測の example（R5）

- `ScriptWorld::last_frame() -> FrameStats`。中身の候補: 使った命令数、答えループの周回数、走ったタスク数、フレーム終わりに ready のまま残ったタスク数、
  tick の中で答えた問いの数、持ち越した問いの数（R4）、落としたメッセージ数（R3）、tick の実時間。VM と rubevy が既に持っている数だけで組む。
  取れない項目は足さず、報告に書く。`ScriptStats`（1 本ぶん）の対。
- worktree `rubevy-wt-factory-survey` の 2 本を、語彙を外して常設する:
  `examples/how_many_scripts.rs`（`Machine` → 無色の名前）、`examples/how_many_subscribers.rs`。冒頭に「ネイティブ専用の計測器、assert しない」と書く
  （`tests/read_cost.rs` と同じ流儀）。`docs/verification/`（無ければ作り、`docs/README.md` の目次に足す）に、
  R1〜R4 の**前後**の表を置く。前の数字は調査の worklog にある。同じ機械・同じ条件で取り直して並べる。

### 3.6 ゲームが 2 回書いていた口（R6）

rubevy_games の 2 本に同じ形で書かれているもの。場所は rubevy_games の worklog §3・§5。

- **prelude つきプログラム**: `{prelude}\n# ---- {name} ----\n{body}\n{tail}\n` を組み、「著者の 1 行目が全体の何行目か」を返す
  （`garden/src/main.rs:2727-2740`、`sabibots/src/main.rs:1630-1645`）。コンパイルそのものは rubevy に入れない
  （ブラウザでは JS の関数、PC では C 由来。rubevy は `ruby-source` feature 以外でコンパイラを持たない）。入れるのは**文字列を組む側**と、
- **エラー行番号の補正**: コンパイラのメッセージの `FILE:LINE:COL:` から prelude ぶんを引く。prelude 側のエラー、末尾より後ろのエラーの扱いも
  garden の実装（`garden/src/main.rs:2761-2795`）が答えを持っている。入力は文字列だけ、依存 0。**sabibots はこれが無く、295 行ずれた番号を出している。**
  `VmInspector::fill` が取る `prelude_lines` と同じ数であること。
- **スクリプトの差し替え**: `ScriptTask` / `ScriptDone` を外して新しい `Script` を挿す 3 行（`sabibots/src/main.rs:1648-1654`）。
  `docs/host-api.md`「Replacing and removing a script」に節はあるが関数が無い。`Commands` の拡張か自由関数かは既存の API の流儀に合わせる。
- **埋め込みの `Host`**: `&'static [(&'static str, &'static str)]`（パス, ソース）か（パス, `.mrb`）から読む `Host`。ブラウザで `require` が動く。
  ソースを読むならコンパイルが要るので、`ruby-source` の有無での振る舞いを `FileHost`（`src/lib.rs:1288-1313`）に揃える。
  ブラウザのコンパイルは JS の橋なので、「コンパイルする関数を外から渡す」形が要るはず。形を決めきれなければ止まって報告。
- **`.rb` を埋め込む build ヘルパ**: 2 本の `build.rs`（35 行、1 バイトも違わない）。rubevy を build-dependency にすると bevy がホスト向けにもう 1 回ビルドされるので、
  **std だけの小さな crate**（同じ repo、例 `rubevy-build`）にする。repo を workspace にする影響（`Cargo.lock`、CI、`cargo publish`）を確かめ、
  大きければ止まって報告。
- **wasm でも動く時計の種**（`SystemTime` ⇄ `Date.now()`）は rubevy に入れない（`js-sys` の依存が増える。共有 crate 側）。

### 3.7 宣言を集める口（R7、data stage）

Ruby が `item :iron_plate, stack: 100` のように並べた宣言を、Rust の `Vec<(String, T)>`（`T: Deserialize`）で受ける。
全部いまの部品で書ける（rubevy_games の worklog §2）: `define_fn` / `define_closure`、キーワード引数は末尾の Hash、`Serde<T>`、`Vm::load_and_run`。

- **置き場所は `sabiruby-serde`**（sabiruby の repo の `serde/`。VM の crate ではない）。Bevy が要らないので、rubevy の外の sabiruby 利用者にも使える。
  rubevy は `sabiruby-serde` に依存しない方針（`Cargo.toml` のコメント）を変えない。rubevy 側は `docs/host-api.md` に
  「Startup で宣言を集める」節と `examples/declarations.rs` を足すだけ。
- `sabiruby-serde` は `no_std`。`Mutex` は使えないので、集めた表は **VM の host store**（`install_host_store::<T>()`、型ごとに 1 つ）に置き、
  走り終わってから取り出す。**`Vm::set_host_state` は使わない**（rubevy が占有している。上書きすると rubevy のコマンドが黙って消える）。
- 呼び出し規約: 第 1 引数が名前（Symbol か String）、残りがキーワード Hash → `T`。名前の重複は後勝ちにせず `ArgumentError`（行番号が付く）を既定にし、
  上書きを許す口（mod が既存の定義を変える）は別の名前で。`#[serde(deny_unknown_fields)]` を例で勧める。
- 読み返し: Rust の表を Ruby から引くネイティブ（`Serde<T>` を返す）を登録するヘルパ。Symbol キーで返す（`Options::symbol_keys()`）。
- sabiruby の作法（`no_std`、`tools/check_no_std.sh`、テスト、CHANGELOG）に従う。`sabiruby-serde` の版上げと公開は著者の指示があってから。
  rubevy_games は `[patch.crates-io]` で git を見ているので、公開を待たずに使える。

### 3.8 resource を名前で読む口（R8）

`Rubevy.resource(:Name)` → Hash（`ReflectResource`。今 `src/` に 0 件）。読みは component と同じく tick の中で答える。
書き（`Rubevy.set_resource(:Name, hash)` か `Rubevy.resource(:Name)[...] =` か）は component の書きと同じ規則（名前を挙げたフィールドだけ、フレームの終わり）。
登録されていない型は `nil`。形は `src/reflect.rs` の component の道をなぞる。名前の決め方（短い型名、衝突時）も component に合わせる。

### 3.9 任意の Ruby 層（R9）

rubevy が同梱するが、**読み込むかどうかは app が決める** `.mrb`。Rust の依存を増やさず、find・component・resource・ask の上に書きやすい名前を載せる。
最初の 1 枚はカメラ: `Rubevy::Camera`（`pan dx, dy` / `move_to x, y` / `zoom 比` / `follow entity`（毎フレーム追う子タスク）/ `position` / `scale`）。
形は **R0 の結果で決める**（`Projection` に書けるか、2D と 3D で何が違うか）。画面座標 → ワールド座標のような**計算**は component では取れないので、
この層は `Rubevy.ask("camera.world_at", x, y)` を投げるだけにし、答える側は app（共有 crate）に置く。答える人がいないときの振る舞いを決めて書く。
読み込みの口は `ScriptWorld` のメソッド 1 つ（`load_and_run`）で、`prelude.mrb` と同じ生成の道に載せる。

### 3.10 既にある数の棚卸しと設定化（R10）

著者「基本は設定可能なように、理由があるなら理由も残して」。新しく足す数（R3 の上限、R4 の時計を見る頻度、R5 の統計）だけでなく、**rubevy に今ある数を全部**同じ形にする。

1. **一覧を作る。** `src/`（`lib.rs`、`reflect.rs`、`prelude.rb`）の `const` と数値リテラルを全部拾う。少なくとも:
   `budget` 200,000 / `frame_time` 8 ms / `overrun` 50 ms の既定（`src/lib.rs:818-820`）、`QUEUE_LIMIT`（R3 で済み）、スクリプトとハンドラの優先度の既定と差、
   ロードパスの既定（`{root}/scripts`）、`$rubevy` の更新、リフレクションの深さや大きさの限度があればそれ、`prelude.rb` の中の数。
   0・1・添字・単位換算（ns ⇄ ms）のように数そのものに意味が無いものは一覧に入れず、入れなかった基準を書く。
2. **出どころを探す。** 各々について rustdoc、`docs/worklog/`、`docs/plans/`、`git log -S` で「なぜこの値か」を探し、見つかったものは引用つきで、
   見つからないものは**「出どころ不明」とそのまま書く**（もっともらしい理由を後から作らない）。
3. **分類する。** (a) 不変量（変えると壊れる。`const` のまま、壊れる理由を 1 行）、(b) 既に設定できる（出どころを rustdoc に足すだけ）、
   (c) 設定できない調整値 → **設定できるようにする**: VM ごとのものは `ScriptWorld<M>` の `pub` フィールド、プラグイン全体のものは `RubevyPlugin` の設定、
   スクリプトごとのものは `Script` のビルダか Ruby の引数。既定値は**今の値のまま**（この段階では動きを変えない）。
4. **記録する。** `docs/numbers.md`（新規。`docs/README.md` の目次に足す）に 1 表: 名前、既定値、どこで変えるか、出どころ（測った日と条件／導出／引用／**不明**）、分類。
   同じ 1 行を各フィールドの rustdoc にも。`docs/host-api.md` の「Time」の表に出どころの列を足す。
5. **既定値を動かす提案は別に出す。** 出どころ不明の既定値、実測（2026-09-20、R1〜R5 の前後）と合わない既定値については、
   「こう測ればこう導ける」という案を報告に書く。動かすかどうかは著者が決める。

以後、rubevy に数を足す変更は `docs/numbers.md` に 1 行足すことを含む。

---

## 4. 段階

各段階 1 コミット。確認は毎回 `cargo test`、`cargo clippy --all-targets`、`cargo build --target wasm32-unknown-unknown`（lib）。
R1〜R5 は計測を伴う。R0 と R6 以降は計測なし。

| 段階 | 到達点 | 確認 |
|---|---|---|
| **R0** | 今の rubevy のまま、Ruby から `Camera2d` を動かし、ズームできるかが分かっている。`examples/camera_from_ruby.rs`（ヘッドレス、型を登録）と `tests/` に 1 本 | `Rubevy.find(:Camera2d)`、`cam[:Transform]=` で位置が変わる。`Projection`（enum の component）の `scale` を書けるか・書けないならなぜか・どう直すか（`src/reflect.rs` の enum の書きの規則）を worklog に。**書けない場合は直し方の案を報告して止まる**（R9 の形が変わる） |
| **R1** | 購読の索引 | 計測器（R5 の元になる worktree の example を一時的に持ってきてよい）で、誰も聞いていない名前への publish の費用が購読の総数に依らなくなった前後の表。`tests/events*.rs` が通る |
| **R2** | 同じ `.mrb` は 1 つの irep | N 台で `ireps` が台数に依らない。起動フレームと RSS の前後。アセットを差し替えたら新しいコードで動くテスト。差し替えを M 回繰り返したときの `ireps` の増え方を測って worklog に（直せないが、数字は残す） |
| **R3** | `dropped()`、`Subscription#dropped`、`ScriptWorld::queue_limit` と `subscribe(limit:)`、既定値の材料、埋め込みの数の一覧 | あふれさせるテスト（既存の `the_oldest_messages_are_dropped_when_nobody_reads` の隣）で件数が合う。フィールドでも `limit:` でも 1 フレームに読める件数の天井が動く。2 本目の VM の上限は別に持てる。不正な `limit:` は `ArgumentError` |
| **R4** | `frame_time` が上限 | 多数のタスクが読みに並ぶ状況で、tick の実時間が `frame_time` + 「測った 1 回ぶんの粒度」に収まる前後の表。持ち越されたタスクが次のフレームで答えを受け取り、飢えないテスト。`docs/host-api.md` の 2 節を更新 |
| **R5** | `FrameStats`、常設の example 2 本、`docs/verification/` の前後の表 | example が README か host-api.md から辿れる。表は R1〜R4 の効果を 1 枚で言う |
| **R6** | prelude つきプログラム + 行番号補正、差し替え、埋め込みの `Host`、build ヘルパ | garden の `in_the_authors_lines` のテストケース（prelude 内、本文、末尾より後ろ）を移して通す。埋め込みの `Host` で `require` が通るテスト。`docs/host-api.md` に節 |
| **R7** | `sabiruby-serde` に宣言を集める口と読み返しのヘルパ、rubevy に節と example | sabiruby 側: `no_std` チェック、テスト（宣言 N 件、欠けたフィールドのエラーに Ruby の行番号、重複、`deny_unknown_fields`）。rubevy 側: `examples/declarations.rs` が Startup で表を作り、最初の `Update` の前に Resource に入っている |
| **R8** | `Rubevy.resource` | component のテストと同じ形のテスト（読み、部分書き、未登録は nil）。読み 1 回の費用を `tests/read_cost.rs` の流儀で測る |
| **R9** | 任意の Ruby 層（カメラ） | R0 の example をこの層で書き直して短くなる。答える人がいない `world_at` の振る舞いのテスト |
| **R10** | rubevy の数の一覧（`docs/numbers.md`）、設定できなかった調整値が設定できる、各々に出どころか「不明」 | 既定値のままで全テストが通り、計測器の数字が R5 の表とぶれの範囲で同じ（動きを変えていない証拠）。設定を変えると効くテストを、足したフィールドごとに 1 つ。出どころ不明の一覧と、既定値を動かす提案を報告して**止まる** |

R10 は R5 の後ならいつでも（R3〜R5 が足す数も一覧に入るので、その後）。
rubevy_games 側（`shared-crate-plan.md`）は R6 が終われば始められる。R7 は工場の data stage（`factory-plan.md` F2）の前まで、R8〜R9 は工場の窓（F5）の前までに要る。
rubevy を触ったあとの **`web/build.sh` + Playwright の確認**（共通 CLAUDE.md）は、games が取り込む段階（`shared-crate-plan.md` S2）で行う。

---

## 5. 分かっている罠

- `publish` は購読者ごとに `build(&mut vm)` を呼ぶ（`src/lib.rs:1108-1115`）。索引にしてもここは変わらない。「1 度作って配る publish」は**今回やらない**
  （受け手が書き換えると全員に見える意味論の違い。入れ子の payload の費用をまだ測っていない）。
- push 1 回ごとに VM が `Q_WAITING` を clone して走査する（sabiruby `ext_task.rs:814-822`）。待ちタスクが多いほど publish も `sleep` の起床も高くなる。
  rubevy からは直せない。R1 の効果がこれに隠れて見えにくいときは、読み手が park していない条件（P が大きい行）で比べる。
- 全タスクが同じ長さ眠ると同じフレームに起きる（p95 が 20 ms）。計測器の `sleep` は調査と同じ値で取り、前後を比べられるようにする。
- `tests/child_task.rs` は負荷時に 1 回だけ落ちたことがある（再現せず）。`tests/pause.rs` も機械が混んでいると落ちる。落ちたら静かな機械で取り直し、報告に書く。
- `Cargo.lock` は 0.5.2 を指している。R7 で隣の sabiruby を使うとき、lock の差分をコミットに混ぜない。
- 例と doctest は `MinimalPlugins` だと Bevy の型が登録されない（`examples/components.rs` が `Transform` を自分で登録している）。R0・R8 も同じ。

## 6. 状況

| 段階 | 状況 |
|---|---|
| R0〜R10 | 未着手（2026-09-20、計画のみ） |

VM 側に残るもの（sabiruby、未計画）: キューごとの待ち手リスト・sleep 期限のヒープ・タスクが自分のいるキューを覚える（待ちタスクの O(N)）、`ireps` の解放。
