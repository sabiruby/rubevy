# 2026-09-20 ゲームが 2 回書いていた口を crate に上げる（R6）

計画 `docs/plans/generalize-plan.md` の段階 R6（3.6）。到達点は 5 つ —
(1) prelude つきプログラムを組んで「著者の 1 行目が何行目か」を返す口、(2) コンパイラのエラー文の行番号から
prelude ぶんを引く補正、(3) スクリプトの差し替えの口、(4) 埋め込みの表から読む `Host`、
(5) `.rb` を埋め込む build ヘルパ。**5 つとも入った。** 計測は無い段階なので数字は取っていない。

元になったのは rubevy_games の 2 本（garden と sabibots）が同じ形で持っていたコードで、
`rubevy_games/docs/worklog/2026-09-20-factory-survey.md` の §3・§5 が「3 本目に 3 回目を書かせない」ために
上げる先を rubevy 本体と指定している。**読むだけ**の約束なので rubevy_games には 1 行も触っていない。

## 何を読んで、何を移したか

移す元（読んだ場所）:

| 元 | 何 |
|---|---|
| `rubevy_games/garden/src/main.rs:2727-2740` | `compile_source`: `{prelude}\n# ---- {name} ----\n{body}\n{tail}\n` を組み、`prelude.lines().count() + 2` を返す |
| 同 `:2742-2795`（`in_the_authors_lines` は `:2761`、`one_diagnostic` は `:2768`） | `in_the_authors_lines` / `one_diagnostic`: `FILE:LINE:COL:` の行番号から prelude ぶんを引く |
| 同 `:6190-6228` | そのテスト 7 ケース |
| `rubevy_games/sabibots/src/main.rs:1630-1645` | `compile_text`: 同じものの別の書き方（`prelude_file` と `start` を引数で取る） |
| 同 `:1648-1654` | `restart`: `ScriptTask` / `ScriptDone` を外して `Script` を挿す 3 行 |
| `rubevy_games/{garden,sabibots}/build.rs` | 35 行、`diff` が 1 行も出さない（確かめた） |
| `rubevy_games/garden/src/platform.rs` の wasm 側 `read` | 埋め込み表 `RUBY_FILES` を `find(|(p, _)| *p == want)` で引く |

新しく書いたのは `src/source.rs`（(1)(2)）、`src/embed.rs`（(4)）、`src/lib.rs` の `replace_script`（(3)）、
`rubevy-build/`（(5)）。**ゲームの語彙（creature / robot / garden / 工場）は `src/` にも公開 API にも入れていない。**
テストの中の名前も `script.rb` / `brain.rb` / `helper.rb` に置き換えた。

## (1)(2) prelude つきプログラムと行番号の補正

### `prelude_lines` の数え方を変えた（garden の式は前提が 1 つある）

garden も sabibots も `prelude.lines().count() as u32 + 2` と書いている。この `+2` は
「prelude の後ろに空行が 1 行できて、その次が `# ---- name ----` の行」という数え方で、
**prelude の文字列が改行で終わっている**ときだけ正しい。ファイルから読めばそうなるので 2 本とも合っているが、
ソースに直接書いた prelude（テストがまさにそれ）では 1 ずれる。

`Program::new` は組み立てた前置きそのものを数えることにした（`src/source.rs`）:

```rust
let head = format!("{prelude}\n# ---- {name} ----\n");
let prelude_lines = head.lines().count() as u32;
```

`str::lines` は末尾の改行の後ろを 1 行と数えず、改行で終わらない最後の行は 1 行と数える。つまり
改行で終わる prelude では `N + 2`（garden の式と同じ数）、終わらない prelude では `N + 1` になり、
どちらでも「著者の 1 行目 = `prelude_lines + 1`」が成り立つ。`tests/source.rs` の
`the_authors_first_line_is_where_the_program_says_it_is` が 2 通りの prelude で実際に本文の行位置を
探して突き合わせている。

この数は `VmInspector::fill`（`rubevy_games/crates/rubevy-arena/src/inspect.rs:242,305-307`）が取る
`prelude_lines` と同じ意味であることも確かめた。あちらも `if line > prelude_lines { line - prelude_lines }` で、
境界の扱い（prelude の最終行は prelude 側）まで一致している。

### エラー文の形式を両方で確かめた

計画書の指示どおり、`sabiruby-compiler` が実際に出す形とブラウザの橋が出す形の両方を見た。

- **ネイティブ**: `sabiruby_compiler::Diagnostic` の `Display` が `"{filename}:{line}:{column}: {message}"`
  （`sabiruby-compiler-0.2.2/src/lib.rs:74-79`、rustdoc に「as `mrbc` prints it」）。
  `CompileError` の `Display` はエラーの診断だけを 1 行に 1 つ並べ、エラーが 1 つも無ければ `"compile error"`
  （同 `:87-99`）。
