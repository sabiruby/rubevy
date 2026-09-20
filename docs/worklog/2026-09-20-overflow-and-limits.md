# あふれを見えるようにし、上限を選べるようにした（R3）

2026-09-20。計画書 `docs/plans/generalize-plan.md` の段階 R3、ブランチ `generalize`（`d989ea0` の上）。
調査は `docs/worklog/2026-09-20-factory-survey.md`（「コードから分かったこと」の上 2 行と「実測 1」）、
64 を決めたときの記録は `docs/worklog/2026-09-15-events.md`、同じ場所を直した前の段階は
`docs/worklog/2026-09-20-subscription-index.md`（R1）。

## 前提の確認

計画書 3.3 は `make_room` を `src/lib.rs:1123-1139` と書いているが、これは調査時点（main `fb4f398`）の行番号で、
R6・R8・R1・R2 が載った着手時点では `:1253-1269` に動いていた。中身は計画書の記述どおり。

```rust
fn make_room(&mut self, queue: ObjId) {
    loop {
        let n = match self.vm.task_queue_len(queue) { Ok(n) => n, Err(_) => return };
        if n < QUEUE_LIMIT { return; }
        match self.vm.task_queue_try_pop(queue) { Ok(Some(_)) => {}, _ => return }
    }
}
```

落としたものを数えていない。ログも出さない。`QUEUE_LIMIT` は `const 64` で、`ScriptWorld::<()>::QUEUE_LIMIT`
として公開されているが**読めるだけ**だった。調査の表（実測 1）の「購読 1 本が 1 フレームに受け取れるのは 64 件
（3840 = 64 × 60）」は、この 64 がそのまま天井として見えていたものである。

`Subscription` は R1 で `{ entity: Option<Entity>, queue: ObjId, seq: u64 }` になっていて、
`entity` が `Option` なのに `None` を作る場所が無いことは R1 の報告が書いていた（計画書 7 章、「R3 が同じ場所を
触るときついでに直してよい」）。

**VM の口が足りるかを先に確かめた。** 計画書の案「`make_room` がキューオブジェクトの ivar を増やし、
`src/prelude.rb` が読む」は、`Vm::ivar_set(&mut self, obj: ObjId, name: &str, v: Value)` /
`Vm::ivar_get(&self, obj: ObjId, name: &str) -> Value`（sabiruby 0.5.2 `src/vm.rs:985-1001`）で書ける。
ivar の表はオブジェクト種別を問わない（`src/object.rs:870-879` は `ObjKind` を見ずに `ivars` を引く）ので、
`Task::Queue` のオブジェクトにも置ける。`ENTITY_IVAR`（`@rubevy_entity`）が既にタスクに対して同じことを
しているので、形も前例どおり。**別の置き方を探す必要は無かった。**

`Rubevy.subscribe(name, limit: n)` のキーワードがネイティブに届くかも先に確かめた。sabiruby の
`native_call_args`（`src/vm.rs:4264-4271`）は、呼び出し側のキーワードを**末尾の Hash として位置引数に足して**
ネイティブに渡す（空の Hash は足さない）。だから `define_closure` のままで `a.get(1)` が Hash になる。

## 作ったもの

1. **`ScriptWorld::queue_limit`**（`pub` フィールド、既定 `QUEUE_LIMIT` = 64）。`budget` / `frame_time` /
   `overrun` と同じ並び・同じ書き方で、rustdoc に出どころを書いた（「**測って決めた数ではない**。
   2026-09-15 の定性的な理由だけ」と、測った材料の置き場所）。
2. **`Rubevy.subscribe(:belt, limit: n)`**。`n` は 1 以上の整数。それ以外（Float、0、負、`limit` 以外の
   キーワード、Hash でない第 2 引数）は `ArgumentError`。
3. **`ScriptWorld::dropped() -> u64`**（VM が落とした累計）と **`Rubevy::Subscription#dropped`**
   （その購読が落とした累計）。前者は `HostState.dropped`、後者はキューオブジェクトの
   `@rubevy_dropped`（`DROPPED_IVAR`）。どちらも `make_room` が同じ 1 回で書く。
4. `Subscription.entity` を `Option<Entity>` から `Entity` に（非公開の型、公開 API は不変）。

### 上限を「購読したとき」ではなく「publish するとき」に解決した

