# 2026-09-20 — rubevy が持っている数の棚卸しと、設定できなかった 1 つ（R10 その 1）

計画書 `docs/plans/generalize-plan.md` の **R10**、前半。結果の表は `docs/numbers.md`。
この記録は、その表をどう作ったか — 何を機械的に拾い、何を落とし、出どころをどこまで探して
どこで「不明」と書くことにしたか、そして 1 つだけ設定できるようにした数をどう通したか。

---

## 1. 拾う

計画書が名指しした 9 ファイル（`src/lib.rs`、`src/reflect.rs`、`src/embed.rs`、`src/source.rs`、
`src/prelude.rb`、`src/layers/camera.rb`、`rubevy-build/src/lib.rs`、`examples/how_many_scripts.rs`、
`examples/how_many_subscribers.rs`）を 2 本の grep にかけた。games 側の一覧（`rubevy_games/docs/numbers.md`）と
同じ式を使ったのは、2 つの表を後で並べて読めるようにするため。

```
grep -nE '^\s*(pub\s+)?const\s' $FILES                                   # 33 行
grep -nE '(^|[^A-Za-z0-9_.":])[0-9]+(_[0-9]+)*(\.[0-9]+)?' $FILES        # 361 行
grep -nE '^\s*[A-Z][A-Z0-9_]*\s*=\s*-?[0-9]' src/prelude.rb src/layers/camera.rb   # 0 行
```

Ruby 側の `NAME = 数` が **0 行**だったのが最初の収穫で、これは garden の `prelude.rb`（`CRUISE`、
`DASH`、`ON_SLOTS` …）と正反対である。rubevy の Ruby はゲームの語彙を持たないので、定数にする数が無い。

`const` 33 行のうち数値は 19 行。残り 14 は名前（`ENTITY_IVAR`、`RESERVED_KINDS`、`PRELUDE`、
`layers::CAMERA`、`RubevySet` の 3 つと `const fn` 3 つ）で、名前も表に入れる約束（計画書 3.10-1）なので
そのまま候補に残した。

**リテラル 361 行から 44 件を選んだ**（12%）。落とした基準は `docs/numbers.md` §0 に 8 項目で書いた。
いちばん多く落ちたのは引数の添字（`a.get(1)`、`request.text(0)`、`request.entity_arg(0)`）で、
`src/lib.rs` の 139 行のうち 50 行ほどがこれである。次が 0 と 1 の初期化・計数。
`src/embed.rs` は数値リテラルを含む行が **1 行**（`self.len() == 0`）しか無く、`src/source.rs` の 8 行は
全部 `FILE:LINE:COL:` を読む字の位置だったので、この 2 ファイルからは 1 件も選んでいない。

迷ったのは 3 つで、`docs/numbers.md` §10 に 6 件として書いた。要点だけ:

* **`ENTITY_TAG = 1`**。値としては添字に近く（どの数でもよい）、基準 2 で落ちるはずだった。
  入れたのは `pub` だからで、rustdoc が「ホストは別の番号を選べ」と言っている以上、
  **外の利用者が見る番号**は表にある方がよい。
* **計測器の 18 件**。`examples/` は `src/` ではない。それでも入れたのは、外の利用者が
  「自分の機械で何本載るか」を測るのに読むファイルで、その既定値の出どころは約束の一部だから。
  集計では (e) として分けたので、`src/` だけ読みたい人は 24 件を見ればよい。
* **カメラ層の数**。層は app が読むかどうかを決める Ruby だが `src/` にある。入れた。
  ただし層は開き直せるので、分類は全部 (b) になった — 層には「設定できない調整値」が 1 つも無い。

## 2. 出どころを探す

各々について `git log -S'<数>' -- <ファイル>`、`docs/worklog/`、`docs/plans/`、`CHANGELOG.md`、rustdoc、
そして（VM 由来と思われるものは）sabiruby 0.5.2 のソースを見た。

いちばん効いたのは **`git log -S`** で、3 つの既定値が 2 つのコミットに固まっていることがすぐ分かった。

