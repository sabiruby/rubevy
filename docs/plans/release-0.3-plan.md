# 0.3.0 の計画: SabiRuby 0.8.0 に上げる（依存のバージョンだけ）

作成 2026-10-19。著者の指示（2026-10-19）: 「R5 の下流のバージョン上げ、進めて」（SabiRuby の
`docs/plans/release-0.8-plan.md` の R5）。

SabiRuby 0.8.0 は 2026-10-19 に公開された（`sabiruby` 0.8.0、`sabiruby-compiler` 0.5.0、`sabiruby-serde` 0.4.0、tag `v0.8.0` = `7f74cd1`）。
公開の前に SabiRuby の R2 が rubevy の写しで確かめてあり（SabiRuby `docs/worklog/2026-10-19-release-0.8.md` の 8 節）、
0.8 の壊す API（`HashData`、`InstanceKind`、`HeapObject`、`CallInfo`、引数の宣言）のどれにも rubevy は触っていない。
要るのは `Cargo.toml` のバージョンの 5 行と、その周りのコメントの文面だけ。

## 入れるもの

| # | 項目 | 中身 |
|---|---|---|
| V1 | 依存を 0.8 系に | `[dependencies]` の `sabiruby "0.8"`・`sabiruby-compiler "0.5"`、`[dev-dependencies]` の `sabiruby "0.8"`（`macros`）・`sabiruby-compiler "0.5"`・`sabiruby-serde "0.4"`。コメント（`rust-version` の出どころ、VM の下限の理由、compiler の対応）を 0.8／0.5 に |
| V2 | バージョン | `rubevy` 0.2.1 → **0.3.0**（`sabiruby` は公開の依存なので、利用者も 0.8 に上げる必要がある = 壊す変更）。`rubevy-build` 0.2.0 → **0.3.0**（下） |
| V3 | 文書 | CHANGELOG を `## 0.3.0 — 2026-10-19` に切る。README の Install の数（`rubevy "0.3"`、VM 0.8、compiler 0.5、「The version is 0.3」）。`outlook.md`／`outlook.ja.md` の公開の表に 1 行 |

**`rubevy-build` を 0.3.0 にする理由。** 中身は変わらず、依存も無い（std だけ）。それでも上げるのは、
`rubevy-build/Cargo.toml` のコメントが言う慣習（「`rubevy` と同じ数。1 つの repo・1 つの tag・1 つの CHANGELOG で一緒に出る」）と、
0.2.0 のときの前例（中身が変わらないまま `rubevy` と一緒に 0.2.0 に動かした。CHANGELOG の 0.2.0 の節）に合わせるため。
patch の 0.2.1 では据え置いた（0.2.x は 0.2.0 と組む）ので、慣習は「minor を揃える」と読める。据え置くと、
0.3.0 の `rubevy` と組むのが 0.2.0 の `rubevy-build` になり、コメントの慣習を書き直すことになる。

## 守ること

- 挙動を変えるコードの直しが要ると分かったら、止めて報告する（SabiRuby の R2 では要らなかった）。
- rubevy と rubevy_games はブラウザ版（wasm32-unknown-unknown）でも動く（`/home/kishima/book/CLAUDE.md`）。wasm の確かめは games の段（`web/build.sh garden` と Playwright）で取る。
- push・`cargo publish`・tag は本体が、著者の確認の後に行う。

## 確かめ

`cargo check --workspace --all-targets --all-features`、`cargo test --workspace`（と `--all-features`）、
`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`（既定・`--all-features`・`-p rubevy-build`）、`cargo check --target wasm32-unknown-unknown`、
`cargo tree -d` で VM が 1 つ、`cargo clippy --workspace --all-targets` の警告が前からの 2 つだけ、
`cargo publish --dry-run`（`-p rubevy-build`、`-p rubevy`）。

## 公開の順（本体）

```
git push origin main
cargo publish --dry-run -p rubevy-build && cargo publish -p rubevy-build
cargo publish --dry-run -p rubevy       && cargo publish -p rubevy
git tag -a v0.3.0 -m "rubevy 0.3.0: on SabiRuby 0.8.0; rubevy-build 0.3.0 (unchanged)" <この commit>
git push origin v0.3.0
```

互いに名指ししないので順序は自由（0.1.0 からの慣例で `rubevy-build` を先に出す。README がそのリンクを踏めるように）。

## 状況

| 項目 | 状況 |
|---|---|
| V1〜V3 | 済み（2026-10-19）。記録は `../worklog/2026-10-19-release-0.3.md` |
| 公開 | 未（本体） |
