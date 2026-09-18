# rubevy を crates.io に出す — 依存を git から版へ

2026-09-18。著者: 「Rubevy のゴールは汎用の SabiRuby つなぎこみ機能だ、Bevy の」。汎用の
ライブラリなら crates.io から `cargo add rubevy` で使えるのが筋なので、初公開する。
`rubevy-arena`（rubevy_games の中の、ゲーム用の 2D の床）は rubevy のゴールの外なので出さない。

## 1. 今

* `rubevy` 0.0.1 は crates.io に無い（`/api/v1/crates/rubevy` が「does not exist」）。
* `Cargo.toml` は SabiRuby の 4 つを **git 依存**で書いている（`[dependencies]` の `sabiruby` と
  optional の `sabiruby-compiler`、`[dev-dependencies]` の `sabiruby-compiler`・`sabiruby`
  （feature `macros`）・`sabiruby-serde`）。crates.io は git 依存のある crate を受け付けない。
  コメント自身が「API が落ち着いたら crates.io の版に戻す」と言っている。
* crates.io の SabiRuby は今日そろった: `sabiruby` 0.5.2、`sabiruby-compiler` 0.2.3、
  `sabiruby-macros` 0.1.0、`sabiruby-serde` 0.1.0（sabiruby `docs/worklog/2026-09-18-release-0.5.2.md`）。
  0.5.2 の中身は main `7be7b86` と同じで、rubevy_games はその rev で動いている。
* rubevy の `Cargo.lock` は SabiRuby を `e2470626`（= 0.5.0）で止めている。rubevy 自身のテストは
  0.5.1 以降でまだ回していない。
* **rubevy_games は rubevy を git で、SabiRuby も git で引いている**（`Cargo.toml` の
  `[workspace.dependencies]`）。理由はそこのコメントのとおり「同じ source でないと VM が 2 つ入る」。
  rubevy が crates.io の `sabiruby` を指すようになると、games が git の `sabiruby` を指したままでは
  まさにその 2 つになる。
* rubevy_games の `.github/workflows/pages.yml` は、Playground のコンパイラを games と同じ
  SabiRuby で作るために、**`Cargo.lock` から `github.com/sabiruby/sabiruby#<rev>` を grep している**
  （37 行目）。games の lock から git の行が消えると Pages の CI が壊れる。

## 2. 決めたこと

1. **rubevy は版で依存する。** 下限は次のとおり（数の出どころ付き）:
   * `sabiruby = "0.5.1"` — 0.5.1 はスケジューラの修正（nil で終わるタスク、dormant キュー）で、
     rubevy の `docs/worklog/2026-09-17-restart-burst.md` が「直す場所は VM」と結論した当の修正。
     0.5.0 では 10 本同時差し替えで VM が止まるので、それを下限にしない。
   * `sabiruby-compiler = "0.2.2"` — 0.5.x の VM を名指しする最初の compiler
     （sabiruby `docs/design/compiler.md` "Publishing"）。rubevy は `highlight()` を使わないので 0.2.3 は要らない。
   * `sabiruby-serde = "0.1.0"`（dev のみ）— 公開されている唯一の版。
   下限で本当に通るかは段階 P1 で確かめる（`cargo update -p … --precise`）。通らなければ
   **勝手に上げずに止まって報告**する。
2. **rubevy_games は版で依存し、`[patch.crates-io]` で git に向け直す。** games の直接の依存を
   rubevy と同じ版の書き方にし、workspace のルートに

   ```toml
   [patch.crates-io]
   sabiruby = { git = "https://github.com/sabiruby/sabiruby" }
   sabiruby-compiler = { git = "https://github.com/sabiruby/sabiruby" }
   sabiruby-serde = { git = "https://github.com/sabiruby/sabiruby" }
   ```

   を置く（`sabiruby-macros` が要るかは `cargo tree` で決める。使われない patch は警告になる）。
   こうすると (a) バイナリの中の `sabiruby` は 1 つのまま、(b) VM の修正を crates.io の公開を
   待たずに games で使える今の速さが残る、(c) `Cargo.lock` に git の rev の行が残るので
   `pages.yml` は 1 行も変えなくてよい。games が rubevy を引くのは git のまま
   （rubevy は毎日動いている。crates.io の 0.0.1 に games を縛らない）。
3. **rubevy の隣の checkout に向けるやり方**は、`Cargo.toml` のコメントを
   `[patch.crates-io]`（`.cargo/config.toml`、git-ignore 済み）に書き換える。
4. **版は 0.0.1 のまま出す。** 著者が付けた版で、まだ一度も出ていない。
5. **公開（`cargo publish`）、main への merge、push は本体が行う。** 実装担当は dry-run まで。

## 3. 段階

### P1 — rubevy（worktree `../rubevy-wt-publish`、ブランチ `publish`）

やること:

1. `Cargo.toml` の 4 つの依存を 2.1 の版にする（feature と optional はそのまま）。コメントを今の
   事実に書き換える（git である理由の段落は消し、隣の checkout に向ける方法を `[patch.crates-io]` で）。
2. `[package]` に crates.io が表示に使うものを足す: `repository = "https://github.com/sabiruby/rubevy"`、
   `readme = "README.md"`、`categories`（crates.io の一覧にある slug だけ。`game-development` は
   ある。ほかは一覧で確かめてから）。`rust-version` は測って決める（bevy 0.19.1 と sabiruby の
   `rust-version` の大きいほう。`cargo metadata` で読む）— 測れなければ書かない。