| 数 | 入ってきたコミット | そこに書いてあること |
|---|---|---|
| `budget` 200,000 | `4c1e89f`「v1: one VM, one task per script」 | 「the ready tasks run for a budget (`Vm::task_run_budget`)」— **数の理由は無い** |
| `frame_time` 8 ms / `overrun` 50 ms | `0d0662b`「time limits per frame」 | 「a time budget (8 ms) that cuts the running slice short, and an overrun limit (50 ms)」— **数を述べているだけ** |
| `max_depth` 16 | `0cf047c`「Components by name, through Bevy's reflection」 | 数に触れていない |

`4c1e89f` が参照している `docs/after-gems-plan.md` は rubevy の repo に無い。sabiruby 側にあったので
読んだが（`sabiruby/docs/plans/after-gems-plan.md:67, 84, 99`）、「1 フレームの上限は命令数の予算」という
**仕組み**の話しか無く、数は 1 つも書かれていない。`docs/worklog/2026-09-15-ecs-bridge.md` にも深さの話は無い。
**3 つとも「不明」と書く**ことにした。もっともらしい理由は後から作れる（8 ms は 16.7 ms の半分、
16 は 2 の 4 乗）が、それは記録ではない。`frame_time` の行には「60 Hz の半分だが、そう書いた人はいない」と
書き添えた — 読む人が同じ推測をするので、推測であることを先に言う方がよいと判断した。

なお R4 の担当が同じことに既に気づいていて、`docs/worklog/2026-09-20-frame-time-as-a-limit.md:314` に
「`ScriptWorld::frame_time` の既定 8 ms の出どころが rustdoc に無い（`budget` 200,000、`overrun` 50 ms も同じ）。
R4 でこのフィールドの rustdoc を書き直したが、既定値の理由は書けるものが手元に無かったので触っていない」と
残していた。R10 はその行の答えである — **理由は今も手元に無い**ので、無いと書いた。

反対に、**探したら他所にあった**数が 4 つあった。これが「引用」が 23 件と多い理由である。

* `Script::priority` の既定 **128** は sabiruby `src/builtins/ext_task.rs:43` の `PRIORITY_DEFAULT`
  （`MRB_TASK_PRIORITY_DEFAULT`）と**同じ数**だった。さらに `ext_task.rs:631` を読むと、
  Ruby の `Task.new` が優先度を省いたときも同じ値になる。つまり
  **スクリプトと子タスクの優先度の差は 0** で、カメラ層の `follow` の子タスクは親と同じ速さで回る。
  計画書が「スクリプトとハンドラ・子タスクの優先度の既定と差」を挙げていたので探したが、
  rubevy にハンドラは無い（ハンドラの優先度差 −20 / +20 はゲーム側の prelude の数で、
  `rubevy_games/docs/numbers.md` §3.3 と §5 にある）。
* 時計の **4 ms** は `ext_task.rs:39` の `TICK_UNIT_MS`（`MRB_TICK_UNIT`）。
  **rubevy はこれを書き写していない** — `tick_scripts` は毎フレーム `Vm::task_tick_unit_ms()` を呼ぶ。
  だから「rubevy が持つ数」ではなく「VM の数」として (d) に分けた。タイムスライス 3 tick も同じ。
* `RubevyPlugin::asset_root` の `"assets"` は bevy の `AssetPlugin` の既定。
* `AS_ARRAY` の 5 型は bevy_reflect 0.19 の事実で、rustdoc にファイルまで書いてあった。

**「測った」が 0 件**になった。これは書いていて手が止まったところで、二度数え直した。
rubevy には測った記録がたくさんある（R1〜R5、`docs/verification/scale.md`）のに、
そのどれも**既定値を決めた記録ではない**。測ってあるのは既定値が買えるもの — 200,000 命令で
読みが 2,600 回・購読が 3,900 件・拒否が 6,000 件、64 件のキューが 16〜335 B × 件数、
`frame_time` 8 ms で 3000 台の tick が 8.28 ms — の方である。
`docs/numbers.md` §8 にそう書いた。この区別は、既定値を動かす提案を書くときの出発点でもある。

## 3. 分類して、1 つだけ設定できるようにした

(a) 8 件 / (b) 14 件 / **(c) 1 件** / (d) 3 件 / (e) 18 件。

