# 2026-09-18 ブラウザに無い std をソースの側で止める — `tests/no_wasm_unsupported.rs`

## なぜ今日これを書くか

同じ日の前半の記録 `docs/worklog/2026-09-18-wasm-instant.md` が、公開版の庭が真っ黒に
なった 1 行を追い切って直し、最後をこう終えている — 同じ形の間違いを止める道は 2 つあって、
**src/ の grep** か、**wasm でヘッドレスに 1 フレーム回す仕掛け**か、どちらを採るかは著者の判断
待ち、と。今日その判断が来て、前者になった。この記録はそれを書いた作業のものである。

前者を選ぶ理由は「安いから」だけではない。あの panic は `cargo build --target
wasm32-unknown-unknown` を**素通りした**（前半の記録のとおり、バグのあるコードでビルドが
通っている）。つまり CI に wasm ビルドを足しても捕まらない。捕まえるには、ビルドではなく
**書かれている字**を見るか、本当に走らせるかしかない。後者は本物を測るが、ブラウザと
Playwright と wasm ビルドが要り、rubevy 単体の `cargo test` には入らない。前者は
`cargo test` の中に住めて、ローカルでも CI でも黙って回る。今日の 1 件は確実に止まる。

## テストの形

`tests/no_wasm_unsupported.rs` は 2 つのテストを持つ。

* `src_names_nothing_a_browser_lacks` — `src/**/*.rs` を全部読み、禁じた std の名前が出る行を
  集めて、1 つでもあれば `ファイル:行: 内容` と理由を並べて落ちる。
* `the_guard_catches_what_it_is_for` — 判定そのものを、`src/` には決して現れない例文に当てる。
  落ちないテストは何も言っていないのと同じで、このガードは「誰かが悪い行を書いた日」にしか
  発火しない。その日が来る前に、判定が生きていることを言えるようにしておく。

禁じたのは指示のとおり `std::time::Instant` / `std::time::SystemTime` / 裸の `Instant::now()` /
`std::thread` / `std::fs` / `std::net` / `std::process`。`std::time::Duration` は禁じていない
（時計ではなく算術で、ブラウザにもある）。`src/lib.rs:717` の `frame_time: Option<std::time::Duration>`
はそのまま通る。

### 裸の `Instant::now()` を use 文で見分ける

`bevy::platform::time::Instant`（ブラウザでは `web-time`）は許して、`std::time::Instant` は
禁じる、という区別を行単位でどう付けるか。`src/lib.rs:1234` は

```rust
ORIGIN.get_or_init(bevy::platform::time::Instant::now).elapsed().as_nanos() as u64
```

とフルパスで書いてあるので、`Instant::now` の直前が `bevy::platform::time::` かどうかを見れば
通せる。問題は `use bevy::platform::time::Instant;` と書いて `Instant::now()` と呼ぶ書き方で、
行だけ見れば `std` のそれと字が同じである。そこでファイル単位で use 文を見て、
`bevy::platform::time::Instant` を名前で取り込んでいるファイルの中でだけ裸の `Instant::now()`
を許すことにした。取り込んでいないファイルの裸の `Instant::now()` は「どちらの `Instant` か
書いていない」として落ちる。**use 文の形で区別する**というのはこの意味である。

そのために use 文を平坦化する必要が出た。生の grep は `use std::time::{Duration, Instant};` を
拾えない — 字面に `std::time::Instant` が無いからである。`flatten_use` は
`use a::{b, c::{d, e}};` を `a::b` / `a::c::d` / `a::c::e` に開く（入れ子も、深さを数えながら
トップレベルのコンマだけで割って再帰する）。禁止語の照合はこの開いた文字列に対して行う。
`use std::{fs, io::Write};` も `use std::{fmt, time::{Duration, SystemTime}};` も、これで当たる。
どちらも `the_guard_catches_what_it_is_for` の中に例として入れてある。

### コメント行は飛ばす

`src/lib.rs:1678` は、その修正が残した rustdoc である。

```rust
//! which is `web-time` in a browser. It is not `std::time::Instant`: that one *panics* on
```

**何が起きたかを説明している文が、説明している当の名前を含んでいる。** これを落とすガードは、
説明を消させる方向に働く。コメントは何も呼ばないのだから、行頭が `//` / `/*` / `*` の行は
丸ごと飛ばすことにした。逆に、コードの行に付いた末尾コメントは飛ばさない（そこには
コードがある）。

### 例外はその行のコメントだけ、cfg は読まない

抜け道は `// wasm:` に続けて理由を書いた**その行**のコメント 1 つだけ。理由が空なら例外に
ならない（`// wasm:` だけの行は素通りしない）。

`#[cfg(not(target_arch = "wasm32"))]` の中も同じ扱いにした。cfg を解析すれば「これは wasm に
入らないから安全」と機械的に言えそうに見えるが、このガードは行を読む道具であって Rust の
条件コンパイルを解く道具ではない。属性がファイルのどこか上のほうに付いているからといって、
その関数がブラウザ側の道から呼ばれないことの証明にはならない。**理由を字で書かせる**ほうが、
半端な静的解析より正直である。

## `src/` に当たった 1 行

書いてすぐ回したら、1 行だけ当たった。

```
src/lib.rs:1307: std::fs::read(path).ok()
    → A browser has no filesystem. An asset comes through bevy's `AssetServer`.
```

`FileHost::read_file`、スクリプトの `require` がファイルを読むところである（`src/lib.rs:1287`
の `struct FileHost`）。これは直すものではなく、ネイティブでだけ意味のあるものだと判断した。
ブラウザにはファイルシステムが無く、そこで `require` がディスクから読めないのは仕様であって
間違いではない。だから例外コメントを付けた。

