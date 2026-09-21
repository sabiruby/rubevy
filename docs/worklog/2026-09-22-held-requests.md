# 2026-09-22 「終わるまで待つ動作」の口（`hold_requests` / `Held`）と、止まった場所を持つ `ScriptEnded`

計画書 `docs/plans/held-requests-plan.md` の段階 **H0 → H1 → H2 → H3**。案は 2026-09-21 に書かれ、
著者が 2026-09-22 に「この形で F4 の前に入れる」と承認した（名前は案のまま:
`hold_requests` / `Held` / `ScriptEnded::at`）。

作業場所は worktree `/home/kishima/book/kishima/rubevy-wt-held`、ブランチ `held`、着手時の先頭は
main と同じ `7d6bdc0`。`CARGO_TARGET_DIR` は指定せず worktree 自身の `target/`。補助ファイルは
スクラッチパッドの `held/` に置き、repo には入れない。

**着手時の数**: `cargo build --all-targets` 警告 0、`cargo test` **180 passed / 0 failed**
（doctest 19 を含む）。`cargo clippy --workspace --all-targets` は `docs/README.md` が書いている
2 件の常設警告（`src/lib.rs` の `collapsible_if`、`examples/headless.rs` の `type_complexity`）。
着手時の機械は `vmstat 1 3` で idle 99%。計測の節（§3.4）の時点では隣の担当の ruby と
docker-compose が動いていたので、そこには「並行実行あり」と書いてある。

---

## 1. H0: 調査 — 箱庭と Battle に同じ形はあるか

### 1.1 「request を持ち続けて後のフレームで答える」形

**無い。Factory だけである。** `take_requests` を呼んでいるのは 4 か所で、

| 場所 | 何をしているか |
|---|---|
| `factory/src/inserters.rs:616`（`answer_moves`） | `factory.move` を `Arms::waiting: HashMap<usize, Request>` に入れ、`finish_swings` が後のフレームで答える |
| `garden/src/main.rs:5408` | その場で全部答える（`world.answer(&request, …)`）。`Request` を持ち越す場所が無い |
| `garden/src/main.rs:5855`（world VM） | 同じ。答えを作るのに時間はかからない |
| `sabibots/src/main.rs:2820` | 同じ。**消えたエンティティの request を `Answer::Nil` で答える 3 行**（`commands.get_entity(e).is_err()`）はあるが、持ち越しではない |

`Request` を構造体のフィールドに持っているのは `grep -rn 'Request' --include=*.rs` で
`factory/src/inserters.rs:181` の 1 行だけだった。つまり案が消す索引と後始末は、**今は 1 つの
ゲームにしか無い**。それでも入れる理由は計画書が書いているとおり（F4 の control stage が同じ形に
なる、Ruby を書かせるゲームは全部同じものを書く）で、**この調査は「他にもある」を確かめに行って
「無い」を確かめて帰ってきた**ことを記録しておく。紙の上の確認としては、Factory の
`Arms::waiting`（タイル → `Request`）と `Arms::forget`（撤去のときに行を消す）と
`how_many_waiting`（HUD と判定が読む数）の 3 つが、それぞれ `Held` component・Bevy の despawn・
`Query<&Held>` の `iter().count()` に置き換わる。タイルではなくエンティティが鍵になるが、
Factory は `Crew::at(tile)` でタイル → エンティティを既に持っている。

### 1.2 「壊れたスクリプトの場所を自前で拾う」形

**Factory が 30 行を書いており、Battle は途中まで書いている。箱庭は何も書いていない。**

* Factory: `factory/ruby/prelude.rb` の `Inserter.said_at`（`error.backtrace.first` を割って
  `prelude_lines` を引く 15 行）と `run_inserter` の `rescue`（`@broke_at` に置く 4 行）、
  `factory/src/inserters.rs:706` の `where_it_broke`（`ivar_get` で読む 5 行）と
  `watch_endings` の組み立て。
* Battle（sabibots）: `report_ended` は `e.value` をログに出すだけで**場所を出していない**。
  一方 `update_hud` は**生きているタスク**について `world.stats(script).location` を読み、
  `robot.prelude_lines` を自分で引いて `panel.at` を作っている（`sabibots/src/main.rs:3406` 以降）。
  つまり「prelude の行数を引く」計算は Battle にも `rubevy-egui/src/inspect.rs:494` にも既にある。