計画書は 2 段（VM の既定 → 購読ごと）と書いている。素直に読むと、ネイティブが購読を作るときに VM の既定を
読んで `Subscription.limit: usize` に写す形になる。**採らなかった。** ネイティブが持っているのは `&mut Vm` だけで、
`ScriptWorld` のフィールドは見えない。見せるには既定値を `HostState` にも置いて、フィールドと同期し続ける
必要がある——数が 2 か所になる。

代わりに `Subscription.limit: Option<usize>` にして、`publish_value` が `s.limit.unwrap_or(self.queue_limit)`
で解決する。数は 1 か所に残り、ネイティブは `limit:` を読むだけで既定を知らなくてよい。
副作用として「app が途中で `queue_limit` を変えると、自分の上限を頼まなかった購読は全部動く」という意味になる。
これは `budget` の動き方（毎フレーム読まれる）と同じ向きなので、揃っているほうを選んだ。

### 捨てた案

* **`queue_limit` をメソッド 2 本（`queue_limit()` / `set_queue_limit()`）にする。** 同期の問題は同じで、
  `budget` との見た目が揃わない。計画書と共通 CLAUDE.md が言う「`budget` と同じ場所・同じ書き方」に従った。
* **`ScriptWorld::<()>::QUEUE_LIMIT` を `#[deprecated]` にする。** 公開 API を壊さない側で扱うのが既定
  （計画書 2 章）。非推奨は壊しはしないが、既存の利用者（テストと計測器）に警告を出す。rustdoc に
  「これは**既定値**であって上限ではない。実際の上限はフィールド」と書いて残すだけにした。
  非推奨にするかは著者判断として報告に出す。
* **番兵（あふれを知らせるメッセージをキューに流す）。** 計画書が既に却下している（既存のスクリプトが
  `pop` の戻り値の型を前提にしている）。`dropped` は別の口なので `pop` の型は変わらない。
* **落とした件数をホスト側の表（購読 → 件数）で持つ。** 購読が終わったとき表から外す仕事が増える。
  ivar はキューに載っているので、購読が消えれば一緒に消える。
* **`limit:` 以外のキーワードを黙って無視する。** Ruby 自身は `unknown keyword` を上げる。
  `limt: 4` と書いた購読が既定のまま静かに動くのは、フレームがいくつも経ってから気づく類の間違い。

### 0 の扱い

`ScriptWorld::queue_limit = 0` は拒んでいない（フィールドなので拒む場所が無い）。意味は「最新の 1 件だけ」に
なる——`make_room` は push の**前に**呼ばれるので、キューを空にしてから 1 件積む。
Ruby 側の `limit: 0` は拒む: スクリプトが `limit: 0` と書くのは数え違いであって、その意味を頼んでいるとは
考えにくい。rustdoc と `docs/host-api.md` の両方にこの違いを書いた。

## 測ったこと

条件は調査・R1・R2 と同じ — 13th Gen Intel Core i7-13700、WSL2、`--release`、`taskset -c 2`、
sabiruby 0.5.2 / bevy 0.19.1。計測器は 2 本で、**どちらもコミットしない**（常設は R5）:

* `examples/factory_events.rs` — 調査の担当が書いたもの（worktree `rubevy-wt-factory-survey` の未コミット）を
  **1 バイトも変えずに**コピーした。前後を比べる表はこれで取る。
* `examples/r3_limits.rs` — R3 で新しく書いた。`read`（1 本の購読者が 1 フレームに読み切れる件数を、上限を
  上げながら見る）と `mem`（溜まったメッセージ 1 件あたりのメモリを `/proc/self/status` の `VmRSS` の差で見る）。

どちらもスクラッチパッドの `r3/` に残した（R5 の担当向け）。

**罠 1（ビルド）は踏まないようにした。** 「前」は `git worktree add --detach ../rubevy-wt-r3-before d989ea0` で建て、
`CARGO_TARGET_DIR` を別（`target-r3-before`）にして、`md5sum` で 2 つのバイナリが別物だと確かめてから測った
（`4174a3a4…` と `31f6d98f…`）。

**罠 2（静けさ）。** 隣の担当（rubevy_games の S3）の cargo と docker がほぼずっと動いていたので、
`pgrep -af "cargo|rustc|docker run|chrome"` が空になるのを待ってから測った。

### 1 本の購読者が 1 フレームに読み切れる件数

1 購読者（`q.pop` して数えるだけ）、ホストは毎フレーム 8192 件 publish、60 フレーム。
予算は既定のまま（200,000 命令 / `frame_time` 8 ms）。上限だけを動かす。