- **ブラウザ**: `sabiruby-playground/wasm/src/lib.rs:139-147` の `sabi_compile` が
  `sabiruby_compiler::compile` を呼び、失敗したら **同じ `CompileError` の `Display`** を `sabi_take_text` に置く。
  違うのは `Options.filename` が定数 `FILENAME = "playground.rb"`（同 `:21`）に固定されていることだけ
  （橋はソースしか受け取らない。`rubevy_games/docs/web.md:115-121` が同じことを書いている）。

実物で確かめた出力（`tests/source.rs` の 2 本が同じことをしている。ここは作業中に `--nocapture` で出したもの）:

```
prelude_lines = 5
--- raw (script.rb) ---
script.rb:7:7: syntax error, unexpected '<'; expected an expression after the operator
--- moved ---
script.rb: script.rb:2:7: syntax error, unexpected '<'; expected an expression after the operator
--- raw (playground.rb) ---
playground.rb:7:7: syntax error, unexpected '<'; expected an expression after the operator
--- moved ---
script.rb: playground.rb:2:7: syntax error, unexpected '<'; expected an expression after the operator
```

つまり**形式は同じで、違うのはファイル名だけ**。`one_diagnostic` はファイル名を見ずに「行の中で最初に現れる
`:数字:数字:`」を探すので、`playground.rb` でも素通りする。テスト `the_browser_bridges_message_moves_too` は
ブラウザを走らせてはいない（走らせられない）ので、何を確かめていないかをテストの rustdoc に書いた。

### 「末尾より後ろ」のケース

garden の実装には末尾（`tail`）の行のための分岐が無い。読んでみると分岐が要らないからで、
`tail` の行は「著者のファイルの最終行の次の行」として素直に報告される — `def` を閉じ忘れた
`syntax error, unexpected end-of-input` がまさにそこを指す。これを消したり最終行に丸めたりすると、
かえって「どこで終わっていないのか」が消える。garden がそう書いていた理由をそのまま採り、
`an_error_past_the_authors_last_line_is_the_line_after_it` で固定した。

## (3) 差し替え

`sabibots/src/main.rs:1648-1654` の 3 行を `replace_script(&mut Commands, Entity, Script<M>)` にした。
形は「`Commands` の拡張トレイトか自由関数か」を計画が既存の流儀に合わせろと言っているので、
repo 全体を見て拡張トレイトが 1 つも無いこと（`trait` の定義は `src/` に 0 件）を確かめ、自由関数にした。
引数を `(name, handle)` ではなく組み立て済みの `Script<M>` にしたのは、優先度や名前の付け方を
ほかの場所と同じ書き方にするため、そして VM の名前タグを `Script` から推論させるため
（`replace_script(&mut c, e, Script::<Mods>::for_vm(h))` が `Mods` の VM の差し替えになる）。

`ScriptDone` を外すのを忘れると「一度終わったスクリプトには二度と別のを載せられない」という、
見た目でわからないバグになる。これは rustdoc に書いた。

## (4) 埋め込みの `Host`

### コンパイルの渡し方を 1 つに決められた

計画は「形が 2 通り以上ありえて決めきれなければ (4) だけ止めて報告」と言っている。止めずに進めた理由:
sabiruby 側を読むと、`require` は **`Host` にしか訊かない**（`sabiruby-0.5.2/src/builtins/ext_require.rs:32-62`
の `__file_exist?` / `__load_file`、`:66-102` の `__exec_file`）。しかも `__exec_file` は読んだバイト列が
`RITE` で始まれば**そのまま走らせ**、そうでなければ `host.compile(&src, &opts)` を呼ぶ（同 `:74-97`）。
つまり「外から渡すコンパイル関数」の形は `Host::compile` そのもの以外にありえない — 呼ぶのがそれだからで、
`FnMut(&[u8], &EvalOptions) -> Result<Vec<u8>, String>` に決めた。

捨てた案: `Fn(&str, &str) -> Result<Vec<u8>, String>`（ソースとファイル名だけ。ブラウザの橋に合う）。
これだと `eval` の `line` と `scopes` を渡す道が閉じる。橋がそれを使えないのは橋の都合なので、
**渡せるものは渡して、使えない側が捨てる**方にした（そう書けることを `tests/embedded_host.rs` の
`a_compiler()` が実演している。あれは `opts.filename` と `opts.debug_info` だけ使い、`scopes` を捨てている）。

### 表を 2 枚にした

