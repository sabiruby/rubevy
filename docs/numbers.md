# 数の一覧 — rubevy が持っている数と名前、その出どころ

計画書 `docs/plans/generalize-plan.md` の段階 **R10**。
対象は `src/lib.rs`、`src/reflect.rs`、`src/embed.rs`、`src/source.rs`、`src/prelude.rb`、`src/layers/camera.rb`、
`rubevy-build/src/lib.rs`、`examples/how_many_scripts.rs`、`examples/how_many_subscribers.rs`。
拾い方と選び方、出どころを探した過程は `docs/worklog/2026-09-20-numbers-inventory.md`。
行番号は `generalize` ブランチの R10 の時点のもの。

**既定値は 1 つも動かしていない。** R10 が変えたのは「変えられるかどうか」だけで、
`cargo test --workspace` は既定のまま 175 件通る（R10 前は 171 件 + 今回足した 4 件）。
出どころ不明の既定値をどう測れば導けるかの案は、この文書ではなく担当の報告にある — 動かすかどうかは著者が決める。

以後、rubevy に数を足す変更は、この表に 1 行足すことを含む。

---

## 0. 読み方

| 欄 | 意味 |
|---|---|
| 位置 | `ファイル:行`。定数なら定義行、無名の数なら書かれている行 |
| どこで変えるか | app が再ビルドせずに、あるいは 1 行で変えられる場所。`const` = 変えられない |
| 分類 | 下の (a)〜(e) |
| 出どころ | **測った**／**導出**／**引用**（文書・コミット・他所の仕様に書いてある）／*理由のみ*（理由らしきことは書いてあるが測った記録も導出も無い）／**不明** |

### 分類

* **(a) 不変量** — 変えると壊れる。`const` のまま。壊れる理由を 1 行添える。
  rubevy の (a) はほとんどが**名前**で、「2 か所に同じ綴りが書いてあって、両方が一致していなければならない」という形をしている。
* **(b) 既に設定できる** — app が変えられる。R10 がしたのは出どころを rustdoc に足すことだけ。
* **(c) 設定できなかった調整値** — R10 で設定できるようにした。**既定値は今の値のまま。**
* **(d) VM の数** — sabiruby（mruby-task）が決めていて、rubevy は訊くだけ。rubevy の側では変えられないし、変えるべきでもない。
* **(e) 計測器の数** — `examples/how_many_*.rs` の既定値。R5 が全部に出どころを rustdoc で添えた。

### 入れなかった基準

一覧に**入れなかった**のは、数そのものに意味が無いもの:

1. **0 と 1**（`= 0` の初期化、`+= 1` の計数、`.max(0.0)`、`unwrap_or(0)`）。
   ただし `subscribe(limit:)` の下限 1 は入れた — それは「これ以下は間違い」という判定だから。
2. **引数の添字**（`a.get(1)`、`request.text(0)`、`request.entity_arg(0)`、`v[v.len() / 2]`、`find_all[0]`）。
3. **単位換算**（`delta * 1000.0 / unit_ms` の 1000、`as_nanos()` まわり、`Duration::from_nanos(16_666_667)` の桁）。
   16,666,667 ns という**値そのもの**は §7 に入れた（60 Hz という選択だから）。
4. **飽和・番兵としての型の限界**（`u64::MAX`、`i64::MAX`、`u32::MAX`、`saturating_add`、`min()` のクランプ）。
   どれも「その型で表せるところまで」であって、選んだ数ではない。
5. **恒等元**（`@zoom = 1.0`、`offset ||= [0.0, 0.0]`、`@base / @zoom` の初期値）。
   1 倍・ずれ無しは「何もしない」の綴りであって調整値ではない。
6. **文字列の中の数**（`"not a RITE binary: {0}"`、`0.5.2` のような版の言及、rustdoc の本文に出てくる測定値）。
7. **テストとアサーションの期待値**（`tests/` の数は本体の数の写し）。
8. **`src/source.rs` の桁の走査**（`0..line.len()`、`colon + 1`）。エラー文 `FILE:LINE:COL:` を読む字の位置であって、設定ではない。
   `src/embed.rs` の数値リテラルは `self.len() == 0` の 1 行だけで、これも入れていない。

迷ったら入れた。迷った理由は §10。

---

## 1. VM のフレームごとの数（`ScriptWorld<M>`）