| 上限 | 1 フレームに読めた件数 | フレーム中央値 | p95 |
|---|---|---|---|
| 1 | 1.0 | 1.06 ms | 1.11 ms |
| 16 | 16.0 | 1.09 ms | 1.24 ms |
| **64（既定）** | **64.0** | 1.14 ms | 1.38 ms |
| 128 | 128.0 | 1.16 ms | 1.25 ms |
| 256 | 256.0 | 1.26 ms | 1.41 ms |
| 512 | 512.0 | 1.53 ms | 1.96 ms |
| 1024 | 1024.0 | 2.02 ms | 2.85 ms |
| 2048 | 2047.0 | 2.63 ms | 7.03 ms |
| 4096 | **3877.4** | 8.34 ms | 10.06 ms |
| 8192 | **3960.5** | 3.68 ms | 7.64 ms |

読み取れること:

1. **頭打ちは約 3,900 件/フレーム。** 上限 2048 までは「上限 = 1 フレームに読める件数」で、
   4096 以上では上限を上げても 3,900 前後で止まる。60 fps なら 234,000 件/秒。
2. 止めているのは**命令の予算**である。200,000 ÷ 3,900 ≈ **51 命令/件** で、これは
   `v = q.pop; break if v < 0.0; n += 1` の 1 周にちょうど見合う数。フレーム中央値も 4096 の行を除けば
   8 ms を下回っている。
3. **既定の 64 は、この天井の約 1/60。** 調査の表の「購読 1 本が 1 フレームに受け取れるのは 64 件」は
   予算ではなく上限が決めていた数だった、ということがこれで言える。
4. 4096 の行だけフレーム中央値が 8.34 ms と跳ねていて、8192 の行（3.68 ms）より大きい。
   繰り返しは 1 回しか取っていないので**説明を付けない**。p95 が 8 ms を超える行が 3 つあるのは
   `frame_time` が上限になっていないためで、これは R4 の仕事。

### 溜まったメッセージ 1 件あたりのメモリ

購読して `sleep 3600` するスクリプト（1 件も読まない）を並べ、キューを上限まで埋めて、その前後の
`VmRSS` の差を件数で割った。payload は 3 種類 — `Num`、短い `Text`（`"item0004"`、8 バイト）、
4 個の Float の `List`。

| payload | 上限 1000 × 10 本 | 上限 10000 × 10 本 | 上限 64 × 1000 本 |
|---|---|---|---|
| `Num` | 20.9 B | 20.8 B | 15.7 B |
| `Text`（8 バイト） | 307.2 B | 316.0 B | 335.1 B |
| `List`（Float 4 個） | 521.4 B | 362.5 B | 332.9 B |

RSS はページ単位で、アロケータの都合も乗るので**桁として読む**: 数値は **16〜21 バイト**、
短い文字列は **300〜340 バイト**、小さな配列は **330〜520 バイト**。
数値以外が 1 件 300 バイトを超えるのは、キューの Array の枠ではなく**ヒープオブジェクト 1 個ぶん**
（オブジェクトのスロット + 中身の Vec + GC の帳簿）が乗るためである。

### 読まない購読が抱える最悪のメモリ

上の表の「上限 64 × 1000 本」の列がそのまま答えになっている（64,000 件が溜まった状態）:

| payload | RSS の増分 |
|---|---|
| `Num` | 984 kB（約 1.0 MB） |
| `Text`（8 バイト） | 20,944 kB（約 20.5 MB） |
| `List`（Float 4 個） | 20,808 kB（約 20.3 MB） |

つまり **上限 × 1 件 × 購読数**。既定の 64 でも、1000 本の購読が短い文字列を溜めれば 20 MB になる。

### 既定値をこの材料からどう導けるか（案。動かしてはいない）

* **上（読める側）からの上限**: 1 本の購読者が 1 フレームに読み切れるのは約 3,900 件。
  これより大きい上限は「いつまでも空にできないキュー」を作るだけで、あふれを遅らせても防がない。
  **L ≤ 3,900。**
* **下（メモリ側）からの上限**: 読まない購読が抱えるのは L × B × S（B = 1 件のメモリ、S = 購読数）。
  測った B は 21 B（数値）〜335 B（短い文字列）。