* 箱庭: `ScriptEnded` を読む system が無い。

なので `ScriptEnded::at` は Factory の 30 行を消し、Battle の `report_ended` に 1 行足せば
場所を言えるようになる、という形で 2 本に効く。

### 1.3 決めたこと（と理由）

計画書 H0 が「決めること」として挙げた 8 点。小さい判断は原則（外の利用者にとって素直か、
今の API を壊さないか、登録しなければ費用が無いか）で決めて、ここに理由を残す。

1. **`Held` の中身は `Vec<Request>`（古い順）**、`HashMap<kind, Request>` ではない。
   1 つのエンティティが同じ種類を 2 つ同時に待つことは**ありうる**からである: スクリプトが
   `Task.new` で作ったタスクは同じ `@rubevy_entity` を継ぎ、購読のハンドラも同じエンティティで
   走る（`docs/host-api.md`）。Map にすると後から来た方が先の方を黙って落とす。
   テスト `two_tasks_of_one_entity_wait_side_by_side` が実際に 2 つ並ぶことを見ている。
2. **`held.answer(kind)` は「いちばん長く待っている方」に答える。** 古い順は VM が起こしたで
   あろう順で、`unsubscribe` が `Subscription::seq` で並べ直しているのと同じ理由（起きる順が
   プロセスごとに変わらない）。
3. **型は `Held<M>`。** `ScriptTask<M>` と同じ理由で、`Request` は特定の VM のヒープを指す
   `ObjId` を持っているので、別の VM の `ScriptWorld` から答えられてはいけない。`M` を型に
   置けば取り違えはコンパイルエラーになる。
4. **載せるのは `drain_commands` の中**（`RubevySet::Tick` の中）で、`RubevySet::Answer` に
   rubevy の system を足さない。理由は 2 つ: (a) `Answer` は「ゲームの set」だと
   `RubevySet` の rustdoc が言っており、そこに rubevy のものを入れると説明が 1 つ増える;
   (b) **登録しなければ費用 0** という条件は「system を 1 本も足さない」が最も素直な満たし方で、
   `drain_commands` は既に `Commands` も問いも持っている。set の境界（`.chain()`）が同期点なので、
   `Tick` の中で積んだ command は `Answer` の system が走る前に反映される —
   `Added<Held>` は**同じフレームの `RubevySet::Answer` から見える**（テスト
   `a_held_question_waits_on_the_entity_that_asked` がフレーム番号で見ている）。
5. **エンティティを持たないスクリプトの問いは、今までどおり `take_requests` へ。** 案のとおり。
   実装では「待つ相手がいるか」を `ScriptTask` の有無で見るので、条件は「エンティティがある」
   ではなく「そのエンティティに**生きているスクリプトがある**」になった（§3.2）。
6. **空になったら component ごと外す。** 外すのは `drain_commands` の頭。`Added<Held>` が
   「待ち始めた」を意味し続けるために必要で、これが実装で 1 つ目の罠になった（§3.3）。
7. **`Reflect` は付けない。** 中身は特定の VM の `ObjId` と `Value` で、シーンにもインスペクタにも
   乗らない。待っている問いは保存できない（計画書 §2 が言っているとおり、タスクの途中を持てない
   のと同じ理由）ので、付けても嘘になる。
8. **`Clone` も付けない。** `Request` の複製は「2 回答えられる」ということで、2 回目は
   `gc_unregister` 済みのキューに push することになる。

形は案から変わっていないので、止まらずに H1 に進んだ。

---

## 2. 読んだもの（今の実物と案のずれ）

* `src/lib.rs` の `Request`（755 行あたり）・`ScriptEnded`（502 行あたり）・`answer_with`（1717 行
  あたり）は計画書の行番号のままだった。
* **`drain_commands` が `world.requests` に積む唯一の場所**であることを確かめた
  （`grep -n 'requests.push' src/lib.rs`）。だから種類による仕分けを 1 か所に足せば済む。
