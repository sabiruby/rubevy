# 購読を名前で引く（R1）

2026-09-20。計画書 `docs/plans/generalize-plan.md` の段階 R1、ブランチ `generalize`（`52bfe7b` の上）。
調査は `docs/worklog/2026-09-20-factory-survey.md` の「実測 1: イベント」、最初に作ったときの判断は
`docs/worklog/2026-09-15-events.md`。

## 前提の確認

計画書 3.1 は `src/lib.rs:1099-1107` を指しているが、これは調査時点（main `fb4f398`）の行番号で、
R6 と R8 が載った着手時点では `publish_value` は `:1157-1180` に動いていた。中身は計画書の記述どおりで、

```rust
let queues: Vec<ObjId> = match self.vm.host_state::<HostState>() {
    Some(state) => state
        .subscriptions
        .iter()
        .filter(|s| s.name == name && (entity.is_none() || s.entity == entity))
        .map(|s| s.queue)
        .collect(),
    None => return,
};
```

`HostState.subscriptions` は `Vec<Subscription>`、`Subscription` は `{ entity: Option<Entity>, name: String,
queue: ObjId }`。`publish` は 1 件ごとにこの `Vec` を端から端まで歩き、名前を文字列で比べる。誰も聞いていない
名前でも全部歩く。調査の数字（購読 1000 件で 1 回 219 ns、購読 1 件あたり約 0.22 ns）はこの `filter` の値段だった。

購読を作る側は `Rubevy.subscribe` のネイティブ 1 か所だけ（`src/lib.rs:2540` 付近）で、そこは
`current_entity(vm)` がエンティティを返さなければ `ArgumentError` にしているので、**`Subscription.entity` は
実際には常に `Some`** である。`Option` は今のところ使われていない（下の「気づいた点」）。
解除は `ScriptWorld::unsubscribe(entity)` の 1 か所で、呼ぶのは 2 つ — `stop_removed_task`（`ScriptTask` が
外れたとき = despawn と差し替え、`src/lib.rs:258`）と `tick_scripts` がタスクの終了を見て `ScriptDone` を
入れるところ（`src/lib.rs:1833` 付近）。`docs/worklog/2026-09-15-events.md` が書いているとおり、
どちらか片方では足りない。

## 作ったもの

`HostState.subscriptions` を `HashMap<String, Vec<Subscription>>` にした。鍵が名前、値がその名前を聞いている
購読の列で、列の並びは購読した順。`Subscription` から `name` が消え（鍵になったので）、代わりに `seq: u64`
（この VM で何番目の `Rubevy.subscribe` だったか）が入った。数え元は `HostState.subscriptions_made`。

* `publish_value` は `get(name)` 1 回。当たらなければそこで返る。当たったら、その名前の購読だけを
  エンティティで絞る。**エンティティ宛と全員宛の区別も、届く順も、絞り込みの中身もそのまま。**
  1 回の `publish` は 1 つの名前しか触らないので、「購読した順に届く」は名前ごとの並びだけで決まり、
  それは `push` が末尾に足す限り保たれる。
* `subscriptions()` は `values().map(Vec::len).sum()`。別に数を持たない（持つと、足す場所と外す場所の両方で
  合わせ続ける second source of truth になる）。歩くのは「名前の数」で、購読者の数ではない。
* `unsubscribe(entity)` は全部の名前を `retain` で歩き、そのエンティティの購読を落として、
  **空になった名前は map から外す**。外さないと、終わったスクリプトの名前が溜まって、`publish` の外れも
  `subscriptions()` も少しずつ高くなる。

`Vec` から `HashMap` にすると、解除のときに名前が出てくる順が**プロセスごとに変わる**（`RandomState`）。
キューを閉じるのは、そこで待っているタスクを起こすことなので（`src/lib.rs` の `unsubscribe` の rustdoc）、
順が変わればタスクが起きる順も変わる。1 本のスクリプトが 6 本購読する形（`tests/restart_burst.rs`）は実際にある。
そこで落とすキューを `seq` で並べ直してから閉じている。`Vec` 1 本だったときの順（購読した順）がそのまま戻る。
並べ直す対象は「1 エンティティが持っていた購読」だけなので数本から十数本で、解除はフレームに何度も起きる道ではない。

依存は増やしていない。`std::collections::HashMap` はこのファイルが既に 3 か所（`in_tick_answerers`、
`reflect_cache`、`resource_cache`）で使っているものと同じで、`wasm32-unknown-unknown` でも動く。
新しい数は入れていない（`QUEUE_LIMIT` は R3 の仕事）。公開 API は 1 つも変わっていない
（`Subscription` も `HostState` も非公開）。

## 捨てた案

* **`BTreeMap<String, Vec<Subscription>>`。** 走査の順が決まるので `seq` が要らなくなる。採らなかったのは、
  引きが O(log n) 回の文字列比較になることと、決まる順が「名前のアルファベット順」であって
  「購読した順」ではないこと。`seq` 1 つで元の順そのものが戻るなら、そちらのほうが「変えていない」と言える。