**(c) が 1 件しかない**のは、R3（`queue_limit`）と R5（計測器の数を全部引数に）が先に済ませていたからで、
残っていたのは `reflect::MAX_DEPTH = 16` だけだった。

### なぜ `pub` フィールドではなくメソッドの対にしたか

計画書 3.10-3 は「VM ごとのものは `ScriptWorld<M>` の `pub` フィールド」と言っている。
`max_depth` はそれができない。**読む場所がネイティブの中**だからである:

* 読み（component → Ruby）は `answer_reflect_requests` の中で、`&mut ScriptWorld` がある。ここは問題ない。
* **書き**（`e[:X] = hash`）は `set_component` / `set_resource` のネイティブが Hash を VM から
  読み出すところで起きる（`src/lib.rs`、`reflect::read_ruby`）。ネイティブに渡るのは `&mut Vm` だけで、
  `ScriptWorld` のフィールドは見えない。

これは R3 が `queue_limit` で踏んだ問題の**裏返し**である。R3 は「上限を購読したときではなく publish する
ときに解決する」ことで 1 か所に保った（publish はホスト側だから）。`max_depth` は逆向きなので、
数を **VM の host state に置き**、`ScriptWorld::max_depth()` / `set_max_depth()` がそこを読み書きする形にした。
数は 1 つ、置き場所は VM。写しは無い。

`HostState` の `#[derive(Default)]` はこれで使えなくなった（既定が 0 になってしまう）ので、
`impl Default` を手で書いた。理由は型の rustdoc に 1 行残した。

### 深さの数え方を変えた（振る舞いは変えない）

`reflect.rs` の 3 本の歩き（`read_at`、`to_ruby`、`apply_at`）は `depth` を**足し上げて** `MAX_DEPTH` と
比べていた。限度を引数にするなら、再帰呼び出し 20 か所すべてに `max` を持ち回ることになる。
代わりに**残り段数を数え下げる**形にした:

```rust
fn levels(max_depth: usize) -> usize { max_depth.saturating_add(1) }
// 入口: read_at(vm, v, tag, levels(max_depth))
let Some(deeper) = left.checked_sub(1) else { return ...; };   // 以前の `if depth > MAX_DEPTH`
```

これで変えた行は各関数 2 行（入口と番兵）だけで、再帰呼び出しは `deeper` を渡す形のまま。
`levels` が `+1` するのは**前の振る舞いをそのまま保つ**ためで、`depth > 16` は「深さ 0 から 16 までの
17 段を通す」という意味だったから、数え下げの初期値は 17 でなければならない。
`saturating_add` にしたのは、app が `usize::MAX` を頼んだときに 0 段に化けないようにするため。

### 設定が効くことの確認

`tests/max_depth.rs`（4 件）。4 段入れ子の component を 1 つ作り、

* 既定（16）では 4 段下の数が読めるし、書ける。**ここが「動きを変えていない証拠」の 1 つ**。
* `set_max_depth(2)` にすると、同じスクリプト・同じ component で 3 段目が nil になり、書きは拒まれる。

書きのテストを書いていて 1 つ見つけた。**深さを超えた書きの断り文句が、深さの話をしない。**
`apply_at` には `too deep` という文があるのに、そこへは（この道からは）決して届かない —
Hash は先に `read_ruby` が VM から読み出すところで切られ、境界の 1 つ先のエントリは
**鍵も値も nil** になって届くので、`apply_at` が言うのは
`b.c: a field name must be a Symbol or a String` である。場所（`b.c`）は正しく、文が nil のことを言っている。
R10 より前からの性質（`MAX_DEPTH` が 16 のときは 17 段目で同じことが起きる）なので直していない。
テストは「`b.c: ` で始まる」だけを見て、なぜそうなるかをコメントに残した。`docs/numbers.md` §9-3 にも挙げた。

## 4. rustdoc に足したもの

計画書 3.10-4 の「同じ 1 行を各フィールドの rustdoc にも」。足したのは 10 か所で、
どれも「この数がどこから来たか」の 1 段落だけ。既にある本文は 1 行も削っていない。