| 名前 | 位置 | 既定値 | どこで変えるか | 分類 | 出どころ |
|---|---|---|---|---|---|
| `budget` | `src/lib.rs:939` | 200,000 命令 | `ScriptWorld::budget`（`pub` フィールド） | (b) | **不明**。スケジューラごと入ってきた（`4c1e89f`「v1: one VM, one task per script」）。コミットにも `docs/after-gems-plan.md` にも選び方は無い。**買えるものは測ってある**: 読み 1 回 75 命令（`worklog/2026-09-17-sync-reads.md`）、購読 1 件 51 命令（`verification/scale.md`）、拒まれた書き 1 件 33 命令（`worklog/2026-09-20-rejected-writes.md`） |
| `frame_time` | `src/lib.rs:990` | 8 ms | `ScriptWorld::frame_time` | (b) | **不明**。`overrun` と同じコミット（`0d0662b`）で、2 つの数を述べるだけで理由は無い。60 Hz の 16.7 ms の約半分だが、そう書いた人はいない |
| `overrun` | `src/lib.rs:1000` | 50 ms | `ScriptWorld::overrun` | (b) | **不明**（同上）。記録にあるのは**関係**だけ: `frame_time` より大きいのは意図的（切り替えられないスクリプトのための最後の網） |
| `queue_limit` / `ScriptWorld::<()>::QUEUE_LIMIT` | `src/lib.rs:1025`, `:1924`, `:1936` | 64 件 | `ScriptWorld::queue_limit`（VM ごと）、`Rubevy.subscribe(name, limit: n)`（購読ごと） | (b)（R3 で (c) から移った） | *理由のみ* + **測った上下限**。64 を選んだ記録は `worklog/2026-09-15-events.md` の定性的な 1 文だけ。R3 が測ったのは上と下で、**64 はそのどちらからも導かれていない**（下の §1.1） |
| `max_depth` / `reflect::DEFAULT_MAX_DEPTH` | `src/reflect.rs:52`, `src/lib.rs:1857`, `:1865` | 16 段 | `ScriptWorld::set_max_depth` / `max_depth`（**R10 で足した**） | **(c) → 設定できる** | **不明**。リフレクションの橋ごと入ってきた（`0cf047c`、2026-09-15）。*限度を置く理由*は書いてある（境界の意味は「スクリプトが読める Hash」であること、循環がスタックを持っていくこと）が、**16 という数**は測っても導いてもいない |

### 1.1 既定値 64 について（著者判断済み: 動かさない）

R3 が測ったのは次の 2 つで、**どちらも 64 を指していない**。rustdoc と併せてここに書くのは、
「64 は測って導いた数ではない」と言い切るためである。

* **上の限界**: 購読 1 本が 1 フレームに読み切れるのは**約 3,800〜3,900 件**。止めているのは上限ではなく既定の予算で、
  1 件あたり約 51 命令（`docs/verification/scale.md`、`worklog/2026-09-20-overflow-and-limits.md`）。
  つまり 64 は「1 フレームで読める量」の約 1/60 である。
* **下の限界**（読まない購読が抱える最悪のメモリ）: 上限 × 1 件 × 購読数。1 件は数値 16〜21 B、
  短い文字列 307〜335 B、Float 4 個の配列 333〜521 B。読まない購読 1,000 本 × 64 件で 1.0 MB（数値）〜20.5 MB（文字列）。

**正しい値は payload と購読数で決まり、rubevy はそのどちらも知らない。** だから案内は
「ゲームに合わせて選ぶ」で、選ぶための 2 つの数が上のこれである。R3 の担当が添えた
「4 MB・購読 200 本・1 件 320 B なら 62」は**後から 64 に合わせて作った式**なので、出どころとしては書かない。

---

## 2. タスクと時計（VM が決めている数）