```rust
    fn read_file(&mut self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(path).ok() // wasm: native-only — a browser has no filesystem to read from
    }
```

`FileHost` の rustdoc にも 1 行足した（「OS に見えないファイルはスクリプトにも見えない」の
続きとして、ファイルシステムそのものが無いブラウザには何も出せない、と）。

コメントに**書かなかったこと**がある。`std::fs::read` が wasm32-unknown-unknown で
`Err` を返すのか panic するのか、である。std の `unsupported` バックエンドはエラーを返すはず
だが、この機械には `rust-src` が入っておらず（`rustup component` の src は未インストール、
`$(rustc --print sysroot)/lib/rustlib/src` は無い）、wasm を走らせる道具もこの作業には
持ち込まなかった。**確かめていないことは書かない**ので、コメントは「ブラウザにファイル
システムが無い」という確実なほうだけを言っている。時計のほうは前半にブラウザの console に
実文が出ている（`time not implemented on this platform`）から、テストの冒頭では panic と
書いてよい。

### 当たらなかったが気づいたこと

すぐ下の `file_exists` は `std::path::Path::new(path).is_file()` で、これは禁止語の一覧に
無いので当たらない。`is_file` は中で OS に訊くから、性質としては `std::fs` の仲間である。
`std::path` ごと禁じることは**しなかった**: `Path` と `PathBuf` の大半は文字列の操作で、
禁じるとブラウザでも正しい普通のコードまで止まる。名前で見ている道具の穴として、ここに
書き残しておく（塞ぐなら `Path::is_file` / `Path::exists` / `Path::metadata` のようにメソッド名を
足す形になる）。今の `src/` で OS に触る `std::path` はこの 1 か所だけである。

`src/` の `std::` の使用はほかに `mem::take`、`sync::{Arc, Mutex, OnceLock, atomic}`、`fmt`、
`collections::HashMap`、`marker::PhantomData`、`hash`、`io::Error` で、どれもブラウザにある。

## テストが本当に効くことの確かめ

ガードを書いた本人が「効きます」と言っても仕方がないので、黒画面の 1 行をそのまま戻してみた。
`clock_ns`（`src/lib.rs:1232`）の頭に一時的に

```rust
    let _probe = std::time::Instant::now(); // TEMPORARY: proving the guard bites
```

を入れる。テストは落ちる。

```
test the_guard_catches_what_it_is_for ... ok
test src_names_nothing_a_browser_lacks ... FAILED

src/lib.rs:1233: let _probe = std::time::Instant::now(); // TEMPORARY: proving the guard bites
    → std has no clock on wasm32-unknown-unknown: `Instant::now()` panics there. Use
      `bevy::platform::time::Instant` (or rubevy's `clock_ns`). A bare `Instant::now()` does not
      say which `Instant` it is. Import `bevy::platform::time::Instant` (the browser has that
      one) or write the path out.
```

**同じ状態のまま** wasm を組むと、通ってしまう。

```
$ cargo build --target wasm32-unknown-unknown
   Compiling rubevy v0.0.1 (/home/kishima/book/kishima/rubevy-wt-guard)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1m 36s
```

前半の記録が言っていたことを直した後のコードで測り直した形で、これがこのテストの存在理由その
ものである。ビルドは何も言わない。ガードだけが言う。

一時の 1 行を戻すと通る。

```
test the_guard_catches_what_it_is_for ... ok
test src_names_nothing_a_browser_lacks ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
```

## 数値

`cargo test`（全バイナリ）: **75 passed, 0 failed, 2 ignored**。
前半の記録の 73 passed に、今日の 2 つが足された数である。

`cargo clippy --all-targets`: 警告 **2 件**で、着手前とまったく同じもの
（`src/lib.rs:1849` の `collapsible_if`、`examples/headless.rs:47` の `type_complexity`、
どちらも main に元からあって今日触っていない）。新しいテストは警告を出していない。

`unsafe` は 1 つも書いていない。根拠の要る数も置いていない（このテストに閾値や上限は無い。
判定は名前の一覧と、行にコメントがあるかどうかだけである）。

## 捨てた案

* **シェルスクリプト（`tools/check_wasm.sh`）にする。** sabiruby の `tools/check_no_std.sh` に
  倣う形。5 行で書けるが、誰かが思い出して叩かないと走らない。`cargo test` の中に置けば、
  ローカルでも CI でも、rubevy を触った人が何もしなくても回る。use 文の平坦化のような、
  grep 1 本では書けない判定も素直に書ける。
* **cfg を解析する。** 上に書いた理由で、理由をコメントに書かせるほうを採った。
* **wasm で 1 フレーム回す仕掛け。** これは捨てたのではなく、**まだ**である。ガードは
  「名前」しか見ない — 例えばブラウザに無いのが std ではなく依存クレートの中にある日、
  あるいは wgpu の機能が足りない日は、何も言わない。本当のことを言えるのは走らせるほうだけで、
  rubevy を触った段階に `web/build.sh garden` と Playwright を入れる約束（`/home/kishima/book/CLAUDE.md`）
  はそのために残っている。今日のテストはその約束を置き換えるものではなく、**その前に**
  1 種類の間違いを止める安いふるいである。

## 今日ブラウザで確かめていないこと

`src/lib.rs` の変更はコメント 2 か所だけで、実行されるコードは 1 文字も変わっていない。
だから `web/build.sh garden` と Playwright での確認（pageerror 0、`?selftest`）は回していない。
上の `cargo build --target wasm32-unknown-unknown` が通ることは、一時の 1 行を入れた状態で
（つまり今より悪い状態で）見ている。
