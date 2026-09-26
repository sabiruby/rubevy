# 0.2.0 の計画: SabiRuby 0.7.0 に上げ、たまった直しを片付ける

作成 2026-09-26。著者の判断（2026-09-26）:
- 「不具合や直すべき点をなるべく取り込んで、クリアにしてからリリースしたい」。
- 範囲は本体の案（この文書）でよい。
- 残したものは、後で参照しやすく整理する（[`../backlog.md`](../backlog.md)）。

SabiRuby 0.7.0（SabiRuby の `docs/plans/release-0.7-plan.md`）では、task がブロックの内側でも待てるようになり、待てない所の例外の文言も変わる。rubevy 0.2.0 はそれに上げ、次の R1〜R6 を入れて出す。

## 入れるもの

| # | 項目 | 種類 | 出どころ |
|---|---|---|---|
| R1 | **スクリプトの止め方が、文書と実際で違う。** `ScriptTask` だけを外しても、`Script` が残っていれば `start_scripts` が作り直す（`PendingScripts: Without<ScriptTask>, Without<ScriptDone>`）。rustdoc（`src/lib.rs` の 288 行付近と 475 行付近）と `host-api.md`「Replacing and removing a script」は「`ScriptTask` を外せば止まる」と書いている。**止めるための API（例 `ScriptWorld::stop` か `commands.entity(e).stop_script()`）を足し、文書を実際に合わせる** | 振る舞い・新 API | `../worklog/2026-09-22-held-requests.md:477`、ある組み込み先の記録（「タイトルへ」の後に止めた場面が流れ直した） |
| R2 | **答えずに捨てた問い合わせ**: `Request` を答えずに落とすと、キューの `gc_register` が漏れる（`ScriptWorld::forget(&Request)` が無い）。そのキューは閉じない（`unsubscribe` と違う）。**著者の判断（2026-09-26）: 閉じる。待っている側は例外で知る**（黙って止まり続けるのを避ける） | 不具合・振る舞い | `held-requests.md:486, 505` |
| R3 | **`Entity::try_from_bits(u64::MAX)` が `Some` を返す**ので、`Request::entity` が存在しない entity になる（`entity_from_bits`、`src/lib.rs:3566`） | 不具合 | `held-requests.md:494` |
| R4 | **`ScriptStats::location` / `frames` が、前置きの行数（`prelude_lines`）を引いていない** | 不具合 | `held-requests.md:508` |
| R5 | **SabiRuby 0.7.0 に上げる**: `Cargo.toml` の `sabiruby = "0.7"`（と compiler / serde の対応）。`src/prelude.rb:7-8` のコメントの古い例外の文言、`host-api.md` の 639・652・1076 行付近（「C function boundary」の説明）を新しい振る舞いに合わせる。待てる経路・待てない経路は SabiRuby の `docs/design/wait-anywhere.md` を引く | 依存・文書 | SabiRuby `docs/worklog/2026-09-26-wait-anywhere.md:535` |
| R6 | **任意の層を 2 つ足す**（`Rubevy::Camera` と同じ形の optional layer）: (a) **ポインタ**（押す・引く・落とすを、カメラを引数にして。「動いた」メッセージも）、(b) **control stage**（`on(:event) { … }` で出来事を受ける handler を task にする。`define_method` で handler をメソッドにする回避は、SabiRuby 0.7.0 で要らなくなるかを確かめて外す）。**どちらも、ある組み込み先で 3 回写したものを汎用にする**。題材には触れず、公開の例（`examples/`）と文書を付ける | 新 API | ある組み込み先の設計メモ（「エンジンに上げる候補」の 1 と 2） |

- rubevy_games の `Factory` の `Arms::waiting` と 30 行の前置きを `Held` に移すのは、games 側の後続（rubevy の `held-requests-plan.md:110-111`）。R6 の後に games を 0.2.0 に上げるときに一緒に行う。

## 守ること

- **rubevy と rubevy_games はブラウザ版（wasm32-unknown-unknown）でも動く**（`/home/kishima/book/CLAUDE.md`）。rubevy に手を入れた段階には、games の `web/build.sh garden` と Playwright（`~/.cache/ms-playwright` の Chromium）で、ページのエラーが 0 件であることと `?selftest` を確かめる段を入れる。
- unsafe はなるべく避ける。マジックナンバーは設定に出す（`docs/numbers.md`）。
- SabiRuby 0.7.0 が公開されるまでは、手元の SabiRuby を `[patch.crates-io]` の path で指して開発する（`Cargo.toml` のコメントの形）。**その patch はコミットしない**。公開後に外して、crates.io の 0.7 で建つことを確かめる。
- ベンチは本体が合図してから測る。

## 段階

| 段階 | 中身 |
|---|---|
| A | R1〜R4（SabiRuby 0.6 のままでできる） |
| B | R5（SabiRuby 0.7.0 の main を path で指して） |
| C | R6 |
| D | games を 0.2.0 に上げる（`Held` への移行を含む）、wasm の確認 |
| E | 公開: SabiRuby 0.7.0 の後に `cargo publish`（`rubevy`、`rubevy-build`）と tag。**著者の確認の後に** |

## 状況

| 段階 | 状況 |
|---|---|
| A | **済み（2026-09-26）**。R1 `stop_script` / `stop_script_for`、R2 捨てた問いはキューを閉じて `Rubevy::Unanswered`、R3 entity の無い問いは `None`、R4 `ScriptStats` の前置きの引き算（`require` したファイルは引かない）。計画外: 手で再起動したときに古い `Held` が残る不具合、2 度目の答えを断る。テスト 236、wasm（garden）pageerror 0・selftest 48 行。記録は `../worklog/2026-09-26-release-0.2-a.md`、CHANGELOG の Unreleased |
| B〜E | 未着手 |