| 名前 | 位置 | 値 | どこで変えるか | 分類 | 出どころ |
|---|---|---|---|---|---|
| `Script::priority` の既定 | `src/lib.rs:158`, `:180` | 128 | `Script::with_priority(n)`（スクリプトごと） | (b) | **引用**: mruby-task の `MRB_TASK_PRIORITY_DEFAULT`（sabiruby `src/builtins/ext_task.rs:43` の `PRIORITY_DEFAULT`）。rubevy が選んだ数ではない |
| 子タスク（`Task.new`）の優先度 | — | 128（同じ） | Ruby 側（`Task.new(priority: n)`） | (d) | **引用**: 同じ `PRIORITY_DEFAULT`（sabiruby `ext_task.rs:631`、`Value::Nil => PRIORITY_DEFAULT`）。**スクリプトとの差は 0** — カメラ層の `follow` の子タスクは親のスクリプトと同じ優先度で走る |
| 時計の 1 tick | `src/lib.rs:2619`（`task_tick_unit_ms()`） | **4 ms** | 変えられない（VM の数） | (d) | **引用**: sabiruby `ext_task.rs:39` の `TICK_UNIT_MS`、コメントに `MRB_TICK_UNIT` とある。rubevy は書き写さず毎回訊いているので、VM が変えれば一緒に動く。`sleep 0` が「次のフレーム」ではなく「時計が次に進むまで」なのはこの粒度 |
| タイムスライス | — | 3 tick | 変えられない（VM の数） | (d) | **引用**: sabiruby `ext_task.rs:41` の `TIMESLICE`（`MRB_TIMESLICE_TICK_COUNT`）。rubevy は触れていない |

---

## 3. プラグインと `require`

| 名前 | 位置 | 既定値 | どこで変えるか | 分類 | 出どころ |
|---|---|---|---|---|---|
| `RubevyPlugin::asset_root` | `src/lib.rs:2312` | `"assets"` | `RubevyPlugin::with_asset_root` / `for_vm` | (b) | **引用**: bevy の `AssetPlugin::default()` が読む所。rubevy が選んだ名前ではない |
| ロードパスの形 | `src/lib.rs:2349` | `["{root}/scripts", "{root}"]` | `ScriptWorld::require_from(host, load_path)`、`vm.set_load_path` | (b) | **不明**。`require` ごと入ってきた（`4c1e89f`）。なぜ `scripts` という名前なのか、なぜ 2 段なのかはどこにも無い |
| `.mrb` の拡張子 | `src/lib.rs:134` | `["mrb"]` | 変えない | (a) 変えると `MrbLoader` がアセットを拾わない | **引用**: RITE バイナリの拡張子（mruby の `mrbc -o` の慣例） |

---

## 4. 名前 — 2 か所で綴りが一致していなければならないもの

数ではないが同じ扱いにする（計画書 3.10-1）。どれも「Rust 側と Ruby 側の両方に同じ字が書いてあり、
片方だけ変えると**黙って**壊れる」という形をしている。だから `const` のままにする（最後の 1 行だけは
層の中なので開き直せて、分類は (b)）。

| 名前 | 位置 | 値 | 相手 | 壊れ方 | 出どころ |
|---|---|---|---|---|---|
| `ENTITY_IVAR` | `src/lib.rs:2408` | `@rubevy_entity` | `src/prelude.rb`（`Task.new` が子タスクに写す）、`docs/host-api.md`（スクリプトが自分で書いてよいと案内している） | `Rubevy.entity` / `ask` / `subscribe` が「エンティティが無い」と言う | *理由のみ*: 「a script would not write it by accident」（`:2405`） |
| `DROPPED_IVAR` | `src/lib.rs:2419` | `@rubevy_dropped` | `src/prelude.rb` の `Subscription#dropped` | 落とした件数が常に 0 に見える | *理由のみ*: `ENTITY_IVAR` に揃えた（`:2415-2418`） |
| `RESERVED_KINDS` の 6 つ | `src/lib.rs:2892` | `component.get` / `component.has` / `components` / `entities.with` / `resource.get` / `writes.rejected` | `src/prelude.rb` の `Rubevy.ask(...)` | 問いが rubevy ではなくゲームの `take_requests` に渡り、誰も答えずスクリプトが永久に park する | *理由のみ*: 綴りの約束（`:2885-2891`）。点区切りにした理由はどこにも無い |
| `$rubevy` とそのキー | `src/lib.rs:2768` | `$rubevy`、`:frame` / `:delta` / `:time` | 全スクリプト、`src/layers/camera.rb:271` | スクリプトが読むものが nil になる | **引用**: `4c1e89f`「`$rubevy` (`:frame`, `:delta`, `:time`) replaces `$frame`/`$delta`」 |
| `ENTITY_TAG` | `src/lib.rs:3513` | 1 | `set_on_free` の判定と `data_new` の作成 | **名前であって大きさではない**。どの数でもよいが、両方が同じでなければ別種の Data をエンティティと取り違える。`pub` なのは、ホストが自分の Data に別の番号を選べるようにするため | **不明**（なぜ 1 か。ただし「どの数でもよい」ので、不明であることが問題にならない唯一の行） |
| `AS_ARRAY` の 5 型 | `src/reflect.rs:65` | `glam::Vec2` / `Vec3` / `Vec3A` / `Vec4` / `Quat` | bevy_reflect の型パス | 読みの形が変わり、`tf[:translation][0]` が書けなくなる | **引用**: bevy_reflect 0.19 がこれらを struct として綴っているという事実（`bevy_reflect/src/impls/glam.rs`）。表であって調整値ではない |
| `camera.world_at` の kind 名 | `src/layers/camera.rb:187` | `"camera.world_at"` | 答える側の app（`examples/camera_from_ruby.rs`） | 答え手が見つからず、`pop` したタスクが永久に park する。層を開き直せば変えられるので分類は (b) | **引用**: R9（`worklog/2026-09-20-camera-layer.md`。カメラは複数ありうるのでエンティティを第 1 引数に足した） |

