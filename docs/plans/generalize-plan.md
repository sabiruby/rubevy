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
   数だけでなく**埋め込みの既定の名前**（`rubevy-build` の `RUBY_FILES` / `ruby_files.rs` / `.rb`、ロードパスの `scripts`、`$rubevy` のキー）も同じ表に入れる。
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
| R0 | **済み**（2026-09-20、ブランチ `generalize` の `bf45bde`、`src/` 無変更）。3 つとも書ける: `Rubevy.find(:Camera2d)`、`cam[:Transform] =`、ズームは `cam[:Projection] = { Orthographic: [ { scale: 2.5 } ] }`（変種名の Hash → tuple 変種なので **Array** → 中の struct への部分書き）。通らない形 3 つもテストにした: 変種の切り替え、tuple 変種のフィールドを名前で書く、同じ tick の 2 回のズーム（書きはフレーム末尾）。`tests/camera.rs` 6 件、全体 70 passed。記録は `docs/worklog/2026-09-20-camera-from-ruby.md` |
| R6 | **済み**（2026-09-20、`generalize` の `cb8bf00`）。`Program::new(prelude, name, body, tail)` と `prelude_lines`、`in_the_authors_lines`（garden の 7 ケースを移した。エラー文の形式はネイティブとブラウザの橋の両方の実物で確かめた — 同じ `CompileError` の `Display` で、違いはファイル名が `playground.rb` に固定されることだけ）、`replace_script`、`EmbeddedHost`（`compile_with` に渡す関数は `Host::compile` と同じ形）、build ヘルパ `rubevy-build`（std のみ・依存 0。repo を workspace にした影響は実測で小: 根の `cargo test` 不変、`Cargo.lock` +4 行、`cargo package --list` 97 ファイルのまま）。101 passed（着手前 81）、wasm の lib ビルド可、依存の追加なし、既存の API は不変。未確認: build.rs → `include!` → `EmbeddedHost` の通し（最初の客は games の S2）。記録は `docs/worklog/2026-09-20-shared-entry-points.md` |
| R7 | **sabiruby 側は済み**（2026-09-20、sabiruby のブランチ `declare` の `2f1043f`、worktree `sabiruby-wt-declare`。VM の crate は無変更、unsafe 0、数 0）。`sabiruby_serde::declare`: `Declarations::<T>::install(&mut vm).define(&mut vm, "unit")` → `load_and_run` → `take(&mut vm) -> Vec<(String, T)>`（宣言順、取り出した後 VM に何も残らない — `live_count` が宣言しなかった VM と一致）。重複は `ArgumentError`、上書きは別名の口 `define_replacing`。エラーは `missing field \`scale\` (TypeError) at data.rb:2` の形。読み返しは `expose(&mut vm, "unit_of", table)`（取り出した後の**別の表**。間にホストの検証が入るため）。表は `T` の host store に置く（使われない `Data` tag を型ごとに 1 つ消費）。serde のテスト 20 → 36、workspace 221 → 237、`thumbv7em-none-eabi` で `no_std` ビルド可。**main への取り込みと push は著者の指示待ち**（games が `[patch]` の git 経由で使うには push が要る）。**rubevy 側の節と example は `sabiruby-serde` の公開待ち**: rubevy の dev-dependency は crates.io の 0.1.0 で、`declare` が無い版に対して example をコミットすると clone しただけでは `cargo test` が通らなくなる |
| R1 | **済み**（2026-09-20、`generalize` の `c66d220`）。`HostState.subscriptions` を名前 → 購読者の `HashMap` に。解除でキューを閉じる順が `HashMap` で変わらないよう購読の通し番号 `seq` で並べ直す（閉じる = 待っているタスクを起こす、なので順が動きに出る）。公開 API 不変、依存の追加なし、新しい数なし。誰も聞いていない名前への publish 1 件: 購読 1000 本で **228 → 9.6 ns**（同じ日・同じ条件で前後を取り直し。100 本で 31 → 8.9 ns、1 本では 7.9 → 13.8 ns とわずかに高い — 名前のハッシュ代。釣り合うのは 10〜30 本）。聞く人がいる publish とフレーム時間は前の 2 回のぶれ（±1〜2%）の中。110 passed。保留: `1 購読 × 1000 件` の `publish_heard` は走るたびに ±40% 動くセルで、判断していない。記録は `docs/worklog/2026-09-20-subscription-index.md` |
| R2 | 実行中 |
| R3〜R5 | 未着手。計測を伴うので 1 本ずつ |
| R8 | **済み**（2026-09-20、`generalize` の `52bfe7b`）。`Rubevy.resource(:Score)`（tick の中で返る、無ければ nil）と `Rubevy.set_resource(:Score, { points: 8.0 })`（名前を挙げた分だけ、フレーム末尾）。ジェネリックは型引数ごと綴る（`"Time<Virtual>"`。`Time` は `Time<()>` で、`:Time` は nil）。Rust の公開 API は 0 個増、依存の追加なし、unsafe 0、新しい数 0。R0 の 3 件も直した（warn のコロン 18 か所、tuple 変種の書きの文書、型の登録は `reflect_auto_register` feature だと確かめて `host-api.md` を差し替え — `bevy_time` / `bevy_pbr` のように手で登録するプラグインも残る）。109 passed（着手前 101）、wasm の lib ビルド可。読み 1 回: component 2.38〜2.45 µs / resource 2.24〜2.66 µs（静かな機械、3 回。差はぶれの中。止めているのは命令の予算で、75 命令と 69 命令）。記録は `docs/worklog/2026-09-20-resources-by-name.md` |
| R9〜R10 | 未着手（R9 の前に著者判断 3 つ: 拒まれた書きを知る口、ズームの向き、答え手のいない問い） |