* `Request::queue` は `Rubevy.ask` のネイティブが `vm.gc_register(queue)` しており
  （`src/lib.rs:4185`）、`push_answer` が `gc_unregister` する。**答えずに捨てた `Request` は
  この登録を VM の寿命ぶん残す。** 今の Factory（`Arms::forget`）もこれを漏らしている。
  `Arg::Value` の方は `RootedValue` の `Drop` が release queue に積むので自動だが、キューの方は
  誰も外さない。`Held` はこれを外す側に回る（§3.1 の `release_held`）。
* `Vm::gc_registered` は **`pub` のフィールド**（sabiruby 0.5.2 `src/vm.rs:461`）なので、
  テストが「宛先の無い登録が残っていないこと」を数で言える。後始末のテストは全部これで書いた。
* **計画書 §2 の「`Program` が既に知っている（`in_the_authors_lines`）」は、rubevy から見ると
  そうではない。** `Program` はゲームが作ってコンパイラに渡す値で、rubevy は
  `Script { source, priority, name }` しか持っていない。H2 の設計はここから変わっている（§4.1）。

---

## 3. H1: `hold_requests` と `Held`

### 3.1 形

足したもの（全部「足すだけ」で、今の API は 1 つも変わっていない）:

* `Held<M>` — component。中身は非公開の `Vec<Request>`。`answer` / `answer_value` / `take` /
  `waiting` / `has` / `len` / `is_empty`。`#[component(on_remove = release_held::<M>)]`。
* `HoldRequests` — `App` の拡張 trait。`hold_requests(kind)` と `hold_requests_for::<M>(kind)`。
* `ScriptWorld::hold_requests(kind)` と `ScriptWorld::held_kinds()`。
* `ScriptWorld` の非公開フィールド `held_kinds: HashSet<String>`。
* `replace_script` が `Held<M>` も外す。

**`App` の口と `ScriptWorld` の口の両方を置いた理由。** 計画書（＝著者が見た案）は
`app.add_plugins(…).hold_requests("factory.move")` という綴りで、これを満たすには `App` の
拡張が要る。一方、置き場所は `ScriptWorld` にしかない（VM ごとの設定で、`drain_commands` が
読む）ので、resource 側の口は実装上どのみち存在する。2 通りの書き方になるが、
`answer_in_tick` が `Startup` system から resource を触る形であることと揃うし、
bevy 自身が `app.add_message::<T>()` と `world.insert_resource(Messages::<T>::default())` の
両方を持っている。`App` の側は 3 行のうち 2 行が `error!` で、resource が無ければ
「plugin より前に呼んだ」と言う。

**後始末を「component の removal hook」1 本に集めた。** これが設計の芯である。`Held` が
エンティティから外れる道は 5 つ（答え切った・despawn・`replace_script`・スクリプトが終わった・
`ScriptTask` を手で外した）あるが、**どの道も最後は「component が外れる」に合流する**ので、
キューの登録を外すのは `release_held` 1 か所で足りる。`ScriptTask` の `stop_removed_task` を
真似た形で、`DeferredWorld` から `ScriptWorld<M>` を取って `gc_unregister` するだけ。

**捨てた案: 外すときにキューを `close` する。** sabiruby の `Task::Queue#close` は待っている
タスクを起こし、`pop` は `nil` を返す（`ext_task.rs:881`）。`unsubscribe` は購読のキューに
これをやっている（「誰も読まないキューで待つタスクは VM の寿命ぶん立ったままになる」）。
同じ理屈は捨てられた `Rubevy.ask` のキューにも当てはまる — スクリプト本体のタスクは 5 つの道の
どれでも終了させられるが、`Task.new` の子タスクは終了させられないので、永久に park する。
**やらなかった**のは、(a) 今の rubevy も、今の Factory も、`Request` を捨てるときに close して
いないので、close は「足す」ではなく「振る舞いを変える」ことになる; (b) `move` が `false` では
なく `nil` を返す道が新しくできる; (c) 計画書が何も言っていない。**著者判断待ちとして報告に出す。**

### 3.2 「待つ相手がいるか」で仕分ける

最初の版は「エンティティを持つ問いなら held」だった。テストが 2 つ落ちて、2 つとも同じことを
言っていた。