3. `cargo package --list` を読み、crate に要らない大きなものが入っていないか見る
   （今 85 ファイル。`docs/` 540K、`tests/` 176K、`assets/` 108K）。examples が `assets/` を
   読むなら assets は残す。`exclude` を足すなら何を外したかと圧縮後のサイズの前後を worklog に。
4. `README.md` に crates.io からの使い方（`cargo add rubevy`、feature `ruby-source`、
   SabiRuby の版との関係）を足す。今の README の書き方に合わせる。
5. `docs/README.md` の目次にこの計画書と worklog を足す。

確認:

* `cargo update` のあと `cargo test`（全部）— SabiRuby 0.5.2 / 0.2.3 で。通過数を報告。
* 下限で: `cargo update -p sabiruby --precise 0.5.1`、`-p sabiruby-compiler --precise 0.2.2` で
  `cargo test`。終わったら lock は最新に戻してコミット。
* `cargo test --features ruby-source`（今 CI は無いので手で）。
* `cargo tree -d | grep sabiruby` が空（`sabiruby` が 1 つ）。
* `tests/` の wasm 番人（`bcb9cc0` で入れた、src/ がブラウザに無い名前を使わないテスト）が通る。
* `cargo check --target wasm32-unknown-unknown`（feature なし。`ruby-source` は C が要るので対象外）。
* `cargo publish --dry-run` が verify まで通る。出力の `Packaged N files, … compressed` を報告。
* `cargo doc --no-deps` が警告 0（docs.rs で出るページなので）。

### P2 — rubevy_games（worktree `../rubevy_games-wt-publish`、ブランチ `rubevy-published`）

P1 のブランチを games から見るために、**コミットしない** `.cargo/config.toml` に
`[patch."https://github.com/sabiruby/rubevy"] rubevy = { path = "../rubevy-wt-publish" }` を置いて確かめる
（本体が rubevy を push したあと、本体が `cargo update -p rubevy` で lock を本物の rev にする）。

やること:

1. ルート `Cargo.toml` の `sabiruby`・`sabiruby-compiler`・`sabiruby-serde` を版にし、2.2 の
   `[patch.crates-io]` を足す。「同じ source でないと VM が 2 つ入る」のコメントを、patch で
   そろえている今の形の説明に書き換える。
2. `docs/README.md` の目次と worklog。

確認:

* `cargo tree -d | grep -E 'sabiruby|rubevy'` が空。`cargo tree -i sabiruby` の source が git の 1 つ。
* `grep -m1 -o 'github.com/sabiruby/sabiruby#[0-9a-f]*' Cargo.lock` が rev を返す（`pages.yml` 37 行目と同じ式）。
  rev が `7be7b86` より進む場合（sabiruby main の docs コミット `08d8e78`）はそれでよい。中身は同じ。
* `cargo test --workspace`。
* **wasm**: `web/build.sh garden` と `web/build.sh sabibots`（PATH に `~/.local/binaryen-version_132/bin`）、
  Playwright（`~/.cache/ms-playwright` の Chromium）で両ページ **pageerror 0** と **`?selftest`**
  （book `CLAUDE.md` の規則。やり方は `docs/web.md` と最近の worklog）。
* unsafe を増やさない（manifest だけの変更のはず。Rust のコードに触る必要が出たら止まって報告）。

### P3 — 本体

P1・P2 の報告をレビュー → rubevy を main に merge・push → `cargo publish -p rubevy` →
タグ `v0.0.1` → games の lock を `cargo update -p rubevy` で本物の rev に → games を merge・push →
Pages の CI を見る → 両 repo の docs の「状況」とこの計画書の下の表を更新。

## 4. やらないこと

* `rubevy-arena` を出さない。games を crates.io の rubevy に切り替えない。
* rubevy の API、テスト、example の中身を変えない。版を上げない。
* sabiruby に触らない。

## 5. 状況

| 段階 | |
|---|---|
| P1 rubevy の依存と package | **済み**（2026-09-18、`6fd46dd` + README 1 行 `bbe0484`、merge `2d9eb92`）。下限の版（0.5.1 / 0.2.2）でも最新（0.5.2 / 0.2.3）でも 75 passed。`rust-version` は bevy 0.19.1 の 1.95.0。記録は `docs/worklog/2026-09-18-crates-io.md` |
| P2 rubevy_games の依存と patch | **済み**（2026-09-18、games `bac6dc1`、merge `62a7cbf`）。games の compiler の下限だけ 0.2.3（エディタが `highlight()` を呼ぶ。§2.1 の 0.2.2 は rubevy の数）。`sabiruby-macros` は patch に要らなかった。wasm 5 走行で pageerror 0・FAIL 0。記録は games `docs/worklog/2026-09-18-rubevy-published.md` |
| P3 公開 | **済み**（2026-09-18）。`rubevy` 0.0.1 を crates.io に公開（`Packaged 87 files, 933.5KiB (296.2KiB compressed)`）、タグ `v0.0.1`（`2d9eb92`）。games の lock は rubevy `2d9eb92` |

残ったもの: `cargo test --features ruby-source` の最初の 1 回だけ `tests/child_task.rs` が 1 本落ち、
再実行 4 回では再現しなかった（C のビルドと並走中。どのテスト名かは取れていない）。
CHANGELOG はまだ無い — 0.0.2 を出すときに作る。