---

## 5. Ruby 側（`src/prelude.rb`、`src/layers/camera.rb`）

`prelude.rb` に数は 1 つも無い（数値リテラルを含む 7 行は全部コメントの中の言及）。
`camera.rb` の数はほとんどが「何もしない」の綴り（§0 の基準 5）で、残りは引数の既定値である。

| 名前 | 位置 | 値 | どこで変えるか | 分類 | 出どころ |
|---|---|---|---|---|---|
| `subscribe(limit:)` の下限 | `src/lib.rs:3791` | 1 以上 | 変えない | (a) 0 以下の上限を頼むのは数え間違いで、`ArgumentError` にするのが R3 の判断 | **引用**: `ScriptWorld::queue_limit` の rustdoc（`queue_limit = 0` は「最新の 1 件だけ」という別の意味を持つ） |
| `Rubevy::Camera.markers` とその順 | `src/layers/camera.rb:29` | `[:Camera2d, :Camera3d]` | 層を開き直す（`Rubevy::Camera.markers` を定義し直す） | (b) | **引用**: bevy の 2 つの marker component。順は「2 つあるときは 2D が先」で、`find` の rustdoc に書いてある |
| `follow` の `every` の既定 | `src/layers/camera.rb:206` | 0（＝時計が進むたび） | 呼び出しの引数 | (b) | *理由のみ*: 「Nothing here decides how fast or how far behind a camera should follow」（`camera.rb:193-195`）。0 は「毎回」であって調整値ではない |
| `follow` の `offset` の既定 | `src/layers/camera.rb:211` | `[0.0, 0.0]` | 呼び出しの引数 | (b) | **引用**: R9 が「今のずれを保つ」から「ずれ無し」に変えた（`worklog/2026-09-20-camera-layer.md`） |

---

## 6. `rubevy-build` の既定の名前（全部 (b)）

数ではないが同じ扱いにする（計画書 7 章 R6 の行）。3 つとも `Embed` のビルダで変えられる。

| 名前 | 位置 | 既定値 | どこで変えるか | 出どころ |
|---|---|---|---|---|
| 表の `static` の名前 | `rubevy-build/src/lib.rs:62` | `RUBY_FILES` | `Embed::name` | **引用**: 2 本のサンプルゲームの `build.rs` がそう書いていた。それ以上の理由は無い — そして**それが理由でよい**（その名前のままなら、ゲームは `include!` の側を書き換えずに `build.rs` を消せる） |
| 書き出すファイル名 | `rubevy-build/src/lib.rs:63` | `ruby_files.rs` | `Embed::out_file` | **引用**: 同上 |
| 拾う拡張子 | `rubevy-build/src/lib.rs:61` | `["rb"]` | `Embed::extensions`（`.as_bytes()` と組で `["mrb"]`） | **引用**: 同上。`.rb` なのは「ソースを `require` する」ための表だから（`導出`寄りの引用） |

---

## 7. 計測器の既定値（`examples/how_many_*.rs`、全部 (e)）

R5 が常設したときに、**全部の `const` の rustdoc に出どころが書いてある**。ここはその要約と、
「引数で変えられるもの」と「行を選べるだけのもの」の区別である（§9-2）。

### 7.1 `how_many_scripts.rs`

