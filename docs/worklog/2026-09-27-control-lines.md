# 2026-09-27 control 層の handler の例外を、作者の行で出す

0.2.0 の公開の後に、ある組み込み先が control 層（`rubevy::layers::CONTROL`）に乗り換えて見つけた不具合
（`docs/backlog.md` に 1 行で残っていたもの）。作業場所は `/home/kishima/book/kishima/rubevy`、
ブランチ `control-lines`、着手時の先頭は main の `b943c24`。補助はスクラッチパッドの `ctl/`。

## 1. 何が起きていたか

`src/layers/control.rb` の `Rubevy::Control#on` は、handler（`on(:event) { … }` のブロック）が
例外を上げると、それを捕まえて `Rubevy.log` に出し、次のメッセージを待ち続ける。出す場所は
`e.backtrace[0]`、つまり VM が付けた行そのものだった。ゲームが作者のファイルの前に前置き
（`Program::prelude_lines` の行）を付けていると、この行は前置き込みのプログラム全体の行になる。

一方 `ScriptEnded::at` と `ScriptStats` は 0.2.0 で、前置きをスクリプト自身のファイルからだけ引く
ようになっている（`authors_places`、`src/lib.rs`。`require` したファイルの行はそのまま。
`worklog/2026-09-26-release-0.2-a.md` §8）。同じ作者に同じ誤りを見せる 2 つの口が、違う行を言っていた。

直す前の層で、下の回帰テストを走らせた出力（前置き 8 行、作者のファイルの 2 行目で `raise`）:

```
left:  ["on(:n): RuntimeError: in the prelude (player.rb:5:in prelude_raises)", "on(:n): RuntimeError: odd (player.rb:10)"]
right: ["on(:n): RuntimeError: in the prelude (player.rb:5)", "on(:n): RuntimeError: odd (player.rb:2)"]
```

`player.rb:10` は 2 + 8。前置きが定義したメソッドの中の `raise` は、前置きの中の行
（`player.rb:5`、作者のファイルの 5 行目ではない）をそのまま指していた。

## 2. どこで直すか — 規則は 1 か所

案は 3 つあった。

1. **Ruby の側で行を直す**（層が `e.backtrace` を読み、`file:line` を割って前置きの行数を引く）。
   規則（「スクリプト自身のファイルだけ引く」「0 の行と前置きの中の行は飛ばして外へ」「`require`
   したファイルはそのまま」）を Ruby にもう 1 つ書くことになる。2026-09-26 まで Rust の側でさえ
   「どのファイルでも引く」誤りがあった規則なので、2 つ目の写しは同じ誤りの置き場所になる。
2. **ホストが前置きの行数を渡すだけ**にして、引き算は Ruby。1 と同じく規則が 2 か所になる。
3. **既存の `authors_places` を Ruby から呼べる形にする**。規則は Rust の 1 か所のまま。

原則（1 か所の規則を使い回す）で 3 にした。足したものは次のとおり。

* `authors_places_in(frames, own, prelude_lines)`（`src/lib.rs`）: `authors_places` の中身を、
  「スクリプト自身のファイル」を引数で受け取る形に出したもの。`authors_places` は外側のフレームの
  ファイルを `own` にしてこれを呼ぶだけになった。規則の本文は 1 つ。
* `backtrace_places(vm, ary)`: `Exception#backtrace` や `caller` の文字列の配列を場所の列にする。
  `authors_line`（`ScriptEnded::at`）が持っていた読み方をそのまま外に出したもの。
* `Rubevy.__authors_place(backtrace, origin, prelude_lines)`: 上の 2 つを Ruby から呼ぶネイティブ。
  `"file:line"` か nil を返す。ホスト API ではなく層のための口なので `__` を付けた
  （`__pop_try` と同じ綴りの慣習）。
* `PRELUDE_LINES_IVAR`（`@rubevy_prelude_lines`）: タスクを始めるところ（`start_scripts`）で、
  `ENTITY_IVAR` の隣に `Script::prelude_lines` を載せる。`src/prelude.rb` の `Task.new` が、
  エンティティと同じく子タスクに写す（`on` をスクリプトの子タスクから呼んでも前置きの長さが分かる）。
  `docs/numbers.md` §4 に 1 行足した。

### 2.1 なぜ `own` を引数にしたか

`authors_places` は「一番外側のフレームのファイルがスクリプト自身のファイル」と読む。スクリプトの
タスクはトップレベルから始まるのでそれで正しい。ところが handler は層が `Task.new` で作ったタスクで
走るので、その例外の一番外側のフレームは層（`control.rb`）の側になる。だから handler の例外だけを
見ても、どれがスクリプトのファイルか分からない。

そこで層は、`on` が呼ばれた時点（スクリプトのタスクの中）のバックトレースを取っておき、その一番外側の
フレームのファイルを `own` にする。「一番外側のフレームのファイル」という読み方も、`authors_places`
と同じ。

### 2.2 `caller` ではなく `caller(0)` — 捨てた試み

最初は `origin = caller` と書いた。テストは片方が通り、片方が落ちた。ネイティブに入る値を
一時の `eprintln!` で見ると、`origin` が空の配列だった:

```
DBG places=[("player.rb", 10)] origin=[] …
DBG places=[("player.rb", 5), ("player.rb", 13)] origin=[] …
```

`control.rb` は `tools/compile_scripts.sh` が `mrbc` に `-g` を付けずに作るので行の表が無く、その
フレームはバックトレースの文字列に出てこない（`Vm::backtrace_text`）。`caller` は本家の算術
（SabiRuby `src/builtins/ext_kernel.rs:365` の `caller`）で「先頭は `caller` 自身、次は呼んだメソッド」
を落とす前提で切るので、出てこないフレームの分だけ余計に切り、スクリプトのフレームまで落として `[]`
を返していた。`caller(0)` なら切るのは先頭の 1 つだけで、`["player.rb:9"]` が返った。

片方が通っていたのは偶然で、`own` が無い（`None`）ので何も引かれず、前置きの中の行
`player.rb:5` がたまたま期待した作者の行 5 と同じ数だったため。`caller(0)` にしてからは、前置きの
中のフレーム（5 ≤ 8）を飛ばし、作者の行（13 − 8 = 5）を出している。層を `-g` 付きで作っても、
`caller(0)` の一番外側はスクリプトのトップレベルのままなので、この読み方は変わらない。

## 3. ログの形の変化

前は `(file:line:in method)` と `e.backtrace[0]` をそのまま出していた。今は `(file:line)` で、
`ScriptEnded::at` と同じく場所だけ。作者の行が無いとき（行の表の無いプログラム、全部のフレームが
前置きの中）は括弧ごと出さない。これも `ScriptEnded::at` が `None` になるときと同じ。

## 4. 回帰テスト（`tests/control_layer.rs`）

ゲームの前置きで `Rubevy.log` を「問いとして返す」ものに差し替え（層は `Rubevy.log` を名前で呼ぶ
ので、これに届く）、テストが層のログを読めるようにした。前置きは 8 行（`Program::new` が付ける
見出しの行を含む）。

* `a_handler_error_is_logged_at_the_authors_line`: 作者のファイルの 2 行目の `raise` が
  `player.rb:2`、前置きが定義したメソッドの中の `raise` が、それを呼んだ作者の行 `player.rb:5`。
* `a_handler_error_in_a_required_file_keeps_its_line`: `require` した `helper.rb` の 12 行目
  （前置きの長さより後ろの行。引かれていれば 4 と出て分かる）の `raise` が `helper.rb:12`。
  helper は `tests/script_ended_at.rs` と同じ作り（`EmbeddedHost` に行の表付きの `.mrb`）。

## 5. 確認

`.rb` を変えたので、`src/prelude.mrb` と `src/layers/control.mrb` は `tools/compile_scripts.sh` と同じ
`docker run --rm -v … kishima/mruby:4.1.0-rc mrbc` で、その 2 つだけ作り直した（ほかの `.mrb` には
触っていない）。

| 確認 | 結果 |
|---|---|
| `cargo test --workspace` | 249 passed / 0 failed / 6 ignored（doctest を含む） |
| `cargo test --workspace --features pointer` | 252 passed / 0 failed / 6 ignored |
| `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps`（`--features pointer` 付きも） | 通る |
| `cargo build --lib --target wasm32-unknown-unknown`（`--features pointer` 付きも。`touch` してから建て直した） | 通る |
| games の箱庭、ブラウザ: `web/build.sh garden` + Playwright（Chromium 1243、swiftshader）`?selftest` | `selftest: ok` 47 行と `done` の 1 行（0.2.0 のときの 48 行と一致）、pageerror 0・requestfailed 0。404 は `favicon.ico` の 1 件で、0.2.0 のときの記録（スクラッチパッドの `rel02/pw.log`）にも同じ 1 件がある |

games の確認は、games の作業ツリー（別の担当が `post-02` で作業中）には触らず、`git archive main` の
写しをスクラッチパッドの `ctl/games` に展開し、その `Cargo.toml` の末尾に
`[patch."https://github.com/sabiruby/rubevy"]` で手元の rubevy（と `rubevy-build`）を向けて建てた。
`CARGO_TARGET_DIR` も写し専用（`ctl/games-target`）。写しは games の main `1d0fadd` のもの。

## 6. 気づいた点

1. **層の `.mrb` に行の表が無い。** `tools/compile_scripts.sh` は `mrbc` に `-g` を付けないので、
   `src/prelude.rb` と `src/layers/*.rb` の中で上がった例外や `caller` には、層のフレームが出ない。
   今回それで `caller` が空になった（§2.2）。作者に見せる行は作者のファイルの行なので困らないが、
   層そのものの誤りを追うときは手がかりが減る。rubevy（道具）の話。直していない。
2. **`caller` の算術は、行の表の無いフレームが混ざると呼んだ側のフレームまで落とす。** SabiRuby
   `src/builtins/ext_kernel.rs:365`。本家 mruby と同じ算術なので、本家でも同じになるかは確かめて
   いない。VM（SabiRuby）の話か本の素材。