* **この 2 つから 64 を導けるか。** 導ける形が 1 つある: 「イベントに割く常駐メモリを 4 MB、
  購読を 200 本、payload を短い文字列（320 B）と見る」と L = 4 MB ÷ (320 B × 200) ≈ **62**。
  64 はこの形に合う。**今の 64 が測って決めた数でないことは変わらない**（2026-09-15 の記録に
  この計算は無い）が、測った材料と矛盾しない数ではある。
* **動かすなら**、payload が数値だけで購読が数十本までと分かっているアプリでは 1,000〜3,900 まで上げられる
  （3,900 で 20 本なら 21 B × 3900 × 20 = 1.6 MB）。逆に文字列や配列を publish して購読が 1,000 本あるなら、
  64 でも 20 MB なので**下げる**ほうが合う。**rubevy は payload も購読数も知らないので、既定を 1 つ選ぶ根拠は
  「一番痛くない side」しかない** — それが今の 64 で、どちらの側に寄せるかは app が
  `ScriptWorld::queue_limit` で言う、という形にした。既定を動かすかどうかは著者の判断。

### フィールドにしたことで速さが変わっていないか（`factory_events 60 3`）

`d989ea0`（R2 まで）と R3 の後を、同じ日・同じ条件で取り直した。最初に取った R3 版は**遅くなっていた**ので、
原因を 2 つ直してから取り直している（下の「速さの後始末」）。

取り直しは **前 → 後 → 前 → 後 の順に 2 巡**、毎回 `pgrep` が空であることを走る前と後で確かめて行った
（スクリプトは `r3/paired-run.sh`、出力は `r3/paired.txt`。4 回とも `QUIET`）。下の表は 2 巡の平均。

| 購読数 × P/フレーム | あふれるか | `publish_heard` 前 | 後 | 差 |
|---|---|---|---|---|
| 1 × 1 | いいえ | 1.9 µs | 1.6 µs | −17% |
| 10 × 10 | いいえ | 11.0 µs | 8.8 µs | −20% |
| 100 × 10 | いいえ | 60.2 µs | 60.5 µs | +0.4% |
| 1000 × 1 | いいえ | 711.5 µs | 683.9 µs | −3.9% |
| 10 × 1000 | **はい** | 942.5 µs | 1042.4 µs | **+10.6%** |
| 100 × 100 | **はい** | 764.1 µs | 869.5 µs | **+13.8%** |
| 100 × 1000 | **はい** | 9.18 ms | 10.43 ms | **+13.6%** |
| 1000 × 10 | **はい** | 905.4 µs | 992.1 µs | **+9.6%** |
| 1000 × 100 | **はい** | 9.67 ms | 10.74 ms | **+11.1%** |
| 1000 × 1000 | **はい** | 95.7 ms | 106.3 ms | **+11.0%** |
| 1 × 1000 | はい | 121.3 µs | 147.7 µs | +21.7%（1 巡目 +0.3%、2 巡目 +43%） |

フレーム中央値も同じ分かれ方をする: あふれるセルで +8.6〜+10.9%、あふれないセルでは −17〜+3%（ぶれの中）。
**読み取り件数（`read_min/med/max`）は 16 セルすべてで前と 1 件も違わない。**

読み取れること — **速さが変わったのは「フィールドにしたこと」ではなく「落とした件数を数えること」である。**
あふれないセル（publish した数と読んだ数が同じ行）は前と同じか速い。あふれるセルだけが 10〜14% 高い。
差を落ちた件数で割ると 1 件あたり約 12 ns で、これは `Vm::ivar_get` + `Vm::ivar_set` が名前を文字列で
引く値段に見合う（下）。**あふれていないゲームは 1 ns も払わない**（`make_room` は 1 件も落とさなければ
即座に返る）。あふれているゲームは、あふれていることを知る代わりに publish の 1 割を払う。
**この 1 割を消すかどうかは著者の判断**として報告に出す（消す道は下に書いた）。

1 購読のセル（1 × 1000）は R1 のときと同じく走るたびに大きく動くので、ここでも判断しない。

## 速さの後始末

最初の版は、落とした件数を `make_room` の中で 3 つの口に書いていた —
`ivar_get`（前の値）、`ivar_set`（新しい値）、`host_state_mut`（VM の累計）。
`make_room` は **publish 1 件 × 購読者 1 本につき 1 回**呼ばれるので、あふれている状況では
この 3 つが毎回乗る。最初の測定では `publish_heard` が
1000 購読 × 1000 件で 94.0 → 109.2 ms（+16%）、10 購読 × 1000 件で 918 → 1042 µs（+13.6%）になっていた。
差を件数で割ると 7〜15 ns/件で、`Vm::ivar_get` / `Vm::ivar_set` が名前を文字列で引く
（`syms.lookup_str` と `intern`、どちらも文字列のハッシュ）ぶんに見合う。