`budget` / `frame_time` / `overrun` / `max_depth` には「**unknown**」と書いた。
`Script::priority` / `asset_root` / `AS_ARRAY` / `rubevy-build` の 3 つの名前には引用元を書いた。
`ENTITY_IVAR` / `DROPPED_IVAR` / `RESERVED_KINDS` / `$rubevy` / `ENTITY_TAG` には
「なぜこれは設定ではないのか」— 同じ綴りが 2 か所にあって両方が一致していなければならない — を書いた。
`tick_scripts` の中の `task_tick_unit_ms()` の呼び出しにも、**4 ms は VM の数であって rubevy の数ではない**、
と 1 行入れた（計画書が「rubevy が持つ数か VM の数かを区別して書く」と言っている行）。

`docs/host-api.md` の「Time」の表には**出どころの列**を足し、3 つとも unknown と書いた。
同じ節に 4 ms の粒度の段落（`sleep 0` は「次のフレーム」ではなく「時計が次に進むまで」）も入れた —
これは R9 の担当が見つけた不足で、計画書 7 章にある。

## 5. 確認

```
cargo test --workspace          175 passed / 4 ignored   （着手前 171 / 4。足したのは max_depth の 4 件）
cargo clippy --workspace --all-targets   新しい警告 0（既存の 2 件のみ:
                                         src/lib.rs の collapsible_if、examples/headless.rs の type_complexity）
cargo build --target wasm32-unknown-unknown --lib        通る
cargo test --test no_wasm_unsupported                    2 passed
cargo doc --no-deps                                      警告 0
```

**既定値のままで全テストが通る**ことが「動きを変えていない証拠」の本体である。
計測（`how_many_scripts` が R5 の表とぶれの範囲で同じか）は**取っていない**: この機械は
rubevy_games 側の別の担当（S6）が長く使っていて、着手時の load average が 17.8、
Playwright の Chromium が 8 プロセス走っていた。静かなときに回す再現コマンドは報告に書いた。
`max_depth` の変更が読み書きの道に足したのは `HostState` からの `usize` の読み 1 回
（答えの周回につき 1 回、ネイティブにつき 1 回）で、VM の命令数は増えていないが、
**測っていないので「ぶれの中」とは書かない**。

## 6. 気づいた点

* **計測器の `Limits::Default` が既定値を写している。** `examples/how_many_scripts.rs:282-283` が
  `scripts.budget = 200_000; scripts.frame_time = Some(Duration::from_millis(8));` と直書きしていて、
  `ScriptWorld` の既定を読んでいない。既定値が動いた日に `default(200k/8ms)` と名乗る行が
  既定ではない設定を測ることになる。行の名札（`:272`）が 3 度目の写し。
  **属する先: rubevy（計測器）**
* **計測器の冒頭の「Every one of them is an argument」が正確ではない。**
  引数なのは `FRAMES` / `REPEATS` と `mem` モードの 2 つだけで、`SCRIPTS` と `SLEEPS` は
  一覧の中から**行を選べるだけ**、`SUBSCRIBERS` / `PER_FRAME` / `QUEUE_LIMITS` / `SETTLE` / `FRAME` は
  `const` のまま。**属する先: rubevy（文書と実物のずれ、小）**
* **深さを超えた書きの断り文句が深さの話をしない**（§3）。`apply_at` の `too deep` は
  component の書きの道からは到達しない。**属する先: rubevy（表示、小）**
* **`docs/host-api.md` の「Time」の表が `ScriptWorld` の `pub` な数の全部ではない。**
  `RunLimits` に渡る 3 つを並べているが、いま `ScriptWorld` には `queue_limit` と `max_depth` もある。
  それぞれ別の節（「Events」「Components by name」）に書いてあるので重複させるかは判断が要る。
  `docs/numbers.md` は 1 枚にまとめてあるので、当面はそちらを指せばよいと考える。
  **属する先: rubevy（文書）**
* **`4c1e89f` が指す `docs/after-gems-plan.md` は rubevy の repo に無い**（sabiruby 側にある）。
  読んだが数は書かれていなかったので出どころは不明のままだが、rubevy の
  コミットメッセージが rubevy に無いファイルを指しているのは、あとから読む人が探すことになる形である。
  **属する先: repo の作法（小）**
