# 2026-09-20 R6 を最初に使った人たちが見つけた 4 つ（R6b）

計画 `docs/plans/generalize-plan.md` の段階 **R6b** のみ。ブランチ `generalize`（worktree
`rubevy-wt-generalize`、R3 の `f72d851` の上）。**計測は無い段階**なので数字は取っていない。
4 つとも **足すだけ** で、既存の名前・意味・既定値は 1 つも変えていない。

出どころは 7 章の「気づいた点」の 4 行:

| 誰が | 何 |
|---|---|
| 09-20 R6 | `EmbeddedHost` を `set_host` したら `set_load_path` も書き直す必要があるのに、型に現れない |
| 09-20 S2（games） | `Program` にコンパイラへ渡すファイル名の置き場所が無い（同じ名前を 2 回書く） |
| 09-20 R2 | 壊れた `.mrb` を毎フレーム load し直し、`error!` が毎フレーム流れる |
| 09-20 R3 | 走り出してからの行番号は `debug_info` で組んだときだけ、と文書が言っていない |

着手前に読んだもの: `docs/worklog/2026-09-20-shared-entry-points.md`（R6 の記録）、
rubevy_games の `docs/worklog/2026-09-20-rubevy-entry-points.md`（S2 が R6 を使ってみた感触）、
games の実際の呼び出し（`garden/src/main.rs:2728-2740` `compile_source`、`:2821-2831`
`compile_world_source`、`sabibots/src/main.rs:1680-1694` `compile_text`。**読むだけ**、1 行も触っていない）。

## 1. 埋め込みの `Host` とロードパスを一緒に受ける口

### 名前を `embed` にしなかった

計画書は形の例として `ScriptWorld::embed(host, load_path)` を挙げ、「名前と置き場所は既存の API の流儀に
合わせて決め、理由を worklog に」と言っている。採ったのは

```rust
pub fn require_from(&mut self, host: impl sabiruby::Host + 'static, load_path: &[&str])
```

理由は 2 つ。(1) **この 2 つが一緒でなければならない理由は「`require` がどこを見るか」1 つ**で、
`embed`（＝ファイルがバイナリの中にある）はその理由ではなく、渡すものの出自の話である。
(2) 型を `EmbeddedHost` に絞らなかったので `embed` では狭すぎる — ブラウザで `fetch` から返す `Host` を
書く利用者も、同じ罠（ロードパスがプラグインのまま）を踏む。`Host` はバイナリの表とは限らない。

`Send + Sync` の bound は書かなかった。`sabiruby::Host` の supertrait なので clippy の
`implied_bounds_in_impls` が出る（実際に 2 件出して消した）。

置き場所は `ScriptWorld` のメソッド。`vm` フィールド越しに 2 回呼ぶものを 1 回にするのだから、
`ScriptWorld` にあるのが自然で、`EmbeddedHost` 側の関連関数にすると `Host` 一般に効かない。

### 忘れたときの手がかり: 採った（ただし 1 通りの形だけ）

計画書は「rustdoc、できれば実行時の 1 回の `warn!` も検討し、採る/採らないの理由を書く」と言っている。

まず **rubevy から `vm.set_host` は見えない**ことを確かめた。sabiruby 0.5.2 の `Vm` には
`set_host`（`src/vm.rs:2640`）も `set_load_path`（`:1041`、中身は `$LOAD_PATH` への代入）もあるが、
**読み出す口が無い**。だから「host を替えたのにロードパスが既定のまま」をプラグイン側で気づく方法は無い。
唯一の見張り台は `EmbeddedHost` の中 — VM がそれに訊きに来るから。

次に「外したときに何が見えるか」を実物で確かめた。`require "helper"` は `.mrb` → `.rb` の順、
ロードパスを 1 つずつ（`sabiruby-0.5.2/src/mrblib/require.rb:62-79`）試すので、
**うまくいっている require も毎回いくつも miss する**。`tests/embedded_host.rs` の既存のテストがその実例で、
`require "deep"` はロードパス `["ruby", "ruby/lib"]` のもとで `ruby/deep.mrb`・`ruby/deep.rb`・
`ruby/lib/deep.mrb` を外してから当たる。しかも host には「これが最後の候補だ」が伝わらない
（`__require_load_paths` の繰り返しは Ruby 側にあり、失敗は最後に LoadError を上げるだけ）。
**miss を見て警告する形は採れない**（動いているゲームで鳴る）。