直したのは 2 つ:

1. **VM の累計は publish 1 件につき 1 回**にした（`make_room` が落とした数を返し、`publish_value` が
   最後にまとめて足す）。1000 本の購読者に配る 1 件で `host_state_mut` の downcast を 1000 回払っていた。
2. **配り先の一覧を細くした。** 上限を持ち回るのに `Vec<(ObjId, usize)>` にしていたが、`ObjId` は `u32` なので
   1 件 16 バイト（`Vec<ObjId>` なら 4 バイト）。`u32` に詰めて 8 バイトにした。1000 購読 × 1000 件/フレームでは
   1 フレームに 16 MB 書いていたものが 8 MB になる。上限が 4,294,967,295 を超えたら頭打ちになるが、
   その大きさのキューはどの機械にも載らない（上の表のとおり 1 件 16 バイトでも 68 GB）。

この 2 つで 1000 × 1000 のセルは +16% から +11% になったが、**消えてはいない**。残っているのは
`make_room` の中の `ivar_get` + `ivar_set` で、どちらも `@rubevy_dropped` という名前を文字列で引く
（`syms.lookup_str` と `intern`）。落ちた 1 件あたり約 12 ns という残りの差は、この 2 回のハッシュに見合う。

消す道は 2 つあり、どちらもこの段階では採らなかった:

1. **件数を `Subscription` の欄（ホスト状態）に置き、Ruby からはネイティブ経由で読む。** publish の道から
   文字列のハッシュが消える。採らなかったのは、`dropped` を `pop` のたびに呼ぶスクリプト
   （`docs/host-api.md` に書いた形）が、キューからその購読を探すのに購読の総数ぶん歩くことになるため。
   `HashMap<ObjId, u64>` を足せば O(1) になるが、購読が消えるときに外す仕事が増え、あふれの道でも
   ハッシュ 1 回は払う（整数の鍵なので文字列よりは安い）。
2. **前の件数を `publish_value` から持ち回り、`ivar_set` だけにする**（ハッシュ 1 回）。半分になるが、
   `Subscription` に欄が 1 つ増え、配り先の一覧も 8 → 12 バイトに戻る。

**どちらを採るか（あるいは 1 割を払うか）は著者の判断**として報告に出す。今の形は計画書の案そのままで、
Ruby 側は `@rubevy_dropped` を直に読める（`dropped` は O(1)）という利点がある。

## 確かめたこと

`cargo test --workspace`: **121 passed / 3 ignored**（着手前 116 / 3。増えた 5 本は下）。
`cargo clippy --workspace --all-targets`: 警告は既存の 2 件のみ（`src/lib.rs` の `collapsible_if`、
`examples/headless.rs` の `type_complexity`）。新しい警告 0。
`cargo build --target wasm32-unknown-unknown --lib` 通る。`cargo test --test no_wasm_unsupported` 2 passed。
`cargo doc --no-deps` 警告 0。`src/prelude.mrb` は `tools/compile_scripts.sh`（Docker の reference mrbc
4.1.0-rc）で作り直した（Docker は既に動いていた）。

足したテストは 5 本:

* `tests/events.rs::what_overflowed_is_counted_for_the_vm_and_for_the_subscription` — 既存の
  `the_oldest_messages_are_dropped_when_nobody_reads` の隣。100 件 publish して、VM の `dropped()` が 36、
  スクリプトから見た `q.dropped` も 36、`q.size` が 64、残った先頭が 36。
* `tests/events.rs::the_vms_queue_limit_moves_what_a_script_is_handed` — `queue_limit = 250` で 300 件
  publish → 250 件残り 50 件落ちる。フィールドの初期値が定数と同じことも見ている。
* `tests/events.rs::a_subscription_may_ask_for_a_limit_of_its_own` — 1 本のスクリプトが同じ名前を
  `limit: 3` と既定の 2 通りで購読し、10 件 publish して 3/7 と 10/0。VM の累計は 7。