* **エンティティ → 名前の索引を別に持つ。** 解除が「全部の名前を歩く」でなくなる。採らなかったのは、
  名前の文字列をもう 1 部持つことになるのと、解除の総仕事量は**今と同じ**（今も全購読を歩いている）で、
  R1 が直すと言っているのは publish だからである。解除が重くなったわけではないので、この段階では足さない。
* **`subscriptions()` 用にカウンタのフィールドを持つ。** O(1) になるが、合わせ続ける数が 1 つ増える。
  `subscriptions()` は HUD とテストのための口（rustdoc）なので、名前の数ぶん足すほうを採った。
* **`Subscription.entity` を `Entity` にする（`Option` をやめる）。** 実際に `None` は作られないので型で言えるが、
  R1 の仕事ではないので触っていない。報告に書く。

## 測ったこと

計測器は worktree `rubevy-wt-factory-survey` の `examples/factory_events.rs`（未コミット、調査の担当が書いたもの）を
両方の worktree の `examples/` にコピーして使った。**R1 のコミットには入れていない**（常設は R5）。
条件は調査と同じ — 13th Gen Intel Core i7-13700、WSL2、`--release`、`taskset -c 2`、sabiruby 0.5.2 / bevy 0.19.1、
60 フレーム × 3 回。前は `git worktree add --detach ../rubevy-wt-r1-before 52bfe7b` に同じ example を置いて建てた。

**罠 1（ビルド）。** 2 つの worktree を同じ `CARGO_TARGET_DIR` で建てると、cargo は 2 回目を「変更なし」と見て
0.11 秒で終わり、`target/release/examples/factory_events` は 1 回目のバイナリのままだった（md5 が一致して気づいた）。
uplift されるファイル名が同じで、cargo が別パッケージと見ていないため。`touch src/lib.rs` してから建て直し、
建てるたびにスクラッチパッドへコピーして md5 が違うことを確かめてから測った。前後を同じ target dir で測る人は
ここを踏む。

**罠 2（静けさ）。** 1 回目の「後」の測定の途中で、隣の担当（rubevy_games S2）の `docker run … cargo build` が
走っていた。中央値は大きく変わらないが p95 が跳ねていた（100×1000 で 28.5 ms 対 12.2 ms、1000×1000 の
`publish_heard` が 119.6 ms 対 94.4 ms）。`pgrep` が空になるのを待って取り直した。以下は取り直した数字で、
「前」は 2 回取ってある（2 回目の終わり際に隣の wasm ビルドが始まったが、1000×1000 の `publish_unheard` が
227.98 対 227.51 ns と一致しているので、見ている列には効いていない）。

### 誰も聞いていない名前への publish（1 件あたり ns。`publish_unheard` ÷ P）

これが R1 の核。行が購読の総数、列が 1 フレームの publish 件数。

| 購読数 | P=1 前 | P=1 後 | P=10 前 | P=10 後 | P=100 前 | P=100 後 | P=1000 前 | P=1000 後 |
|---|---|---|---|---|---|---|---|---|
| 1 | 67 / 121 | 237 | 21.8 / 23.3 | 26.7 | 12.4 / 15.2 | 17.1 | 7.9 / 6.1 | 13.8 |
| 10 | 122 / 169 | 195 | 31.4 / 36.4 | 36.5 | 10.5 / 11.9 | 10.1 | 8.1 / 8.5 | 9.2 |
| 100 | 146 / 179 | 124 | 47.8 / 46.1 | 20.5 | 35.9 / 35.7 | 9.8 | 31.3 / 31.4 | 8.9 |
| 1000 | 767 / 1299 | 222 | 234 / 255 | 24.5 | 221 / 223 | 10.9 | 228.0 / 227.5 | 9.6 |

（前は 2 回ぶんを `/` で並べた。P=1 の列は `Instant::now()` 2 回ぶんを 1 件で割っているので、どの行も
100〜1300 ns の計器の値段が乗っている。読むのは P=100 と P=1000 の列。）

**P=1000 の列**: 前は購読 1 件で 7.9 ns、1000 件で 228 ns — 購読 1 件あたり 0.22 ns で増えていく
（調査の読み取り 3 と同じ）。後は 13.8 / 9.2 / 8.9 / 9.6 ns で、**購読の総数に依らない**。
1000 購読で 24 倍安くなった。P=100 の列も同じ（31.3 → 8.9、221 → 10.9）。

購読が 1 件のときだけ、後のほうがわずかに高い（7.9 → 13.8 ns）。名前のハッシュ（`RandomState`、SipHash-1-3）は
購読が 0 件でも払うが、前の全走査は購読 1 件なら文字列比較 1 回で済んでいた。釣り合うのは購読 10〜30 件あたり。
ゲームがイベントを使うのはその上なので、この向きで正しい。

### 聞く人がいる publish とフレーム時間（悪くなっていないこと）