`require "helper"` は `.mrb` → `.rb` の順に試す（`sabiruby-0.5.2/src/mrblib/require.rb:73-79`）。
なので**バイトの表を先に引く**ようにすると、コンパイラの無いビルドでも `.mrb` だけ置けば `require` が通る。
`tests/embedded_host.rs` の `a_compiled_file_needs_no_compiler` がそれで、素材はこの crate に既にある
`assets/scripts/helper.mrb`（`tools/compile_scripts.sh` が reference mrbc で作ったもの）を `include_bytes!` した。
テストが自分で作った bytecode ではない。

`./` で始まる名前は `$LOAD_PATH` を通らず書かれたまま host に来る（`require.rb:62-68`）ので、
先頭の `./` だけ落としてから引くことにした。落とさないと `require "./ruby/helper.rb"` が
表にある `"ruby/helper.rb"` に当たらない。テスト `a_path_that_says_here_is_the_same_file` がこれを押さえている
（実装から `strip_prefix("./")` を外すとこのテストは LoadError で落ちる）。

### `FileHost` と口を揃えた

`FileHost::compile` の中身（`ruby-source` の有無で分かれる 2 つの `#[cfg]`）を `reference_compile` という
`pub(crate)` の関数に出し、`FileHost` と `EmbeddedHost` の両方がそれを呼ぶようにした。
これで「feature の無いビルドでの断り文」が 2 か所に書かれることがなくなる。
`tests/embedded_host.rs` の `source_without_a_compiler_says_what_is_missing`（`ruby-source` が**無い**ときだけ走る）が
その文面に `ruby-source` が入っていることを確かめる。

## (5) build ヘルパと workspace の影響

`rubevy-build/` を作った。std だけ・依存 0。`Embed::new("ruby").write()` が
`OUT_DIR/ruby_files.rs` に `pub static RUBY_FILES: &[(&str, &str)]` を書く。
静的の名前・出力ファイル名・拡張子・text か bytes か（`include_bytes!`）は全部変えられる。
既定値は「2 本のゲームの build.rs がそう書いていた」ことが出どころで、そう rustdoc に書いた
（S2 でゲーム側が `include!` の行を変えずに済む、という意味もある）。

計画が確かめろと言っている **workspace 化の影響**を実際に測った（`Cargo.toml` に
`[workspace] members = [".", "rubevy-build"]` を足した状態と、足す前の比較）:

| 見たもの | 結果 |
|---|---|
| repo の根の `cargo test` | 変わらない。root が package の workspace は既定で root だけを対象にするので、`[workspace]` を足す前と同じ 21 バイナリ（lib の unittest + `tests/` の 20 本）がそのまま対象になる。ヘルパのテストは `cargo test --workspace` か `-p rubevy-build` で走る |
| `Cargo.lock` | 4 行増えるだけ（`[[package]] name = "rubevy-build" version = "0.0.1"`、依存は無いので中身も無い）。ほかの行は 1 つも動かない（`diff` で確認） |
| `cargo publish`（`cargo package --list --allow-dirty` で代用） | **97 ファイルのまま変わらない。** 入れ子の package は `.crate` に入らない。警告も出ない |
| CI | この repo に `.github/` は無い（確かめた）。影響なし |
| `cargo build --target wasm32-unknown-unknown --lib` | 変わらない（ヘルパは build-dependency で、ターゲットに出ていかない） |

影響が小さいので止めずに進めた。workspace にしなくても（根に `[workspace]` を書かずに）
`cargo test` も `cargo package` も同じ結果になることは先に確かめてあるが、その場合
`rubevy-build/Cargo.lock` が別にでき、根の `cargo test --workspace` でヘルパのテストが回らない。
lock が 1 つで済む方を採った。

## 確認

`CARGO_TARGET_DIR=…/rubevy/target`（この repo で cargo を動かすのは自分だけ、という取り決め）。
**計測はしていない。**

- `cargo test --workspace`: **101 passed、0 failed、2 ignored**（`tests/read_cost.rs` の 2 件は元から ignore）。
  着手前の baseline は 81 passed。増えた 20 は `tests/source.rs` 6・`tests/embedded_host.rs` 6・
  `tests/replace.rs` +1・`rubevy-build` の単体 4・その doctest 1・rubevy の doctest +2。
- `cargo clippy --workspace --all-targets`: **新しい警告 0**。残る 2 件（`src/lib.rs` の `collapsible_if`、
  `examples/headless.rs` の `type_complexity`）は着手前から出ているもので、`git stash -u` した状態で
  同じ 2 件が出ることを確かめてある。