**1 つ目**: `Rubevy.ask('wait')` の直後に `raise` するスクリプト。`tick_scripts` はその tick の
中で `ScriptDone` を入れて `ScriptEnded` を送るが、**問いが `Request` になるのは同じフレームの
`drain_commands`（あと）**なので、既に死んだスクリプトのエンティティに `Held` が載る。
`Added<Held>` が鳴り、ゲームは死んだ腕を振り始める。

**2 つ目**: エンティティを持たないタスク。`current_entity` は `Value::Nil` を返し、
`HostCommand::Ask` は `u64::MAX` を積む。ところが **`Entity::try_from_bits(u64::MAX)` は
`Some` を返す**（generation が 0 でなければ有効なビット列）ので、`request.entity.is_some()` は
真になってしまう。`commands.entity(<存在しないエンティティ>).try_insert(…)` は何もせず、
問いは消え、キューの登録が残る。

どちらも「エンティティがある」ではなく「**そのエンティティに生きているスクリプトがある**」で
見れば消える。`live: Query<(), (With<ScriptTask<M>>, Without<ScriptDone<M>>)>` を足して
`live.contains(entity)` にした。当てはまらない問いは**今までどおり `world.requests` に行く** —
捨てるのではなく。捨てると「誰も答えない問い」を rubevy が作ることになり、`take_requests` に
出せばゲームが答えて（sabibots は既にこの形のために 3 行書いている）キューも解放される。
`ScriptWorld::hold_requests` の rustdoc に「登録した種類でも `take_requests` に出ることがある」と
書いた。

### 3.3 罠: 空になった `Held` の削除と、同じフレームの再質問

`examples/walk_to.rs` を書いたら **1 本目の脚だけ歩いて止まった**。

```
INFO walk_to: arrived at Vec2(8.0, 0.0)
INFO rubevy: [script] leg 0: there
（ここで止まる）
```

原因は command の順序だった。`drain_commands` の頭で空の `Held` に `try_remove` を積み、
同じ system の終わりで**その component に新しい問いを push** していた。command は後で適用
されるので、**削除が新しい問いごと持っていく**。しかも `release_held` が走るので、キューの登録も
外れ、スクリプトは永久に park する。

直し方は、掃いたエンティティを覚えておいて（`swept: Vec<Entity>`）、その分は
既存の component に push せず「新しい component」の側に回すこと。すると command は
「削除 → 挿入」の順に並び、**`Added<Held>` がもう一度鳴る**。これは副作用ではなく
**欲しかった性質**である: `loop { move }` は「待ち終わって、また待ち始める」ので、
1 回の問いに 1 回の `Added` が鳴るのが正しい。テスト
`a_question_asked_again_is_a_new_arrival` が 3 周まわして `added == 3`・`removed == 3` を見る。

**`Added<Held>` で足りない場合も書いた**: 1 つのエンティティが同時に 2 つ以上待つときは、
2 つ目は既にある component に入るので `Added` は鳴らない。`Changed<Held>` と `waiting()` を
読む、と rustdoc に書いた。

### 3.4 費用: 登録しなければ 0

`examples/how_many_scripts` を main（`7d6bdc0`）とこのブランチで別々の `CARGO_TARGET_DIR` に
release で建て、`md5sum` で別のバイナリだと確かめてから、`taskset -c 2` で**交互に 6 巡**
（3 巡は before→after、3 巡は after→before）走らせた。**並行実行あり**: 隣の担当の `ruby` と
`docker-compose` が動いており、`vmstat 1 3` の idle は 88〜93% だった。

```
$ md5sum hms-before hms-after
f9c948de60bd0fcc70104bc9815af640  hms-before
05acae8acfb8cc0357611912fd9757c3  hms-after
$ taskset -c 2 ./hms-<版> 90 3 1000 0.004
```

`default(200k/8ms)` の行の `stats_tick`（＝`FrameStats::time_ns` の中央値、`frame_time` が
縛っている数）:

| 巡 | 1 | 2 | 3 | 4 | 5 | 6 | 最小 | 最大 |
|---|---|---|---|---|---|---|---|---|
| before | 7.538 | 7.540 | 7.039 | 7.124 | 8.309 | 6.866 | 6.866 | 8.309 |
| after | 8.004 | 6.936 | 8.008 | 7.494 | 6.402 | 6.584 | 6.402 | 8.008 |