| 購読数 × P | フレーム中央値 前 / 後 | `publish_heard` 前 / 後 |
|---|---|---|
| 1 × 1000 | 326.8 / 250.4 µs → 384.2 µs | 137.6 / 113.6 → 162.9 µs |
| 10 × 1000 | 1.475 / 1.458 ms → 1.458 ms | 941 / 921 → 938 µs |
| 100 × 100 | 3.496 / 3.481 ms → 3.447 ms | 799 / 774 → 773 µs |
| 100 × 1000 | 11.880 / 11.882 ms → 11.706 ms | 9.174 / 9.194 → 8.983 ms |
| 1000 × 10 | 3.824 / 3.834 ms → 3.829 ms | 947 / 972 → 902 µs |
| 1000 × 100 | 12.434 / 12.326 ms → 12.651 ms | 9.532 / 9.335 → 9.560 ms |
| 1000 × 1000 | 97.892 / 98.913 ms → 97.273 ms | 94.53 / 95.91 → 94.45 ms |

購読 10 件以上のセルは前の 2 回のぶれ（±1〜2%）の中で、悪くなっていない。
**1 × 1000 のセルだけは判断を保留する**: `publish_heard` が前 113.6〜137.6 µs に対して後 162.9 µs で、
前の 2 回のぶれより外に出ている。ただし汚れていたほうの「後」の測定では同じセルが 110.2 µs で、前の 2 回より
**速い**。このセルは走るたびに ±40% 動いており、3 回の測定からは「上がった」とも「ぶれ」とも言えない。
購読 1 件のときにハッシュぶん（10〜20 ns/件）高くなること自体は上の unheard の表で見えているので、
同じものが出ているのだとすれば 1000 件で 10〜20 µs。**購読 1 件で毎フレーム 1000 件 publish する形が
気になる利用者がいたら測り直す**、という状態にしてある。読み取り件数（`read_*` の列）は全セルで前と同一。

## 確かめたこと

`cargo test --workspace`: 110 passed / 3 ignored（着手前 109 / 3。増えた 1 本は下）。
`cargo clippy --workspace --all-targets`: 警告は既存の 2 件のみ（`src/lib.rs` の `collapsible_if`、
`examples/headless.rs` の `type_complexity`）。新しい警告 0。
`cargo build --target wasm32-unknown-unknown --lib` 通る。`cargo test --test no_wasm_unsupported` 2 passed。

テストを 1 本足した（`tests/events.rs::within_a_name_the_message_goes_out_in_the_order_it_was_subscribed_to`）。
索引にして初めて「名前ごとの列」という構造ができたので、その構造が Ruby から見てどう見えるかを留める
——1 本のスクリプトが 3 本（同じ名前 2 本＋別の名前 1 本）購読し、ホストが `publish_value` に
数え上げるクロージャを渡して 1 回 publish する。クロージャは購読者ごとに 1 回走るので、先に購読したほうが 1、
後が 2 を受け取れば「名前の中では購読した順」が言える。別の名前は 0 件のまま。`subscriptions()` が 3 を返すことも
同じテストで見ている（1 エンティティが複数の名前を持つ形は、既存のテストでは `tests/restart_burst.rs` の
6 本だけだった）。既存の `tests/events.rs` 10 本と `tests/restart_burst.rs` は 1 行も変えずに通る。

## 気づいた点

* **`Subscription.entity` が `Option<Entity>` なのに `None` は作られない。** 作る場所は
  `Rubevy.subscribe` のネイティブ 1 か所だけで（`src/lib.rs:2540` 付近）、そこは
  エンティティが無ければ `ArgumentError` にしている。`publish` の
  `entity.is_none() || s.entity == entity` の左側（全員宛）は publish の引数の `Option` で、こちらは要る。
  `Entity` にすれば「購読は必ずエンティティに属する」が型で言える。rubevy（型が緩い）。R1 の範囲外なので触っていない。
* **`ScriptWorld::publish` の rustdoc は「誰も購読していない名前には何も積まれないので、ゲームは自由に
  publish してよい」と書いていた**（`src/lib.rs:1124-1126`）。R1 の前は、その「自由に」が購読の総数ぶん
  かかっていた。文書のほうが先に正しかった例。本（mruby の移植の章）の素材。
* **計測器 `examples/factory_events.rs` の P=1 の列は、計器の値段（`Instant::now()` 2 回）がそのまま乗る。**
  R5 で常設するときは、publish の回数を測る側に合わせて「1 フレームぶんをまとめて測って割る」か、
  P=1 の行を出さないかを決めたほうがよい。rubevy（R5 の材料）。
* **同じ `CARGO_TARGET_DIR` で 2 つの worktree の example を建てると、2 つ目が建たずに 1 つ目のバイナリが残る**
  （上の「罠 1」）。前後を測る段階（R2〜R5）は全部これを踏む。`docs/verification/` を作るとき、
  再現手順に「md5 で別物だと確かめる」を入れておくとよい。repo の作法／R5 の材料。
