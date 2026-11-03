# 0.4.0 の計画: SabiRuby 0.9.0 に上げる（依存のバージョンだけ）

作成 2026-11-03。SabiRuby の `docs/plans/release-0.9-plan.md` の R6（下流のバージョン上げ）。著者の判断（2026-11-03）で進めた。
形は 0.3.0（`release-0.3-plan.md`）と同じ。

SabiRuby 0.9.0 は 2026-11-03 に公開された（`sabiruby` 0.9.0、`sabiruby-compiler` 0.6.0、`sabiruby-serde` 0.5.0、tag `v0.9.0` = `a3a798d`）。
0.9.0 は VM に**足すだけ**のリリース（feature `checkpoint` は既定 off、`Vm::output_sink`）で、rubevy はどちらも使わない。
公開の前に SabiRuby の R3 が rubevy の写しで確かめてある（SabiRuby `docs/worklog/2026-11-03-release-0.9.md` の 4 節、249／251 passed）。

## 入れるもの

| # | 項目 | 中身 |
|---|---|---|
| V1 | 依存を 0.9 系に | `[dependencies]` の `sabiruby "0.9"`・`sabiruby-compiler "0.6"`、`[dev-dependencies]` の `sabiruby "0.9"`（`macros`）・`sabiruby-compiler "0.6"`・`sabiruby-serde "0.5"`。コメント（`rust-version` の出どころ、VM の下限の理由、compiler の対応）を 0.9／0.6 に |
| V2 | バージョン | `rubevy` 0.3.0 → **0.4.0**（`sabiruby` は公開の依存。0.x の caret では minor が major なので、`^0.8` のままだと 0.9 を名指しするアプリに VM が 2 つ入る）。`rubevy-build` 0.3.0 → **0.4.0**（0.3.0 と同じ慣習: minor を揃える） |
| V3 | 文書 | CHANGELOG を `## 0.4.0 — 2026-11-03` に切る。README の Install の数（`rubevy "0.4"`、VM 0.9、compiler 0.6、「The version is 0.4」）。`outlook.md`／`outlook.ja.md` の公開の表に 1 行 |

## 確かめ

0.3.0 と同じ段: `cargo check --workspace --all-targets --all-features`、`cargo test --workspace`（と `--all-features`）、
`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`（既定・`--all-features`・`-p rubevy-build`）、`cargo check --all-targets --features ruby-source`、
`cargo check --target wasm32-unknown-unknown`、`cargo tree -i sabiruby -e normal` で VM が 1 つ、`cargo clippy --workspace --all-targets` の警告が前からの 2 つだけ、
`cargo publish --dry-run`（`-p rubevy-build`、`-p rubevy`）。

## 公開の順

```
git push origin main
cargo publish --dry-run -p rubevy-build && cargo publish -p rubevy-build
cargo publish --dry-run -p rubevy       && cargo publish -p rubevy
git tag -a v0.4.0 -m "rubevy 0.4.0: on SabiRuby 0.9.0 (sabiruby 0.9, sabiruby-compiler 0.6); rubevy-build 0.4.0 (unchanged)" <この commit>
git push origin v0.4.0
```

## 状況

| 項目 | 状況 |
|---|---|
| V1〜V3 | 済み（2026-11-03）。記録は `../worklog/2026-11-03-release-0.4.md` |
| 公開 | この commit を push して出す（2026-11-03）。結果は SabiRuby の `docs/plans/release-0.9-plan.md` の R6 と、その worklog `2026-11-03-release-0.9.md` |
