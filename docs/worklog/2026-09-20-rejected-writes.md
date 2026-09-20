# 2026-09-20 拒まれた書きをスクリプトから読める口（R9 の前）

計画 `docs/plans/generalize-plan.md` の段階 R9 の前に、著者が決めた「**拒まれた書きをスクリプトから
読める口**」（2026-09-20、著者判断 2）を足す。出どころは R0 の気づき 2（`docs/worklog/2026-09-20-camera-from-ruby.md`）
と R8 の気づき（同 `-resources-by-name.md`）— component と resource の 2 か所で、書きが拒まれても
ホストの `warn!` だけで Ruby には何も返らない。

## なぜ「その場で返す」ができないのか（設計の出発点）

`Rubevy.set_component` / `Rubevy.set_resource` は**問い（ask）ではなく命令（command）**で、
`push_command` でキューに積むだけ（`src/lib.rs`、`set_component` のネイティブ）。実際に書くのは
フレーム末尾の `apply_component_writes` / `apply_resource_writes` で、そこには `&mut World` はあるが
`Vm` は無い（借用の都合。これは「読みは tick の中、書きはフレーム末尾」という rubevy の約束そのもの）。
だから `e[:X] = hash` の戻り値は渡した Hash のままにするしかなく（`src/prelude.rb` の `Entity#set`）、
**拒まれたことは次のフレームまで誰も知らない**。著者の言う「直前のフレームに拒まれた書き」はこの構造の
必然で、口の形はそこから決まる。

## 形を 2 つ比べた（`$rubevy` か、ask か）

計画は「`Rubevy.find` などの prelude の流儀」と「`$rubevy`（毎フレーム更新されるグローバル）」の
どちらが小さいかを比べて決めろと言っている。比べた。

- **`$rubevy` に載せる道**: `set_frame_state`（`src/lib.rs`）は毎フレーム Hash を 1 つ作って 3 つの
  スカラを入れている。ここに配列を足すと、**拒否が 1 件も無いフレームでも**配列 1 つと（件があれば）
  件数ぶんの Hash と Entity オブジェクトを VM の中に作ることになる。読まないゲームが払う。
  そのうえ `$rubevy` は全スクリプトが見る 1 つの値なので、「自分の書きが拒まれたか」を出すには
  各スクリプトが `by` を見て `select` することになり、**誰のものか**という情報を必ず持ち回る必要がある。
- **ask の道**（採用）: 予約 kind を 1 つ（`writes.rejected`）足して `RESERVED_KINDS` に入れ、
  `src/prelude.rb` に `Rubevy.rejected_writes` を 1 メソッド。**聞かれたときだけ**値を組み立てる
  （拒否の無いフレームの費用は、ホスト側の空の `Vec` が 1 つ残ること以外に無い）。そして
  `Request::entity` が**聞いた側のエンティティ**なので、ホストが答える時点で「自分のぶんだけ」に
  絞れる — Ruby 側には絞り込みのコードが 1 行も要らない。

ask の方が小さいので ask にした。決め手は 2 つ目（誰のものかの絞り込みが、Ruby ではなく答える側で
ただで済む）で、これは `$rubevy` の道では原理的に得られない。

## 「自分の書き」をどう定義したか

`by` は**書いたタスクのエンティティ**（`@rubevy_entity`。`Task.new` で作った子タスクは親のものを継ぐ、
`src/prelude.rb`）で、`on`（書かれた相手）とは別に持つ。スクリプトは他のエンティティの component を
書けるので、この 2 つは違う。`Rubevy.rejected_writes` が答えるのは `by == 聞いた人` のものだけ。
Ruby の Hash には `by` を入れていない（常に自分なので）。ホスト側の `ScriptWorld::rejected_writes()` は
**全部**を返し、各件が `by` と `on` の両方を持つ — HUD やテストはこちらを読む。
`tests/rejected_writes.rs::a_write_to_another_entity_is_still_this_scripts_write` が、
他人の component への書きが「自分の書き」として返ることと、ホスト側が両端を知っていることの両方を主張する。

## 覚えておく長さ — ここで設計を 1 つ変えた

最初は著者の言葉どおり「直前のフレームぶん」を実装した（`apply_component_writes` の先頭で毎フレーム
`clear()`）。テストを書いていて破綻した。**スクリプトは「次のフレーム」に起きられない。**
`sleep 0` は現在の tick を期限にして park するが、rubevy は外部クロック（`task_external_clock(true)`）で
mruby-task の時計を進めるのは `tick_remainder` が 1 tick（`TICK_UNIT_MS` = 4 ms）貯まったフレームだけ
（`src/lib.rs`、`tick_scripts` の頭）。テストのように 1 フレーム 5 ms の環境でも起きるのは 1〜2 フレーム後で、
60 fps でも 16 ms なので 4 tick ぶんまとめて進む。**毎フレーム空にすると、書いた本人が見るより先に消える。**

そこで規則を「**書きのあったフレームだけ置き換える**」に変えた。フレームの書きが 0 件なら前のまま、
1 件でもあれば（component か resource のどちらでも）まず全部捨ててからそのフレームの拒否を入れる。
著者の「覚える量は 1 フレームぶん（次のフレームで置き換わる）」は満たしている — 入っているのは常に
**1 つのフレームの書きが拒まれたぶん**だけで、それより古いものは混ざらない。違うのは「次のフレーム」が
「次に書いたフレーム」になったことで、これは上の理由で**そうしないと口が使えない**から。
実装は `apply_component_writes`（2 つのうち先に走る方）が両方のキューを見て判断する。