**版の中のばらつき（1.4〜1.6 ms）が版の間の差（中央値で 0.1 ms）より大きい。** 同じ巡の中で
`insn` の列が 122,000 になったり 33,253 になったり（＝スクリプトが予算を使い切れなかった巡が
ある）しているのがその証拠で、これは機械が別の仕事をしていたということである。
`insn=122000` かつ `frames_per_turn=1.0` の「素直に回った」巡だけを取ると
before 6.866 / 7.039、after 6.402 / 6.584 / 6.936 で、**after の方が速い** — つまり差は
計測器に見えていない。

機構から言える理由も書いておく。登録が 0 のとき 1 フレームに増えるのは
(a) 0 件の `Query` を 1 回まわすこと、(b) `Vec::new()` 1 つ（確保なし）、(c) ゲームの問い 1 件
につき `HashSet::is_empty()` 1 回、の 3 つだけで、system は 1 本も増えていない。

**数は 1 つも足していない**（`docs/numbers.md` に行は増えない）。`Held` に上限は置いていない:
待てる問いの数はスクリプトが問うた回数で、それは `budget` が縛っている（`rejected_writes` が
上限を置かなかったのと同じ論法）。

### 3.5 テスト（`tests/held.rs`、15 本）

| 場面 | テスト | 見ているもの |
|---|---|---|
| 着く | `a_held_question_waits_on_the_entity_that_asked` | `Added<Held>` が 1 回、`take_requests` には出ない、答えるとスクリプトが進む |
| 同時に 2 つ | `two_tasks_of_one_entity_wait_side_by_side` | 古い順、`answer` は古い方 |
| 繰り返し | `a_question_asked_again_is_a_new_arrival` | `added == removed == 3` |
| 登録していない種類／エンティティ無し | `what_is_not_held` | `take_requests` に出る |
| 予約された種類・in-tick の種類 | `what_cannot_be_held` | 登録されない（`held_kinds() == 1`） |
| 答えた | `an_answer_lets_the_queue_go` | 登録数 2 → 1 |
| `Arg::Value` つき | `a_question_that_carried_a_hash` | 登録数 3 → 1（キューと Hash の両方） |
| despawn | `a_despawned_entity_leaves_no_question_behind` | 3 → 0 |
| `replace_script` | `replace_script_takes_the_waiting_question_with_it` | 3 → 1（新しいタスクだけ） |
| `ScriptTask` を手で外す | `a_script_stopped_by_hand_is_swept` | 3 → 0 |
| 正常に終わった | `a_script_that_ends_takes_its_waiting_questions_with_it` | 終了のフレームで `Held` が消える、3 → 0 |
| 例外で終わった（同じ tick で問うた） | `a_script_that_raised_asks_but_does_not_wait` | `Held` に載らず `take_requests` に出る |
| 例外で終わった（前のフレームから待っていた） | `a_script_that_raised_takes_its_waiting_questions_with_it` | 3 → 0 |
| `Task::Overrun` | `a_script_that_overran_takes_its_waiting_questions_with_it` | 3 → 0 |
| 一時停止 | `a_pause_keeps_what_is_waiting` | 止まっている間は減りも増えもせず、再開後に答えが届く |

`a_script_stopped_by_hand_is_swept` は最初 `ScriptTask` だけを外して書いて落ちた。
**`Script` を残したまま `ScriptTask` を外すと、`start_scripts` が次のフレームで走らせ直す**
（`PendingScripts` が `Without<ScriptTask>, Without<ScriptDone>` だから）。`ScriptTask` の
rustdoc と `replace_script` の rustdoc は「`ScriptTask` を外せば止まる」と書いているが、
止まるのは**タスク**であってスクリプトではない。気づいた点に書いた。

### 3.6 `examples/walk_to.rs`

ゲームの語彙に寄らない実例。`walk_to(x, y)` が「歩き終わるまで返らない」1 行で、`Held` の
3 つの使い方（`Added<Held>` で動作を始める、`waiting()` で引数を読む、`answer` で答える）が
1 本ずつ出てくる。数は 1 つだけ（`speed: 4.0`）で、「何フレームかかかるくらい遅い」以外の
根拠は無く、example のものなのでそう書いてある。最後の 1 歩は目標に**ぴったり載せる**ので
「着いた」が許容誤差ではなく `==` で言える（根拠の無い許容誤差を置かずに済む）。