| 名前 | 位置 | 既定値 | 引数で | 出どころ |
|---|---|---|---|---|
| `SCRIPTS` | `:79` | `[10, 100, 300, 1000, 3000]` | **選べるだけ**（第 3 引数がこの中の 1 つに絞る） | **引用**: 調査（10 と 100 は 2 本のゲームの実数、1000 はこの機械で 60 Hz のフレームを使い切るあたり、3000 は意図的にその先） |
| `SLEEPS` | `:84` | `[0.004, 0.25]` | **選べるだけ**（第 4 引数） | **導出**: 0.004 は 1 tick（`TICK_UNIT_MS`）＝ 60 Hz の次フレームで起きる最短、0.25 は約 15 フレーム |
| `FRAMES` | `:87` | 90 | 第 1 引数 | **引用**: 調査の `factory_machines 90 3` |
| `REPEATS` | `:92` | 3 | 第 2 引数 | **引用**: 調査以降ずっと 3 |
| `SETTLE` | `:97` | 30 フレーム | `const` | **導出**: `SLEEPS` の最長が約 15 フレームで、その 2 回ぶん |
| `FRAME` | `:101` | 16,666,667 ns | `const` | **引用**: 60 Hz |
| `mem` モードの既定 | `:452-453` | 1000 台 / 60 フレーム | 第 2・第 3 引数 | **不明**（この 2 つだけ rustdoc が無い。直書き） |
| `Limits::Default` の 2 つ | `:282-283` | 200,000 / 8 ms | `const` | **引用**（§1 の既定値の**写し**）。写しなので連動しない — §9-1 |
| `Limits::Generous` | `:286-287` | 100,000,000 / なし | `const` | *理由のみ*: 「五百倍で、時計は無し」（`:256-258`）。どちらの上限もフレームを終わらせないことが目的 |
| `Limits::Tight` | `:291` | 200,000 / 1 ms | `const` | **導出**: 既定の 1/8（`:259-261`）。tick がそれに合わせて縮まなければ、使っているのは答えの中ではない |
| `Limits::Clocked` | `:293-` | 100,000,000 / 1 s | `const` | **導出**: `generous` と、時計を読むこと以外は同じにするため（`:262-266`） |

### 7.2 `how_many_subscribers.rs`

| 名前 | 位置 | 既定値 | 引数で | 出どころ |
|---|---|---|---|---|
| `SUBSCRIBERS` | `:71` | `[1, 10, 100, 1000]` | `const` | **引用**: 調査 |
| `PER_FRAME` | `:76` | `[10, 100, 1000]` | `const` | **導出**: 調査の 1 の列を落とした（計器自身の時計 2 回がその 1 件に丸ごと乗るため） |
| `FRAMES` / `REPEATS` | `:79-80` | 60 / 3 | 第 2・第 3 引数 | **引用**: 調査の `factory_events 60 3` |
| `QUEUE_LIMITS` | `:85` | `[1, 16, 64, 128, …, 8192]` | `const` | **引用**: R3。既定の 64 とその前後の 2 冪、最後は予算がスクリプトに読ませる量を超えたところ |
| `LIMIT_MODE_PER_FRAME` | `:89` | 8192 | `const` | **導出**: `QUEUE_LIMITS` の最大（最後の行以外は上限が縛るようにするため） |
| `MEM_LIMIT` / `MEM_SUBSCRIBERS` | `:94-95` | 1000 / 10 | 第 2・第 3 引数 | *理由のみ*: 「重さを測る価値のある件数」「1 プロセスの RSS がノイズより大きく動く本数」（`:91-93`）。数そのものは**不明** |
| `TURNS`（計器の値段） | `:237` | 100,000 回 | `const` | **導出**: 1 組が数十 ns なので数 ms、周回 1 組ぶんが 100 万分の 1（`:234-236`） |

---

## 8. 集計

数えたのは §1〜§7 の表の行で、**44 件**。

### 分類ごと

| 分類 | 件数 |
|---|---|
| (a) 不変量 | 8 |
| (b) 既に設定できる（出どころを rustdoc に足しただけ） | 14 |
| **(c) 設定できなかった調整値 → R10 で設定できるようにした** | **1** |
| (d) VM の数（rubevy のものではない） | 3 |
| (e) 計測器の数 | 18 |
| **合計** | **44** |

### 出どころごと