VM 側に残るもの（sabiruby、未計画）: キューごとの待ち手リスト・sleep 期限のヒープ・タスクが自分のいるキューを覚える（待ちタスクの O(N)）、`ireps` の解放。

## 7. 気づいた点（段階の報告から本体が集める）

実装担当は、仕事の範囲の外で気づいたことを直さずに報告と worklog の末尾に書く（`implementer.md` の報告の形式 6）。
本体は段階をレビューするたびにここへ写し、行き先を決める。消さずに「状況」を更新する。

| 日付・段階 | 気づいた点 | どこ | 属する先 | 状況（計画に足した／著者判断待ち／見送り・理由） |
|---|---|---|---|---|
| 09-20 R0 | enum の書きが失敗したときの warn が `not written whole: : the fields of …` とコロンで始まる（component 直下では `path` が空） | `src/reflect.rs:460-462` ほか `:449` `:456` `:486` `:489` | rubevy（表示のバグ） | 計画に足す: R8 のついでに直す |
| 09-20 R0 | 書きが拒まれたことがスクリプトから分からない（`warn!` だけ、`Entity#set` は渡した値を返す）。カメラ層で変種が想定と違うと `zoom` が黙って効かない | `src/lib.rs:2124-2127`、`src/prelude.rb:40-43` | rubevy（口の設計） | **著者判断待ち**: 「最後の書きの問題」を読める口を足すか。R9 の前に |
| 09-20 R0 | `host-api.md` に tuple 変種の書きの規則が無い（Array でしか書けない）。読みの表も struct 変種と一括り | `docs/host-api.md:422-436` | rubevy（文書と実物のずれ） | 計画に足す: R8 で 1 文 |
| 09-20 R0 | `host-api.md` の「`DefaultPlugins` does it」: bevy 0.19.1 の `bevy_camera` / `bevy_transform` に `register_type` は無く、登録しているのは `reflect_auto_register` feature。どちらが効いているかは未確認 | `docs/host-api.md:406-408` | rubevy（文書、小） | 計画に足す: R8 で確かめて直す。games は `DefaultPlugins` なので F0 でも確かめる |
| 09-20 R0 | repo に rustfmt の設定が無く、`cargo fmt --check` が既存コードで落ちる。担当が `cargo fmt` を走らせると無関係な差分が出る | repo の根 | repo の作法 | **著者判断待ち**: 設定を置くか「fmt は使わない」と書くか。それまで担当には「`cargo fmt` を走らせない」と伝える |
| 09-20 R0 | 誰も答えない `Rubevy.ask(...).pop` は永久に park する。任意の Ruby 層の `world_at` に答え手がいないときの振る舞いが決まらない | `Rubevy.ask` | rubevy（口の設計） | R9 は「投げっぱなしでキューを返す」で始める。答え手の有無を聞ける口は**著者判断待ち** |
| 09-20 R1 | **計測の罠 2 つ**: (1) 2 つの worktree を同じ `CARGO_TARGET_DIR` で建てると 2 つ目の example が建たず 1 つ目のバイナリが残る（md5 が一致して気づいた）。(2) 隣の担当の `docker run … cargo build` が `pgrep -af "cargo|rustc"` に出る前後で p95 が跳ねた（28.5 ms 対 12.2 ms） | 計測の手順 | repo の作法／R5 の材料 | 計画に足した: R2 以降の依頼文に「前の版は別の target で建て、md5 で別物だと確かめる」。R5 の `docs/verification/` の再現手順にも書く |
| 09-20 R1 | 計測器の P=1 の列には計器の値段（`Instant::now()` 2 回、100〜1300 ns）がそのまま乗る | `examples/factory_events.rs`（未コミットの計測器） | rubevy（R5 の材料） | 計画に足す: R5 で常設するとき、1 フレームぶんをまとめて測って割るか、P=1 を出さない |
| 09-20 R1 | `Subscription.entity` が `Option<Entity>` なのに `None` は作られない（作る場所は `Rubevy.subscribe` の 1 か所で、エンティティが無ければ `ArgumentError`） | `src/lib.rs` の `Subscription` | rubevy（型が緩い、非公開） | 見送り（非公開の型。R3 が同じ場所を触るとき、ついでに直してよい） |
| 09-20 R1 | `ScriptWorld::publish` の rustdoc は前から「誰も購読していない名前には何も積まれないので自由に publish してよい」と書いていた。R1 の前は、その「自由に」が購読の総数ぶんかかっていた。文書のほうが先に正しかった例 | `src/lib.rs` の `publish` の rustdoc | 本の素材 | book の findings に写す |
| 09-20 R8 | **書きが拒まれたことが分からない問題が 2 か所になった**（component と resource。どちらも `warn!` だけで Ruby には nil） | `src/lib.rs` の `apply_component_writes` / `apply_resource_writes` | rubevy（口の設計） | **著者判断待ち**（R0 の同じ行と 1 つの判断）。足すなら両方を見る形で |
| 09-20 R8 | `reflect_cache` / `resource_cache` は、型が登録し直されると古いまま（ホットリロードや mod のロード順で起きうる。普通のゲームでは起きない） | `src/lib.rs` | rubevy（既存の性質） | 見送り。R10 でキャッシュの寿命を rustdoc に 1 行 |
| 09-20 R8 | resource の読みは `$rubevy`（frame / delta / time）と役割が重なる。どちらを勧めるか | `docs/host-api.md` | 計画書（R10 の範囲） | 計画に足す: R10 で `$rubevy` のキーを棚卸しするときに書く |
| 09-20 R8 | `host-api.md` の「Time」節が `Time<Virtual>` を名前で読めることを知らない | `docs/host-api.md` | 文書 | 計画に足す: R4 がこの節を書き直すときに 1 行 |
| 09-20 R8 | bevy 0.19 で `Components::get_valid_resource_id` ほか 2 つが deprecated（resource が component になったため）。rubevy は `get_valid_id` を使っていて影響なし | `bevy_ecs-0.19.1/src/component/info.rs:608,632,686` | 本の素材／games への注意 | 見送り（記録のみ） |
| 09-20 R6 | **`EmbeddedHost` を `set_host` したら `set_load_path` も書き直す必要があるのに、型に現れない**。プラグインは `{root}/scripts` と `{root}` を入れるので、忘れると `require` が静かに LoadError になり、ブラウザでだけ起きる | `src/lib.rs` の `RubevyPlugin::build` | rubevy（口の設計） | **著者判断待ち**: host とパスを一緒に受ける口（例 `ScriptWorld::embed(host, paths)`）を足すか。S2 の実感を見てから |
| 09-20 R6 | `Program::new(prelude, name, body, tail)` は `&str` 4 本で、取り違えても型が通る（`name` と `body` を逆にすると静かに 1 行のプログラムができる） | `src/source.rs` | rubevy（API の使いにくさ） | S2 で使ってみてから。直すならビルダを**足す** |
| 09-20 R6 | `tests/embedded_host.rs` が生成物 `assets/scripts/helper.mrb`（Docker の mrbc で作る）を `include_bytes!` している。`helper.rb` を変えて作り直し忘れると古いバイトコードで通り続ける | `tests/embedded_host.rs`、`tools/compile_scripts.sh` | repo の作法 | 見送り（今ある他のテストと同じ性質）。R10 のついでに、生成物が古いと落ちる確認を足せるか見る |
| 09-20 R6 | `rubevy-build` の既定の**名前** 3 つ（`RUBY_FILES` / `ruby_files.rs` / `.rb`）の出どころは「2 本のゲームがそう書いていた」だけ。数ではないが、R10 の一覧に入れるのか | `rubevy-build/src/lib.rs` | 計画書（R10 の範囲） | 計画に足す: R10 は数だけでなく「埋め込みの既定の名前」も一覧に入れる（全部引数で変えられることは確認済み） |
| 09-20 R7 | **キーワードを 1 つも書かない宣言（`item :iron_plate`）は `define_fn` では受けられない**（引数の数が固定。`src/convert.rs:440`）。`define_closure` で 1〜2 個を受ける。調査の「`(Symbol, Serde<T>)` で受けられる」は半分だけ正しかった | 調査 §2、計画 3.7 | 計画書の前提 | `Declarations` が吸収済み。rubevy 側の example はこの口を使うので影響なし |
| 09-20 R7 | 複数行に分けた宣言のエラー行は**最後の行**（SEND 命令の行番号）。「宣言の頭」ではなく閉じる行を指す | sabiruby の行番号の持ち方 | 本の素材／rubevy の文書 | R7 の rubevy 側の節に 1 行。book repo の findings に写す |
| 09-20 R7 | `Vm::backtrace(Some(mid))` はネイティブ名を先頭に足すが、例外が抱える `Exception#backtrace` には入らない（mruby は C フレームを直下の Ruby フレームに置く） | sabiruby `src/vm.rs:2570-2574`、`:2589-2600` | VM（小さな食い違い） | **著者判断待ち**（VM 本体の話） |
| 09-20 R7 | `HostStore::take` を「返さない」使い方（番号を手放さず値だけ取り上げる）は rustdoc が想定していない（「借りた側が `restore` する」）。今回の `Declarations::take` はこの使い方 | sabiruby `src/host_store.rs:124-138` | VM（rustdoc）／今回の設計の前提 | **著者判断待ち**: この使い方を認めて rustdoc に書くか、host store に「取り出して閉じる」口を足すか |
| 09-20 R7 | `tools/check_no_std.sh` は VM の lib しか見ていない。`sabiruby-serde` も `no_std` なのに対象外 | sabiruby `tools/check_no_std.sh:5-6` | sabiruby（確認の網） | **著者判断待ち**（小。対象に足すだけ） |
| 09-20 R0 | ズームの向き（`zoom 2` は寄るのか引くのか）と、2D は `scale`・3D は `fov` という数の違い | R9 の設計 | rubevy（Ruby 層） | **著者判断待ち**（R9 の前） |