```
$ ./target/debug/examples/walk_to
INFO walk_to: arrived at Vec2(8.0, 0.0)
INFO rubevy: [script] leg 0: there
INFO walk_to: arrived at Vec2(4.0, 0.0)
INFO rubevy: [script] leg 1: there
INFO walk_to: arrived at Vec2(0.0, 0.0)
INFO rubevy: [script] leg 2: there
```

---

## 4. H2: `ScriptEnded::at`

### 4.1 計画書の前提が実物と違っていた — `prelude_lines` は rubevy に無い

案は「prelude の行数を引く計算は `Program` が既に知っている（`in_the_authors_lines`）ので、
ゲームはそれを書かなくてよくなる」と書いている。**知っているのはゲームであって rubevy ではない。**

* `Program { source, name, prelude_lines }` はゲームが組み立ててコンパイラに渡す値で、rubevy は
  受け取らない。走っているスクリプトについて rubevy が持っているのは
  `Script { source, priority, name }` だけである。
* `in_the_authors_lines(message, prelude_lines, prelude_name)` は**コンパイラのメッセージ**を
  書き換える関数で、実行時の backtrace は扱わない。引数の `prelude_lines` は呼び手が渡す。

だから `ScriptEnded::at` が「著者の行」を言うには、`prelude_lines` が**スクリプトと一緒に
走っていなければならない**。3 つ考えて、1 つ目を採った。

| 案 | 採否 |
|---|---|
| (a) `Script::prelude_lines`（既定 0）を足し、`with_prelude_lines` で渡す | **採用。** 足すだけで壊れない（`_m` が非公開なので外から構造体リテラルで作れず、フィールドが増えても構築は壊れない — `ScriptEnded` と同じ論法）。`Program::prelude_lines` から `ScriptEnded::at` まで線が通る |
| (b) `at` は**プログラムの行**を返し、引き算はゲームがやる | 捨てた。`src/source.rs` の冒頭が書いている失敗（「著者が 118 行目に書いたものを 600 行目と報告した」）をそのまま再現する口になる。しかも backtrace のどの frame が著者のものかを選べないので、DSL の中で raise したときは DSL の行が出る |
| (c) `at` に backtrace 全部（`Vec<(String, u32)>`）を入れる | 捨てた。選ぶ 20 行をゲームが書くことになり、H2 が消したかった 30 行が 20 行になるだけ。終わるたびに Vec を 1 つ作る |

**`Script` にフィールドを足すことは案の範囲外**なので、原則（外の利用者にとって素直か・今の API
を壊さないか）で決めて理由をここに残す。壊れる利用者はいない（crates.io に出ていないし、
games には `Script { … }` の構造体リテラルも 1 つも無い — `grep` で確かめた）。

### 4.2 形: 文字列 1 つか、ファイルと行か

**`at: Option<(String, u32)>` にした。** 案の例は `"inserter.rb:27"` という文字列 1 つだが、

* rubevy が今「場所」を返しているのは `ScriptStats::location: Option<(String, u32)>` と
  `ScriptStats::frames: Vec<(String, u32)>` の 2 つで、**どちらも組**である。Battle の
  `update_hud` も `rubevy-egui` の `VmInspector::fill` もこの組を受けて `format!` している;
* 行番号を数として要る利用者がいる（エディタのパネルがその行へ飛ぶ）。文字列から取り出すのは
  `factory/src/data.rs::line_in` が書いている 15 行で、それを書かせないための口である;
* 文字列が要る側は `format!("{file}:{line}")` の 1 行で済む。

`in_the_authors_lines` は「メッセージ全体を書き換えて返す」関数で場所を返していないので、
「合わせる」相手は `ScriptStats` の方だと判断した。**名前は案のまま `at`。**

### 4.3 終わったタスクから場所を読む道

探りのテストを書いて実測した（`tests/zz_probe.rs`、コミットには入れず消した）。
`ScriptEnded` が届く瞬間に:

```
--- raise, nested (debug_info=true) ---
  finished=true location=None frames=[]
  is_exception=true inspect=Ok("#<RuntimeError: boom>")
    frame Some("t.rb:2:in inner")
    frame Some("t.rb:5:in outer")
    frame Some("t.rb:7")
--- raise, no debug info (debug_info=false) ---
    （frame 無し）
--- normal end (debug_info=true) ---
  is_exception=false inspect=Ok("2")
    backtrace raised: undefined method 'backtrace' for Integer (NoMethodError)
--- overrun (debug_info=true) ---
  is_exception=true inspect=Ok("#<Task::Overrun: the task ran past its time limit inside a call that cannot be switched out>")
    frame Some("t.rb:2")
    frame Some("t.rb:2")
```

**タスクにフレームは残っていないが、例外はフレームを持っている。** SabiRuby は raise の時点で
`[irep, pc, mid]` の組をオブジェクトの `@__bt` に置き、文字列は訊かれたときに作る
（`Vm::keep_backtrace` / `backtrace_text`）。だから `funcall(value, :backtrace)` で読める。

**`@__bt` を `ivar_get` で直接読む道は採らなかった**: 名前は VM の内部の綴りで、
`Exception#backtrace` の方が公開の道である。しかも `factory/src/data.rs::from_raise` が既に
この道を通っている。走るのは**失敗した終わりのときだけ**で、正常終了は `is_exception` が
偽なので問い合わせもしない。

**`Task::Overrun` にも場所がある**のが、この形のいちばんの拾い物である。`Overrun` は
`Exception` であって `StandardError` ではないので Factory の `rescue => e` は捕まえられず、
あの腕は `inserter.rb:?` と言っていた。rubevy が例外そのものから読むなら、そこが言える。

frame の綴りは `file:line` か `file:line:in method` なので、`:in ` を**右から**探して落とし、
残りを最後のコロンで割る（ファイル名にコロンが入っていてもよい）。`:0` の frame は
「行表が無い irep」なので飛ばす。

線引きは `in_the_authors_lines` と同じ「`prelude_lines` より大きいか」で、
**内側から外へ**見ていって最初に見つかった著者の行を返す。DSL の中で raise したときに
「それを呼んだ著者の行」が出るのはこれによる。

### 4.4 prelude の中で壊れたら何を言うか

**著者の行が 1 つも無ければ `None`。** 案がそう書いており、実物でもそれが正しかった:

* **ふつうは `None` にならない。** prelude のメソッドの中で raise しても、それを呼んだのは
  著者の行なので、外側の frame にそれがある（テスト
  `a_raise_inside_the_prelude_is_reported_at_the_line_that_called_it`）。
* `None` になるのは「著者のファイルに一度も入らずに prelude が raise した」ときだけで、それは
  `run_inserter` の `raise "this file defines no inserter"` のような場合である。**指させる
  著者の行が本当に無い**ので、何が起きたかは `value` が言い、場所は言わない。

prelude 側の行を `("prelude.rb", 120)` のように返す道も考えたが、**rubevy は prelude の名前を
知らない**（`in_the_authors_lines` はそれを引数で受け取る）ので、名前を置くなら `Script` に
もう 1 つフィールドが要る。指させない行を出すために公開のフィールドを増やすのは釣り合わないと
判断した。ゲームは `value` を持っている。

### 4.5 テスト（`tests/script_ended_at.rs`、9 本）

素の行、prelude つきで著者の行、prelude の中で raise → 呼んだ行、prelude だけで raise → `None`、
正常終了 → `None`、デバッグ情報なし → `None`、`Task::Overrun` → 行が出る、起動できなかった
スクリプト → `None`、そしてゲームが書く形の通し 1 本。

`Program::new` の `prelude_lines` が 6 ではなく **7** であることを推測で書いて 1 回落とした
（prelude と著者のファイルの間に空行と区切りのコメントが入る）。テストの側を直し、なぜ 7 なのかを
コメントに書いた。

`grep -rn 'ScriptEnded\s*{'` を rubevy_games に対して回して、**`..` なしで分解している所は
1 つも無い**ことを確かめた（読んでいるのは `e.entity` / `e.status` / `e.value` だけ）。
`Script { … }` の構造体リテラルも 0 件。