採ったのは **「当たりようのない ask」だけを見る形**: 求められたパスの**先頭のディレクトリ**が
表の使っているディレクトリのどれでもないとき。表が `ruby/...` だけなら `assets/scripts/helper.rb` は
どんな名前でも当たらない — それが「プラグインのロードパスのまま」の状態そのものである。
上の `require "deep"` の 3 回の miss は全部先頭が `ruby` なので鳴らない。1 つの host につき 1 回
（`said` フラグ）。実物（一時的に `LogPlugin` を足して観測、コミット前に外した）:

```
WARN rubevy::embed: rubevy: an EmbeddedHost was asked for "assets/scripts/helper.mrb", and it has
nothing under assets/ — its 2 files are under ruby/. If a `require` is failing, the VM's load path
is still pointing where this host cannot reach: `ScriptWorld::require_from(host, &["ruby"])` sets
the host and the load path together. (said once)
```

この 1 件のあと、同じ `require` が続けて出す 3 つの ask（`assets/scripts/helper.rb`・
`assets/helper.mrb`・`assets/helper.rb`）では**もう出ない**ことも同じ走行で見た（出力が 1 行だけ）。

**偽陽性が無いとは言い切れない**: ロードパスに「表の外を指す項目」を混ぜ、それが当たる項目より前にあると、
require は成功するのに 1 回鳴る。ただしその項目は `EmbeddedHost` にとって**どうやっても当たらない項目**なので、
1 回言う価値のある状態だと判断した。文面も断定ではなく「もし require が失敗しているなら」と書いてある。

テストは `src/embed.rs` の単体 4 本（ふつうの require の miss では鳴らない／表の外からの ask は鳴り、
2 度目は鳴らない／ディレクトリの無い表は素の名前で届く／`directory_of` と `strip_here`）と、
`tests/embedded_host.rs` の `a_host_installed_without_its_load_path_finds_nothing`（罠そのものを
**LoadError として**固定した。ログの行はテストからは見ない — 見ていないことをテストの rustdoc に書いた）。

## 2. `Program` が名前を持つ

`pub name: String` を足した。`Program::new` のシグネチャは変えていない（`name` を捨てずに持つだけ）。
メソッドではなくフィールドにしたのは、`source` と `prelude_lines` がフィールドだから。

公開 API としては「pub フィールドが 1 つ増えた」＝構造体リテラルで作っていたコードが壊れる形だが、
`Program` は **R6 で入ったばかりで crates.io に出ていない**（0.0.1 に無い。公開は著者の指示待ち）ので、
壊れる利用者はいない。games の 3 か所は `Program::new` を呼んでいるだけで、リテラルは 0 件（確認済み）。

`docs/host-api.md` の例を `compile(&program.source, &program.name)` の形に直した。
S2 の報告 3 番が言っていた「片方だけ変えると区切りコメントとコンパイラのファイル名が食い違う」はこれで消える。

## 3. 壊れた `.mrb` を毎フレーム load し直さない

### 何が起きていたか

`start_scripts` は `pending: Query<(Entity, &Script<M>), Without<ScriptTask<M>>>` を回し、
load に失敗したら `error!` して `continue` していた。`continue` はエンティティを**手つかずで残す**ので、
次のフレームも同じエンティティが `pending` に出てくる。つまり N 台 × 毎フレームの parse とログ。
（`ScriptWorld::programs` は成功したものしか覚えないので、R2 の表も効かない。）

### 直し方: 覚えるのは「プログラム」、印を付けるのは「エンティティ」

2 つに分けた。

- **プログラム**: `ScriptWorld.broken: HashMap<Box<[u8]>, String>`。鍵は `programs` と同じ**バイト列そのもの**、
  値は VM が言ったこと。`irep_of` が `programs` → `broken` の順に見て、どちらにも無いときだけ `vm.load` する。
  `error!` は `irep_of` の中、**初めて会ったときの 1 回だけ**。同じ本文の 2 台目以降は parse もログも無しで
  同じ文面を受け取る。数え口 `ScriptWorld::broken_programs()`（`loaded_programs()` の対）。
- **エンティティ**: `ScriptEnded { status: Failed, value: <VM が言ったこと> }` を 1 回書き、
  `ScriptDone<M>` を挿す。`pending` の条件に `Without<ScriptDone<M>>` を足したので、以後は素通りされる。

「今の型で `ScriptEnded` の Failed が言えるか」は確かめた: `ScriptStatus::Failed` と `value: String` で
そのまま言える。`ScriptEnded` の rustdoc に「走り出さなかったスクリプトもこの形で終わる」と 1 段落足した。
新しいメッセージ型を足さなかったのは、ゲームから見れば news は同じ（このエンティティにスクリプトは走っていない）で、
終わりを表示している既存のゲームが**何も変えずに**これを表示できるから。