* `tests/events.rs::a_limit_that_is_not_a_count_is_an_argument_error` — `limit: 0`・`limit: 2.5`・
  `limt: 4`・`Rubevy.subscribe(:tick, 4)` の 4 通りを Ruby 側で rescue して、メッセージと
  `e.backtrace.first` を報告させる。拒まれた購読が 1 本も残っていないことも見ている。
* `tests/two_vms.rs::each_vm_has_its_own_queue_limit_and_its_own_dropped_count` — 片方の VM だけ
  `queue_limit = 4` にして両方に 10 件ずつ publish。4/6 と 10/0、`dropped()` も VM ごと。

**行番号のテストで 1 回落ちた。** `e.backtrace.first` が空だった。`sabiruby_compiler::Options` の
`debug_info` は既定が `false` で、行表（DBG セクション）の無いプログラムは backtrace に言うことが何も無い。
`tests/events.rs` の `compile` は既定のままだったので、このテストだけ `debug_info: true` で組む
`run_with_lines` を足した。**「`ArgumentError` に Ruby の行番号が付く」は、`-g` で組んだプログラムに限る** —
`src/lib.rs` の rustdoc にもそう書いた。

**もう 1 回、cargo に騙されかけた。** `src/lib.rs` を書き換えて `cargo build --lib` が通った直後の
`cargo test --test events` が、新しいフィールドを「そんなフィールドは無い（あるのは `vm`, `budget`,
`frame_time`, `overrun`）」と言って落ちた。`touch src/lib.rs` してからやり直すと通った。
R1 が踏んだ「同じ `CARGO_TARGET_DIR` で 2 つの worktree」とは別の形だが、**古い成果物を新しいソースだと
思い込む**という点では同じ罠である。以後、ライブラリを直した直後のテストは `touch` を挟んでいる。

## 気づいた点

* **`Vm` に `Sym` で instance variable を読み書きする口が無い。** `Vm::ivar_get` / `ivar_set` は名前を
  `&str` で取り、毎回 `syms.lookup_str` / `intern`（どちらも文字列のハッシュ）を通る。ホストが 1 つの名前を
  毎フレーム何万回も書く道（今回の `@rubevy_dropped`）では、これが測れる費用になる（上の「速さの後始末」）。
  `Vm::intern` で取った `Sym` を渡せる `ivar_get_sym` / `ivar_set_sym` があれば消える。
  sabiruby `src/vm.rs:985-1001`。**VM（口の不足）／著者判断待ち。**
* **`sabiruby_compiler::Options::debug_info` の既定が `false`** なので、既定で組んだプログラムの例外は
  `backtrace` を持たない。rubevy の `Program` / `in_the_authors_lines`（R6）はコンパイラのメッセージの
  行番号を直す道具だが、**走り出してからの例外の行番号**は別の話で、ゲームが `-g` を渡しているかどうかで
  決まる。`docs/host-api.md` はこれに触れていない。rubevy（文書と実物のずれ、小）。
* **`ScriptWorld::unsubscribe` の中のローカル変数も `dropped` という名前**で、こちらは「手放すキューの列」
  という別の意味。R3 が `dropped` に「あふれで落とした件数」という意味を与えたので、同じファイルの中で
  1 語が 2 つを指すようになった。触っていない（範囲外）。`src/lib.rs` の `unsubscribe`。rubevy（読みやすさ、小）。
* **`r3_limits read` の上限 4096 の行だけフレーム中央値が 8.34 ms と跳ね、上限 8192 の行（3.68 ms）より
  大きい。** 繰り返しを 1 回しか取っていないので判断していない。R5 が `how_many_subscribers` を常設するとき、
  この形（上限を動かしながら読める件数を見る）も入れるなら、繰り返しを取って確かめる価値がある。
  rubevy（R5 の材料）。
* **`frame_time` が上限でないことが、この計測でも見えた。** 上限 2048 以上の 3 行は p95 が 7.0〜10.1 ms で、
  設定値の 8 ms を超える。R4 の仕事そのものなので直していない。計画書 3.4。
* **短い文字列 1 件が 300〜340 バイト**というのは、キューの Array の枠（16〜21 バイト）ではなく
  VM のヒープオブジェクト 1 個の値段である。`Answer::Text` を毎フレーム大量に publish するゲームは、
  数値で publish する同じゲームの 15〜20 倍のメモリを使う。`docs/host-api.md` に 1 行入れた。
  本（mruby の移植の章）の素材にもなりそう。
