# Backlog — 決めて後に回したもの

「やらないと決めたもの」「後で決めるもの」を 1 か所にまとめる。1 項目 1 行で、**何か／なぜ後にしたか／何が起きたら見直すか／詳しい記録**を書く。
やると決めて計画書に移したら、この表から消し、移った先を書く。

作成 2026-09-26（0.2.0 の範囲を決めたとき、`plans/release-0.2-plan.md`）。

## 振る舞い・API

| 項目 | なぜ後に | 見直すとき | 記録 |
|---|---|---|---|
| **同期アクセスの S5（同期の書き込み）** | 設計の判断が要る。今は書き込みを次のフレームに回す形で困っていない | 書き込みの 1 フレームの遅れが問題になったとき | `plans/sync-access-plan.md:155, 180` |
| **`gc_step(work)` とヒープの上限** | VM（SabiRuby）の機能で、設計の判断が要る | GC の停止時間かメモリの上限が問題になったとき | `outlook.md:183-184`、SabiRuby `docs/backlog.md` |
| 消えた entity への書き込みが黙って捨てられる | 要望待ち | 利用者が気づけずに困ったとき | `plans/generalize-plan.md:276` |
| 遅い `answer_in_tick` のクロージャに気づく道が無い | 要望待ち | 同上 | `plans/generalize-plan.md:287` |
| `ScriptEnded` が「始まらなかった」と「失敗した」を分けない | 要望待ち | 同上 | `plans/generalize-plan.md:289` |
| reflect_cache が古くなる | 要望待ち | 型を実行中に登録し直す使い方が出たとき | `plans/generalize-plan.md:307` |
| task ごとの予算の取り分 | 要望待ち | 1 本の task が予算を食い尽くす例が出たとき | `plans/generalize-plan.md:269` |
| 「深すぎて読めない」と本物の nil を区別できない | 文書で足りている | 利用者が取り違えたとき | `plans/generalize-plan.md:274` |
| `ScriptWorld::dropped` が VM 全体の合計で、購読ごとではない | 報告だけ | 購読ごとに知りたい例が出たとき | rubevy_games `docs/plans/factory-plan.md:290` |
| トップレベルの DSL では、例外の行番号を取れない | 報告だけ（汎用の API か、本の素材か） | DSL の誤りの報告を良くしたくなったとき | rubevy_games `docs/plans/factory-plan.md:288` |

## 文書・道具

| 項目 | なぜ後に | 見直すとき | 記録 |
|---|---|---|---|
| 「予算は床ではなく天井」の説明 | games から出た文書の宿題 | 次に `host-api.md` の Time の節を触るとき | rubevy_games `docs/plans/shared-crate-plan.md:201` |
| `helper.mrb` が古いことの検査 | 道具 | helper を変えるとき | `plans/generalize-plan.md:314` |
| Bevy の文字組みに日本語の分かち書きのデータが無い（ICU4X の警告が大量に出る。禁則が効いているかは未確認） | Bevy / parley の話で、rubevy の外 | Bevy を上げるとき | ある組み込み先の記録 |

## 著者の判断待ち（rubevy_games）

`rubevy_games/docs/plans/author-review.md` の B 節にある: Factory の地図の既定の大きさ（32×32 は仮）、Battle のボットの数に上限が無いこと、`rubevy-egui` の設定の下限、`--shot` の猶予の秒の出どころ、判定の作法を `implementer.md` に足すか、本の素材に写すもの。
