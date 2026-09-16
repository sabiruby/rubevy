# 10 本まとめて入れ替えると VM が止まる、の再現

2026-09-17。ブランチ `restart-burst`、rubevy main は `fa37eaa`。
`rubevy_games/docs/worklog/2026-09-17-garden-G4.md` §5a の報告——
庭の生き物を 10 匹まとめて入れ替えると VM 全体が数秒止まり、
`Vm::task_pending()` は true のまま `task_run_limits` が `Ok` と 0 命令を返す、
9 匹なら平気、0.4 秒ずつ空ければ平気——を、rubevy のテストとして最小の形で再現した記録である。

原因は VM 側にあり、sabiruby のブランチ `task-end-nil` で直した
（`sabiruby/docs/worklog/2026-09-17-task-end-nil.md`）。ここに書くのは
**rubevy 側で何がその引き金を引いていたか**と、**再現テストをどう書いたか**。

## 1. 引き金は rubevy の購読の閉じ方だった

`ScriptTask` を外すと `stop_removed_task`（`src/lib.rs:145`）が走り、
タスクを `Task#terminate` して `ScriptWorld::unsubscribe(entity)` を呼ぶ。
タスクを止めるほうではなく、この `unsubscribe` が購読キューを閉じる。
閉じたキューで `pop` を待っていたタスクには `Rubevy::Unsubscribed` が上がる
（`docs/host-api.md`「A subscription is let go of …」、`tests/events.rs` の
`a_task_waiting_on_a_subscription_ends_when_the_script_does`）。

庭の生き物はその例外を **中身のない `rescue`** で受けている。Ruby では中身のない `rescue` 節の値は
nil なので、**反射タスクはどれも nil で終わる**。生き物 1 匹につき反射 6 本、10 匹で 60 本。

VM 側の `Vm::task_run_limited` は「`task_run_once` が nil を返したら、もう走るものがない」と
読んでいた。nil で終わったタスクの結果も nil なので、**60 本 × 1 フレーム**を丸ごと失っていた。
9 匹（54 本）で「平気」に見え 10 匹で「止まる」ように見えたのは、庭のそのときのフレームレートの差である。
「古い context の解放が追いつかない」という庭の読み方は外れていた。context は関係ない。

## 2. 再現テスト（`tests/restart_burst.rs`）

庭をそのまま持ってこられないので、**同じ形の最小**を書いた。

* スクリプト 11 本。10 本が生き物（`Rubevy.subscribe` 6 本 → それぞれに `q.pop` で待つ
  `Task.new` を 6 本 → 脳は `loop { Rubevy.ask("brain").pop }`）、
  反射タスクの `rescue` は庭と同じく**中身なし**。
* 11 本目は「見張り」。`loop { Rubevy.ask("watch").pop }` だけで、テストは一切触らない。
* 25 フレーム回して全員が動いていることを確かめ（`subscriptions()` が 60）、
  そのあと **1 フレームで 10 本全部**の `ScriptTask` を外して新しい `Script` を入れる。
* 主張は 2 つ。**そのあとの 3 フレームのどれもが見張りの命令数を増やすこと**と、
  3 フレーム後に新しい脳 10 本が 1 命令以上走っていること。

見張りを主役にしたのは、報告のいちばん悪い症状が「**入れ替えていないものまで凍る**」だったから。
入れ替えた側だけを見ると「新しいタスクの立ち上がりが遅い」と読めてしまう。

## 3. 数字

テストの中に一時的に 200 フレームぶんの計測を入れて取った（計測用の枝は消してある）。
「見張り」は 1 フレームあたり 53 命令ずつ進むスクリプトである。

直す前の VM（sabiruby `9c8ebf2`）:

```
PROBE frame 1: watcher 1305 (+0) brains 0
PROBE frame 20: watcher 1305 (+0) brains 0
PROBE frame 40: watcher 1305 (+0) brains 0
PROBE frame 60: watcher 1305 (+0) brains 0
PROBE frame 80: watcher 2365 (+53) brains 14390
PROBE frames in which the watcher did not move: 60 of 200
```

**ちょうど 60 フレーム。** 10 匹 × 反射 6 本 = 60 と一致する。
1 本の終了につきホストの 1 フレーム、という読みが当たっていた証拠である。

直した VM（sabiruby `task-end-nil`）:

```
PROBE frame 1: watcher 1358 (+53) brains 4320
PROBE frame 2: watcher 1411 (+53) brains 4850
PROBE frames in which the watcher did not move: 0 of 200
```

**0 フレーム。** 入れ替えた直後のフレームで新しい脳がもう 4,320 命令走っている。

テストそのものの結果:

```
（直す前）frame 1 after the burst ran nothing for the script nobody touched:
          1305 instructions before the frame, 1305 after
（直した後）test ten_scripts_restarted_in_one_frame_do_not_stop_the_vm ... ok
```

## 4. ビルドの都合（読む人へ）

**このテストは直した VM でしか通らない。** 確認した時点で sabiruby の修正はまだ公開も push も
されていないので、`Cargo.lock` が指している git の VM（`9c8ebf2`）では落ちる。
確認には `.cargo/config.toml`（git-ignore されている、`Cargo.toml` の冒頭のコメントが勧めている道）に

```toml
[patch."https://github.com/sabiruby/sabiruby"]
sabiruby = { path = "../sabiruby-wt-sched" }
sabiruby-compiler = { path = "../sabiruby-wt-sched/compiler" }
sabiruby-serde = { path = "../sabiruby-wt-sched/serde" }
```

を置いた。`Cargo.toml` には入れていないし、コミットもしていない
（patch を当てると `Cargo.lock` から `source` 行が消えるので、ロックも元に戻してある）。
**sabiruby の修正が push／公開されたら `cargo update -p sabiruby` でロックを進めること。**
それまでこのテストは CI で落ちる。

「落ちるテストを先に入れる」ことにしたのは、直った瞬間にロックを進めれば済み、
逆にロック待ちでテストを手元に置いておくと、何が直ったのかの証拠が残らないから。

## 5. rubevy 側で直すことは何もない

引き金を引いたのは購読の閉じ方だが、**あれは正しい**。
タスクを閉じたキューで起こさなければ、反射タスクは誰にも起こされないまま `WAITING` で残る
（`docs/worklog/2026-09-16-bridge-followups.md` で直したのがそれである）。
中身のない `rescue` もスクリプトの書き方の自由の範囲で、
「nil を返すな」とゲームの作者に言うのは筋が悪い。直すのは VM だった。

確認: `cargo test --workspace` が 53 件すべて通る（上の patch あり、11 ファイル）。