- `cargo build --target wasm32-unknown-unknown --lib`: 通る。
- `cargo test --test no_wasm_unsupported`: 通る（新しい `src/source.rs`・`src/embed.rs` は
  `std::fs` も時計もスレッドも呼んでいない — それがこの段階の目的でもある）。
- `cargo build --lib --features ruby-source` と、その feature でのテスト: 通る。
- `cargo doc --workspace --no-deps`: 警告 0。
- `cargo fmt` は走らせていない（repo に rustfmt の設定が無く、無関係な差分が出る。R0 の報告の 5 番）。

### 確かめていないこと

**build.rs → `include!` → `EmbeddedHost` の通しは、この repo では通していない。**
build script は自分の crate のビルド中にしか動かないので、rubevy 自身に `build.rs` を足さない限り
end-to-end のテストは書けない（足すのは公開する crate のビルドを変えることなので採らなかった）。
確かめたのは 2 つに割った両端: ヘルパが書く**文字列**が `pub static NAME: &[(&str, &str)] = &[…];` の形であること
（`rubevy-build` の `the_table_is_what_the_other_side_includes`）と、その形の表を `EmbeddedHost` に渡すと
`require` が通ること（`tests/embedded_host.rs`）。通しは rubevy_games の S2 が最初の客になる。

**ブラウザでの動作も確かめていない。** 共通 CLAUDE.md の `web/build.sh` + Playwright の確認は、
計画書 4 章の注記どおり games が取り込む段階（`shared-crate-plan.md` S2）で行う。
この段階で `src/` に足したものは std の時計・ファイル・スレッドを 1 つも呼んでいない。

## 気づいた点

1. **`sabibots` の行番号のずれは、この段階では直っていない。** 何を: `sabibots/src/main.rs:1630-1645` の
   `compile_text` は `prelude_lines` を返すが、エラー文をそのまま `error!` に流しているので、
   エディタに出る番号が prelude ぶん（実測 295 行）ずれたままである。どこで: rubevy_games。
   なぜ気になるか: 直す部品はこの段階で rubevy に入ったので、あとは呼ぶだけになった。
   どこに属するか: そのゲーム固有（S2 の作業）。

2. **`Program::new` の引数が `&str` 4 本で、取り違えても型が通る。** 何を: `Program::new(prelude, name, body, tail)`。
   どこで: `src/source.rs`。なぜ気になるか: `name` と `body` を逆に渡すと、コンパイルは通って
   「本文が 1 行のプログラム」ができる。ビルダ（`Program::with_prelude(p).named(n).tail(t).build(body)`）なら
   起きないが、型が 2 つに増える。2 本のゲームの呼び出し側がちょうどこの 4 つを持っていたので
   今回は素直な形にした。どこに属するか: rubevy（API の使いにくさ）。S2 で実際に呼んでみて
   気持ち悪ければ、ビルダを**足す**（既存は消さない）のがよいと思う。

3. **`EmbeddedHost` を入れると `set_load_path` を書き直す必要があるのに、それが型に現れない。** 何を:
   `RubevyPlugin::build` が `{root}/scripts` と `{root}` を `$LOAD_PATH` に入れる（`src/lib.rs:1572`）。
   埋め込みの表の鍵は普通 `ruby/...` なので、`set_host` だけして `set_load_path` を忘れると
   `require` が静かに LoadError になる。どこで: `src/lib.rs`。なぜ気になるか: 忘れやすく、
   ブラウザでだけ起きる。`EmbeddedHost` を渡すと load path も一緒に決まる口（`ScriptWorld::embed(host, paths)` のような）が
   あってもよいが、公開 API を増やす判断は著者のもの。どこに属するか: rubevy（口の設計）。

4. **`assets/scripts/*.mrb` は Docker の reference mrbc で作られている**（`tools/compile_scripts.sh`）。
   何を: `tests/embedded_host.rs` が `assets/scripts/helper.mrb` を `include_bytes!` するようになった。
   なぜ気になるか: これで **テストが生成物に依存する**。`helper.rb` を変えて `.mrb` を作り直し忘れると、
   このテストは古いバイトコードを見て通り続ける（今回は `helper.rb` に触っていない）。
   どこに属するか: repo の作法（生成物とテストの関係）。今は害が無いので直していない。

5. **`rubevy-build` の既定値 3 つ（`RUBY_FILES`・`ruby_files.rs`・`.rb`）の出どころは「2 本のゲームがそう書いていた」以上のものではない。**
   なぜ気になるか: R10 が作る `docs/numbers.md` は数の一覧だが、この 3 つは数ではなく名前なので
   一覧に入るのかどうかが決まっていない。どこに属するか: 計画書（R10 の範囲）。
   全部引数で変えられるようにはしてあり、出どころは rustdoc に 1 行書いた。