| 出どころ | 件数 |
|---|---|
| **測った**（日付・条件つきの実測がある） | **0** |
| **導出**（既にある制約から出ている） | 7 |
| **引用**（文書・コミット・他所の仕様に書いてある） | 23 |
| *理由のみ*（理由らしきことは書いてあるが測った記録も導出も無い） | 7 |
| **不明** | **7** |
| **合計** | **44** |

**「測った」が 0 件**である。これはこの一覧のいちばん大きな事実で、
**rubevy の既定値は 1 つも測って決められていない** — 測ってあるのは既定値が「買えるもの」
（1 フレームに何件読めるか、1 件が何バイトか、tick が何 ms になるか）であって、
既定値そのものではない。R3・R4・R5 が測ったのは全部その前者である。
「引用」が 23 件と多いのも同じ話の裏側で、rubevy の数の半分は**他所（mruby-task、bevy、調査の表）から来た数**で、
rubevy が自分で決めた数は少ない。

### 出どころ不明の 7 件

| 何 | どこ | 影響 |
|---|---|---|
| `budget` 200,000 | `src/lib.rs:939` | 毎フレーム、全ゲーム |
| `frame_time` 8 ms | `src/lib.rs:990` | 毎フレーム、全ゲーム |
| `overrun` 50 ms | `src/lib.rs:1000` | 切り替えられないスクリプトが出たフレームだけ |
| `max_depth` 16 | `src/reflect.rs:52` | 読み書き 1 回ごと。ただし bevy の component は 2〜3 段なので普通は当たらない |
| ロードパスの `scripts` という名前と 2 段という形 | `src/lib.rs:2349` | `require` のたび。名前なので「正しい値」は無い |
| `how_many_scripts` の `mem` モードの 1000 台 / 60 フレーム | `examples/how_many_scripts.rs:452-453` | 計測器だけ |
| `ENTITY_TAG` の 1 | `src/lib.rs:3513` | 無し（どの数でもよい。一覧の一貫性のために挙げている） |

`queue_limit` 64 は「不明」ではなく *理由のみ* に数えた — 選んだときの定性的な 1 文が残っているため（§1.1）。
`MEM_LIMIT` / `MEM_SUBSCRIBERS` も同じ扱い。

### 機械的に拾った数と、選んだ数

（R10 が rustdoc を足す**前**の状態で数えた。足した rustdoc の中の数は本文であって設定ではない。）

| どこ | `const` 宣言 | 数値リテラルを含む行 | 選んだ数 |
|---|---|---|---|
| `src/lib.rs` | 14（うち数値 3） | 139 | 15 |
| `src/reflect.rs` | 2（うち数値 1） | 22 | 2 |
| `src/embed.rs` + `src/source.rs` | 0 | 9 | 0 |
| `src/prelude.rb` + `src/layers/camera.rb` | 0（`NAME = 数` の行が 0） | 44 | 4 |
| `rubevy-build/src/lib.rs` | 0 | 3 | 3 |
| `examples/how_many_*.rs` | 17（うち数値 15） | 144 | 18 |
| sabiruby 側（rubevy にファイルが無い） | — | — | 2 |
| **合計** | **33（うち数値 19）** | **361** | **44** |

`const` は `grep -nE '^\s*(pub\s+)?const\s'`、リテラルは
`grep -nE '(^|[^A-Za-z0-9_.":])[0-9]+(_[0-9]+)*(\.[0-9]+)?'` で拾い、
rustdoc とコメントだけの行を落としてから目で選んだ。**リテラル 361 行から 44 件**（12%）。
`src/lib.rs` の選んだ数が拾った `const` より多いのは、名前のついていない数（`priority: 128`、
ロードパスの組み立て、`$rubevy` のキー）を入れたため。
落とした 9 割の内訳は §0 の「入れなかった基準」で、rubevy がゲームの語彙を知らない
（座標も速さも色も持たない）ぶん、games 側の一覧（1,246 行から 260 件、21%）より比率が低い。

`src/` だけを見たい人は **24 件**（44 − 計測器 18 − VM の数 2）を読めばよい。

## 9. 気づいた点（一覧の仕事の外。**直していない**）