### やり直し: 印は「一時停止」であって判決ではない

`ScriptDone` を挿しただけだと、アセットが直っても二度と起動しない。そこで:

- **同じ id の中身の差し替え**（ホットリロード、エディタの Apply）: `retry_failed_scripts::<M>` を
  `RubevySet::Deliver` の先頭に足した。`MessageReader<AssetEvent<MrbAsset>>` の `Modified` を読み、
  そのアセットを指す**失敗したスクリプト**から印を外す。
- **別のアセット**: `replace_script` が前から `ScriptDone` を外しているので、そのまま通る。

失敗したエンティティを見分けるために非公開のマーカー `ScriptStartFailed<M>` を足した。
`ScriptDone` だけでは「走り切って終わった」と区別が付かず、`Modified` でそちらまで再起動してしまう。
マーカーは公開していない（ゲームが見るのは `ScriptEnded`）。起動に成功したときは
`Has<ScriptStartFailed<M>>` が真のときだけ外す（3000 台の起動に 1 台ぶんの `remove` も増やさないため）。

`AssetEvent::Removed` は見ていない。アセットが消えたエンティティにやり直すものは無い。

### 覚え続ける量に上限は要るか

**要らない**と判断し、理由を `ScriptWorld::broken` の rustdoc に書いた。`programs`（成功した方）と同じ議論:
鍵がプログラムそのものなので、**別々の**壊れたプログラムを作らない限り増えない。1 本につき増えるのは
そのバイト列 1 部で、そのバイト列を抱えている `MrbAsset` より小さく、`load` に成功した場合に VM が
永久に持つ irep より安い。上限を置けば (a) 根拠の無い数が 1 つ増え、(b) 上限からこぼれたプログラムは
「毎フレーム parse」に戻る。数え口を公開したのは、上限が無いものは外から見えるべきだから。

### テスト（`tests/broken_script.rs`、6 本）

- `a_broken_program_is_reported_once_however_long_it_stands`: **20 フレーム回して ending が 1 件**。
  フレーム数がこのテストの本体（前は「言われる回数＝生きたフレーム数」だった）。`ScriptDone` が付き、
  `broken_programs() == 1`、`loaded_programs() == 0`。
- `a_herd_of_one_broken_program_parses_it_once`: 同じバイト列を別の `AssetId` 2 つ・エンティティ 5 台。
  ending は 5 件（各エンティティに 1 回）、`broken_programs()` は 1。
- `two_broken_programs_are_two`。
- `an_asset_mended_under_its_own_handle_starts_the_script`: `Assets::insert(id, 直した .mrb)` で走り出す。
  印が外れていること、壊れた本文は覚えたままであることも見る。
- `replacing_a_broken_script_starts_the_new_one`。
- `a_script_that_runs_to_its_end_still_ends_the_way_it_did`: ふつうの終わり（`Finished` と `"42"`）が
  変わっていないこと。2 本の道が合流する場所なので置いた。

ログが 1 回であることは、テストからは見られない（`MinimalPlugins` に購読者がいない）。
作業中に一時的に `LogPlugin` を足して 20 フレーム走らせ、実物を見た:

```
ERROR rubevy: rubevy: 34v0 failed to load: invalid RITE binary: bad identifier (expected RITE)
```

これ 1 行きりだった。`LogPlugin` はコミット前に外してある。

## 4. `debug_info` の 1 行

`docs/host-api.md` の `Program` の節に足した。確かめたこと:

- `sabiruby_compiler::Options::debug_info` の既定は `false`（`sabiruby-compiler-0.2.2/src/lib.rs:45`）。
  `-g` に当たる（`:34`、`:111` で `ffi::DEBUG_INFO`）。
- **ブラウザの橋は引数で受け取る**: `sabiruby-playground/wasm/src/lib.rs:139-142` の
  `sabi_compile(src, len, debug)` が `debug_info: debug != 0`。そして JS 側の包み
  `sabiruby-playground/web/sabi.js:67` が `compile(src, debug = true)` — **ページの既定は付ける方**。

つまりネイティブの既定（無し）とブラウザの既定（有り）が逆向きで、そのことを書いた。

## 確認

`CARGO_TARGET_DIR=…/rubevy/target`（この repo で cargo を動かすのは自分だけ、という取り決め）。**計測はしていない。**

- `cargo test --workspace`: **135 passed / 0 failed / 3 ignored**（着手前 121 / 0 / 3。
  `tests/read_cost.rs` の 3 件は元から ignore）。増えた 14 は `tests/broken_script.rs` 6、
  `src/embed.rs` の単体 4、`tests/embedded_host.rs` +1、`tests/source.rs` +1、
  doctest +2（`ScriptWorld::require_from` と `Program::name` の例）。