上限は置かなかった。**根拠**: 書きには命令を使うので、フレームの `budget` が既に件数を縛っている。
測った（`tests/rejected_writes.rs::what_one_rejected_write_costs`、debug ビルド、1 件 = ネイティブ呼び 1 回の
最短のループ）:

```
rejected writes: [(100, 3361, 100), (1100, 36361, 524)]
one write costs 33.0 instructions; the last frame kept 524
a budget of 200,000 bounds a frame at 6061 of them
```

1 件 33 命令。既定の予算 200,000 命令なら 1 フレーム 6,000 件ほどが上限で、しかも **1,100 件を書かせた
スクリプトは予算で切られてフレームをまたぎ、最後のフレームに残ったのは 524 件**だった — 予算が
実際に縛っている証拠がそのまま出ている。1 件は 4 つの小さなフィールドと短い文字列 2 つ（型名と理由）なので、
6,000 件でも数百 kB の桁。根拠の無い上限を置くと、そこから落ちるのは「探していた 1 件」になる。

## `FrameStats` に件数を足すか

足さなかった。R5 の方針（`FrameStats` の rustdoc: 「every number in it is one the tick already had」）に
照らすと、拒否は **tick の中の出来事ではない** — 書きは tick の後の 2 つのシステムで当てられる。
tick の統計の構造体にフレームの別の部分の数を入れると、その構造体が言っていることが嘘になる。
ゲームが欲しい数は `world.rejected_writes().len()` で正確に取れる（`ScriptWorld::rejected_writes` の
rustdoc にこの理由を 1 段落書いた）。

## `warn!` は残した

著者の指示どおり。ホストのログは今までどおりで、Ruby 側の口はそれと**同じ文**を返す
（`reflect::apply_ruby` の `Err` をそのまま `reason` に入れている）。1 か所だけログと一覧がずれるのは
**相手のエンティティが消えていた場合**で、これは元から `warn!` を出していない（`continue` するだけ）。
拒んだのではなく相手が無くなっただけなので一覧にも入れない、と決めて両方に書いた。

## 確認

同じ機械（13th Gen Intel Core i7-13700、WSL2、debug ビルド）。この段階は計測の段階ではないので、
時間の数字は取っていない（機械は別の担当の garden selftest で load average 155。命令数は機械に依らない）。

- `cargo test --test rejected_writes` → 7 passed, 1 ignored。
- 全体の結果は次の段階（カメラ層）と合わせて `docs/worklog/2026-09-20-camera-layer.md` に。

## 試して捨てたもの

- **毎フレーム `clear()`**（上記）。テスト `a_script_reads_what_became_of_its_own_write` が
  `sleep 0` の後に 0 件を読んで落ちたことで分かった。原因は VM の時計が 4 ms 単位であることで、
  rubevy 側では直せない（直すなら「1 フレームだけ待つ」口を VM か rubevy に足すことになり、
  それは別の話）。
- **Ruby 側で `select` する形**（ホストは全件返し、prelude が `w[:by] == Rubevy.entity` で絞る）。
  `by` を Hash に入れる必要が出るうえ、全スクリプトが全員ぶんを受け取ってから捨てることになる。
  絞り込みは答える側でただでできる。
- **`Rubevy.rejected_writes(all: true)` のようなキーワード引数**。「使う人が出るまで口を増やさない」
  （計画 6 章、R0 の判断と同じ原則）に従って足さなかった。全件が要るのは今のところホスト側だけで、
  そこには `ScriptWorld::rejected_writes()` がある。

## 気づいた点（仕事の範囲の外。直していない）

1. **スクリプトから「次の 1 フレームだけ待つ」と言えない。** 何を: `sleep 0` は「VM の時計が次に
   進むまで」であって「次のフレームまで」ではない。どこで: `src/lib.rs` の `tick_scripts`（`tick_remainder`
   が 1 tick 貯まったときだけ `task_advance_ticks`）と sabiruby `ext_task.rs` の `sleep_us`
   （`TICK_UNIT_MS` = 4 ms）。なぜ気になるか: 「書いて、次のフレームに結果を見る」は component の書きの
   意味論そのものなので、これが言えないと拒否の一覧の寿命のような設計判断がすべて引きずられる（今回
   引きずられた）。どこに属するか: rubevy の口の設計（`Rubevy.next_frame` のようなものを足すか、
   `$rubevy[:frame]` を見て回すかの判断）。**足すなら別の段階で。**
2. **`Rubevy.rejected_writes` は「書いた瞬間」と結び付いていない。** 何を: 一覧は型名と理由を持つが、
   スクリプトのどの行が書いたかは持たない。どこで: `RejectedWrite`（`src/lib.rs`）。なぜ気になるか:
   同じ型に 2 か所から書くスクリプトでは、どちらが拒まれたか分からない。どこに属するか: rubevy の口の設計。
   足すとしたら書いた時点の `Vm::task_location` を `ComponentWrite` に持たせる（1 件あたりの費用が増えるので、
   使う人が出てからで良い）。
3. **`docs/host-api.md` の「Time」の表に `frame_time` の既定 8 ms とあるが、VM の時計の粒度（4 ms）は
   どこにも書かれていない。** 何を: スクリプトから見える時間の最小単位。どこで: `docs/host-api.md` の
   「Time」。なぜ気になるか: `sleep 0.001` を書いた人が 4 ms 待たされる理由が文書から追えない。
   どこに属するか: 文書と実物のずれ。R10（数の棚卸し）の材料。