1. **計測器の `Limits::Default` が既定値を写している。** `examples/how_many_scripts.rs:282-283` は
   `scripts.budget = 200_000; scripts.frame_time = Some(Duration::from_millis(8));` と直書きしていて、
   `ScriptWorld` の既定を読んでいない。既定値が動いた日に、`default(200k/8ms)` と名乗る行が
   **既定ではない設定**を測る。行の名札（`:272`）も同じ数を 3 度目に書いている。
   直すなら `ScriptWorld::new` の値をそのまま使い、名札はその値から組む。
2. **計測器の冒頭の「Every one of them is an argument」が正確ではない。**
   `how_many_scripts.rs:73` と `how_many_subscribers.rs:64` はそう書いているが、
   引数になっているのは `FRAMES` / `REPEATS` と `mem` モードの 2 つだけで、
   `SCRIPTS` と `SLEEPS` は**行を選べるだけ**（一覧に無い台数は指定できない）、
   `SUBSCRIBERS` / `PER_FRAME` / `QUEUE_LIMITS` / `SETTLE` / `FRAME` は `const` のままである。
3. **深さを超えた書きの断り文句が、深さの話をしない。** `max_depth` を超える Hash は
   **VM から読み出す側**（`reflect::read_ruby`）で先に切られ、境界の 1 つ先のエントリは
   鍵も値も nil になって届く。だから `apply_at` の `too deep` には（この道からは）決して届かず、
   スクリプトが `Rubevy.rejected_writes` で読むのは
   `b.c: a field name must be a Symbol or a String` である（`tests/max_depth.rs`）。
   場所は正しいが、文は nil のことを言っていて深さのことを言っていない。R10 より前からの性質。
4. **`docs/host-api.md` の「Time」の表に `queue_limit` と `max_depth` が無い。** R10 で出どころの列を足したが、
   表が並べているのは `RunLimits` に渡る 3 つだけで、`ScriptWorld` の `pub` な数はいま 4 つある。
   「Events」と「Components by name」にそれぞれ書いてあるので重複させるかどうかは判断が要る。

---

## 10. 分類に迷った数と、迷った理由

1. **`max_depth` は (a) か (c) か。** rustdoc が挙げる理由のうち「循環がスタックを持っていく」の方は
   **不変量寄り**で、大きくしすぎると本当に壊れる（再帰 1 段が 1 スタックフレーム）。
   それでも (c) にしたのは、もう一方の理由「境界の意味はスクリプトが読める Hash であること」が
   **方針**であって、方針は app のものだから。壊れ方（スタック）は rustdoc に書いた。
2. **`ENTITY_TAG = 1` を「数」に数えるか。** 数えた。値は 1 でなくてもよく、その意味では添字に近い（§0-2 で落とす側）が、
   `pub` で外に見えていて「ホストは別の番号を選べ」と rustdoc が言っている以上、
   **外の利用者が見る番号**なので表に出す方がよいと判断した。
3. **ロードパスの `scripts` を (b) にするか (c) にするか。** (b) にした。
   `ScriptWorld::require_from` と `vm.set_load_path` でロードパス全体を置き換えられるので「変えられない」ではない。
   ただし**「`{root}/lib` を 1 つ足したい」だけの app も全体を綴り直す**ことになる。
   足す口を作るかどうかは、欲しい利用者が出てからでよいと考える（R6b の判断と同じ向き）。
4. **`Rubevy::Camera` の数を rubevy の数と数えるか。** 数えた（§5）。層は app が読むかどうかを決める Ruby で、
   rubevy の `src/` に入っている以上 rubevy が配っている数である。
   ただし**層は開き直せる**ので、分類はどれも (b) になった — 「設定できない調整値」は層には 1 つも無い。
5. **計測器の数を一覧に入れるか。** 入れた（§7、18 件で全体の 41%）。
   `examples/` は `src/` ではないが、外の利用者が「自分の機械で何本載るか」を測るために読むものなので、
   その既定値の出どころは rubevy の約束の一部だと判断した。
   ただし §8 の集計は (e) を分けてあるので、`src/` だけを見たい人は 24 件を見ればよい。
6. **`Script::priority` の 128 を「引用」にするか「不明」にするか。** 引用にした。
   rubevy がこの数を選んだ記録は無いが、`MRB_TASK_PRIORITY_DEFAULT` と**一致している**ことは
   sabiruby のソースで確かめられ、一致していること自体が意味を持つ（子タスクと同じ優先度になる）。
   「他所の仕様に書いてある」を引用に数える、という §0 の定義に当てはまる。
