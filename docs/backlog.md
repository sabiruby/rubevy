# Backlog — 決めて後に回したもの

「やらないと決めたもの」「後で決めるもの」を 1 か所にまとめる。1 項目 1 行で、**何か／なぜ後にしたか／何が起きたら見直すか／詳しい記録**を書く。
やると決めて計画書に移したら、この表から消し、移った先を書く。

作成 2026-09-26（0.2.0 の範囲を決めたとき、`plans/release-0.2-plan.md`）。

## 振る舞い・API

| 項目 | なぜ後に | 見直すとき | 記録 |
|---|---|---|---|
| **同期アクセスの S5（同期の書き込み）** | 設計の判断が要る。今は書き込みを次のフレームに回す形で困っていない | 書き込みの 1 フレームの遅れが問題になったとき | `plans/sync-access-plan.md:155, 180` |
| **`gc_step(work)` とヒープの上限** | VM（SabiRuby）の機能で、設計の判断が要る | GC の停止時間かメモリの上限が問題になったとき | `outlook.md:223-224`、SabiRuby `docs/backlog.md` |
| **何も待っていない `Task.new` の子は、`stop_script` でも despawn でも止まらない**（`loop` や `sleep` の中にいる子。購読や held の問いで待っている子は `Unsubscribed` / `Unanswered` で巻き戻る） | 直すには VM が要る。SabiRuby の `Task` は作った側を記録しない（`TaskData` に親が無い）ので、rubevy からは「この実体のスクリプトが作ったタスク」を見つけられない。要るのは、タスクに作り手か持ち主の id を持たせること（例: `Task.new` の時点の `Task.current` を VM が記録し、`Vm::task_children(task)` か `Vm::task_terminate_tree(task)` で辿れる形）。rubevy 側だけで prelude の `Task.new` 上書きから一覧を持つ案もあるが、終わったタスクの片付けと GC の登録を rubevy が抱えることになる | 止めた・消した実体の子が動き続けて困る例が出たとき、または SabiRuby にタスクの親子が入ったとき | `worklog/2026-09-26-release-0.2-a.md` §4.3・§7-2、rustdoc の `stop_script` |
| **古いプログラムを自動で VM に返す**（今は `ScriptWorld::unload_programs` をゲームが呼ぶ） | 差し替えを rubevy は知らない（表のキーは bytes）、返したものを再び走らせると読み直しと空の irep 208 バイトが毎回かかる | 呼び忘れて irep が溜まる例が出たとき | `worklog/2026-09-27-release-0.2-bcd.md` §1.4 |
| ポインタが 2D だけ（3D は near plane の x・y） | 3D の「カーソルの下の地面」は面の選び方がゲームのもの | 3D のゲームが欲しがったとき | `worklog/2026-09-27-release-0.2-bcd.md` §2.2 |
| `WorldGrab` / `WorldClick` が画面の位置（`cursor`）を持たない | 0.2.0 の後、games がポインタに乗り換えて見つけた（2026-09-27）。画面の位置があれば、3D のゲームがクリックの判定（slop）だけ使い、光線は自分で飛ばせる。箱庭（3D）はこれが無いので自前のクリック判定のまま | 3D のゲームがクリックの判定を使いたがったとき、次にポインタのメッセージの形を変えるとき | rubevy_games `232ce84`（post-02） |
| 「この押し下げは UI が取った」と印を付ける口が無い | 同上（2026-09-27）。今は読む側ごとに、games-shell の `WorldClicks` のような上張り（UI の上の押し下げを捨てる層）を書くことになる | UI の上のクリックを捨てる上張りが 2 つ目の利用者にも要ったとき | rubevy_games `crates/games-shell/src/camera.rs`（`WorldClicks`） |
| 消えた entity への書き込みが黙って捨てられる | 要望待ち | 利用者が気づけずに困ったとき | `plans/generalize-plan.md:276` |
| 遅い `answer_in_tick` のクロージャに気づく道が無い | 要望待ち | 同上 | `plans/generalize-plan.md:287` |
| `ScriptEnded` が「始まらなかった」と「失敗した」を分けない | 要望待ち | 同上 | `plans/generalize-plan.md:289` |
| reflect_cache が古くなる | 要望待ち | 型を実行中に登録し直す使い方が出たとき | `plans/generalize-plan.md:307` |
| task ごとの予算の取り分 | 要望待ち | 1 本の task が予算を食い尽くす例が出たとき | `plans/generalize-plan.md:269` |
| 「深すぎて読めない」と本物の nil を区別できない | 文書で足りている | 利用者が取り違えたとき | `plans/generalize-plan.md:274` |
| `ScriptWorld::dropped` が VM 全体の合計で、購読ごとではない | 報告だけ | 購読ごとに知りたい例が出たとき | rubevy_games `docs/plans/factory-plan.md:290` |
| トップレベルの DSL では、例外の行番号を取れない | 報告だけ（汎用の API か、本の素材か） | DSL の誤りの報告を良くしたくなったとき | rubevy_games `docs/plans/factory-plan.md:288` |

直したもの: control 層の handler の例外の行が前置き込みだった件（2026-09-27 に組み込み先が見つけた）は、`control-lines` ブランチで `ScriptEnded::at` と同じ規則に揃えた（`worklog/2026-09-27-control-lines.md`、`CHANGELOG.md` の Unreleased）。

## 文書・道具

| 項目 | なぜ後に | 見直すとき | 記録 |
|---|---|---|---|
| 「予算は床ではなく天井」の説明 | games から出た文書の宿題 | 次に `host-api.md` の Time の節を触るとき | rubevy_games `docs/plans/shared-crate-plan.md:201` |
| `helper.mrb` が古いことの検査 | 道具 | helper を変えるとき | `plans/generalize-plan.md:314` |
| Bevy の文字組みに日本語の分かち書きのデータが無い（ICU4X の警告が大量に出る。禁則が効いているかは未確認） | Bevy / parley の話で、rubevy の外 | Bevy を上げるとき | ある組み込み先の記録 |

## 著者の判断待ち（rubevy_games）

`rubevy_games/docs/plans/author-review.md` の B 節にある: Factory の地図の既定の大きさ（32×32 は仮）、Battle のボットの数に上限が無いこと、`rubevy-egui` の設定の下限、`--shot` の猶予の秒の出どころ、判定の作法を `implementer.md` に足すか、本の素材に写すもの。