- `cargo clippy --workspace --all-targets`: **新しい警告 0**。残る 2 件は着手前と同じ
  （`src/lib.rs` の `collapsible_if`、`examples/headless.rs` の `type_complexity`）。
  作業中に 4 件出して全部消した: `empty line after doc comment`、`items after a test module`
  （`mod tests` をファイル末尾へ）、`implied_bounds_in_impls` ×2（`Send + Sync` を外した）、
  `type_complexity`（`start_scripts` の query を `PendingScripts<M>` という `type` にした。
  `RunningTasks` と同じ流儀）。
- `cargo build --target wasm32-unknown-unknown --lib`: 通る。
- `cargo test --test no_wasm_unsupported`: 通る（足したものに時計もファイルもスレッドも無い）。
- `cargo doc --workspace --no-deps`: **警告 0**。途中 1 件出した
  （公開の rustdoc から非公開の `start_scripts` へのリンク）ので文に直した。
- `cargo fmt` は走らせていない。`git stash` も使っていない。
- 未追跡の `examples/factory_events.rs` / `examples/r3_limits.rs`（前の担当の計測器、R5 の材料）には触っていない。
  clippy は `--all-targets` でこの 2 本も見ているが、新しい警告は出していない。

### 確かめていないこと

- **ブラウザでは走らせていない。** 共通 CLAUDE.md の `web/build.sh` + Playwright は games 側の段階で行う
  （計画書 4 章の注記）。この段階で足したものに wasm で落ちる呼び出しは無い（`no_wasm_unsupported` が見ている）。
- `EmbeddedHost` の警告が**実際のブラウザのコンソールに**出ることは見ていない（ネイティブの `LogPlugin` で見た）。
- games はまだ `require_from` も `Program::name` も使っていない（読むだけの約束。使うのは games 側の段階）。

## 気づいた点

1. **`replace_script` は `ScriptStartFailed` を外さない。** 何を: 起動に失敗したエンティティに
   `replace_script` を使うと `ScriptDone` は外れるが非公開のマーカーは残る。どこで: `src/lib.rs` の
   `replace_script` と `start_scripts`。なぜ気にならないか（＝直していない理由）: 次の `start_scripts` が
   成功したところで外す（`Has<ScriptStartFailed<M>>`）ので、残るのは「差し替えたが新しい方も起動しない」間だけで、
   そのときは付いていてよい。なぜ書くか: `replace_script` が 3 つ目の `remove` を持つ形も同じくらい自然で、
   どちらが良いかは「マーカーを公開するか」を決めるときに一緒に決まる。どこに属するか: rubevy（設計の小さな分岐）。

2. **`MrbLoader` は壊れた `.mrb` をアセットの段階で弾くのに、`Assets::add` はしない。** 何を:
   `MrbLoader::load`（`src/lib.rs:128-130`）は `sabiruby::rite::parse` で検証してから `MrbAsset` を作るが、
   ゲームが自分でコンパイルして `Assets::add` する道（games の 3 本とも）はそこを通らない。
   なぜ気になるか: 今回直したのは後者の道の話で、前者の道では壊れたアセットはそもそも生まれない。
   つまり「壊れた `.mrb`」を実際に踏むのは**自分でコンパイルするゲーム**（＝プレイヤーが Ruby を書くゲーム）だけ。
   どこに属するか: 本の素材（同じ検証が 2 か所にあることと、その片方しか通らない道があること）。

3. **`ScriptEnded` が「走り出さなかった」と「走って例外で終わった」を区別しない。** 何を: どちらも
   `ScriptStatus::Failed`。どこで: `src/lib.rs` の `ScriptStatus`。なぜ気になるか: ゲームが再試行の
   ボタンを出したいとき、「文法は通ったが実行時に落ちた」と「そもそも積めなかった」は違う扱いになりうる。
   今回わざと足さなかった（列挙子を増やすのは公開 API の意味を変える側で、欲しい利用者がまだいない）。
   どこに属するか: rubevy（口の設計、要ると言う人が出てから）。

4. **`docs/host-api.md` の「Replacing and removing a script」は、失敗して止まったスクリプトのことを
   まだ「One program, one irep」の節でしか言っていない。** 何を: 差し替えの節から見ると、
   `replace_script` が「印を外す」役目も持ったことが読み取れない。どこに属するか: rubevy（文書の置き場所）。
   R10 か、次に host-api.md を大きく触る段階で 1 文。
