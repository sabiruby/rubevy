# 2026-09-20 — R9 の最初の読者が踏んだ 4 つと、書かれていなかったこと（R10 その 2）

計画書 `docs/plans/generalize-plan.md` の **R10**、後半。前半（数の棚卸し）は
`2026-09-20-numbers-inventory.md`。こちらは 7 章に溜まっていた小さな宿題 —
カメラ層の `scale`、`follow` の止まり方、層を 2 回読んだとき、そして文書 7 か所 — を片付けた記録。

---

## 1. `scale` → `magnification`

R9 の担当が自分で挙げた件（計画書 7 章、09-20 R9 の行）。`Rubevy::Camera#scale` は
**倍率**（大きいほど寄る）で、bevy の `OrthographicProjection::scale` は**画面に入る世界の量**
（小さいほど寄る）。同じ語が境界の両側で逆の意味を持っていて、example の出力に `4.0x` と
`scale 0.25` が並ぶ。著者の判断は「`magnification` に改名。未公開なので別名は残さない」。

機械的な置き換えだが、置き換えて**いけない** `scale` が同じファイルに 5 か所ある
（bevy 側の `scale` を説明している行と、`zoom_field` が返す `:scale`）。grep で 8 か所を見て、
3 か所だけを直した。`src/layers/camera.rb`、`tests/camera_layer.rs`（7 か所の Ruby 片）、
`examples/camera_from_ruby.rs`、`docs/host-api.md`、`CHANGELOG.md` の R9 の項。

`magnification` の rustdoc（Ruby のコメント）には、**なぜ `scale` ではないのか**を 1 段落書いた。
名前を変えるだけでは、次に読む人が「短いから `scale` にしよう」と戻せてしまう。

`.mrb` は `tools/compile_scripts.sh`（Docker の mrbc 4.1.0-rc）。Docker は動いていたので使った
（起動はしていない）。走らせる前に全 `.mrb` の md5 を取り、後で照合したところ
**変わったのは `src/layers/camera.mrb` の 1 つだけ**だった — mrbc の出力は同じ入力に対して同じなので、
このスクリプトは全ファイルを作り直しても差分を出さない。

## 2. `follow` の止まり方 — 計画書の前提が実測と違った

計画書 7 章はこう書いている: 「`follow` の子タスクは…カメラ自身が despawn されると `move_to` が
**毎フレーム拒まれ続ける**」。**これは違った。** 手を入れる前に測った。

`tests/camera_layer.rs` に一時的な probe を置き、カメラを despawn した前後の
`ScriptWorld::last_frame().instructions` を毎フレーム読んだ:

```
PROBE instructions before despawn: [236, 236, 236, 236, 236]
PROBE instructions after  despawn: [193, 0, 0, 0, 0, 0, 0, 0, 0, 0]
PROBE rejected_writes after 30 frames: 0
PROBE following?: Some("yes")
```

つまり **子タスクは気づいたフレーム（193 命令）で終わっていて**、そのあと VM は 1 命令も使っていない。
拒まれた書きも 0 件である。なぜか: `move_to` は `position` を読み、`position` は
`@entity[:Transform]` を読む。**despawn されたエンティティの component は nil** なので `position` が nil、
`move_to` が nil、ループが `break` する。そして**拒否が 0 件**なのは、
`apply_component_writes` が「消えたエンティティへの書きは拒否ではない」と決めているから
（`src/lib.rs`、「there is nothing left to refuse」）。計画書の前提は、この 2 つを見ずに書かれていた。

**本当に壊れているのは `following?`** で、タスクが自分で終わったあとも true を返し続ける。
「別のものを追い始めてよいか」を `following?` で見るスクリプトは、永久に見られない。
目標が消えた場合（既存のテストがある方）も同じで、そちらのテストは `following?` を見ていなかった。

### 直し方: フラグをやめて、タスクそのものを印にする

`@following`（真偽）と `@follower`（タスク）の 2 つがあり、食い違えるのが元だった。
`@following` を消して、`following?` を `@follower ? true : false` にした。
子タスクのループは `while camera.follower == me`（`me = Task.current`）になり、抜けたあとに
`camera.unfollow if camera.follower == me` で自分の印を外す。

`me` と比べるのは、**後から来た `follow` の印を消さない**ため。`Task.new` は
ブロックを即座には走らせず、`@follower = Task.new { ... }` の代入が終わってから
スケジューラが回すので、ブロックの中で `camera.follower` は必ず入っている。

ここで 1 つ確かめたことがある。**2 回目の `follow` が 1 回目のタスクを残さないか。**
残るなら、カメラを取り合う 2 本が走る。probe を書いた:

```
PROBE instructions, one follower then two: [.., 236, 463, 236] / [236, 236, ...]   # 2 本にならない
PROBE after two follows, camera x: -100.0
PROBE after the second target went, camera x: -100.0    # 1 本目が生きていれば 100.0 に戻るはず
```

**残っていない。** 理由は読めば分かる: `follow` は先頭で `unfollow` を呼び、そのあと
`position` と `translation_of` を読む — **この 2 つは `Rubevy.ask` でタスクが park する**。
その隙に古いタスクが起き、印が外れているのを見てループを抜ける。
今回の書き換えはこの偶然に寄りかからなくなった（印はタスクの同一性なので、park しようがしまいが正しい）が、
**前も壊れてはいなかった**ことは記録しておく。

`tests/camera_layer.rs` に `following_stops_when_the_camera_itself_is_gone` を足した
（命令数が 0 に落ちること、拒否が 0 件であること、`following?` が false になること）。
既存の `following_stops_when_the_target_is_gone` にも `following?` の行は足していない —
そちらは「スクリプトが最後の行まで走った」を見ており、新しいテストが両方を見るため。

## 3. 層は 2 回読んでも 1 回

計画書は「層の側で守るか `ScriptWorld` の側で覚えるか、小さい方を選ぶ」と言っている。

**層の側**（`camera.rb` を `unless defined?(Rubevy::Camera)` で囲む）は、290 行のインデントが全部動き、
しかも**その層 1 枚しか守らない** — 2 枚目を足す人が同じことを覚えていなければならない。

**`ScriptWorld` の側**にした。`load_and_run` が走らせたプログラムをバイト列で覚え、
2 回目は走らせずに `Ok(false)` を返す。戻り値を `Result<(), String>` から `Result<bool, String>` に
広げたのは、「走らなかった」を黙って `Ok(())` で返すのが嘘になるから。未公開の API なので壊れる利用者はいない
（呼び出し 4 か所は `.expect(..)` を文として書いており、そのまま通る）。

鍵をバイト列にしたのは R2 の `programs` と同じ理由で、同じ表には**しなかった** — 2 つは違う問いに答える
（「このプログラムのタスクはどの irep から生えるか」と「このプログラムの top level はもう走ったか」）。
覚える量に上限は置いていない: 層とライブラリは app の `Startup` に名前で書かれるもので、
実行時に作られるものではないから、有限である。この理由はフィールドの rustdoc に書いた。

失敗したプログラムは覚えない（直して再試行できるように）。

テスト `a_layer_taken_up_twice_is_run_once`: top level が `$ran` を増やすプログラムを 2 回読ませて
`$ran == 1`、別のプログラムは走る、カメラ層も 2 回目は `Ok(false)`。
**カメラ層そのものはメソッドを定義するだけなので 2 回走っても同じに見える** — だから
「top level が何かする」プログラムでテストした。

`docs/host-api.md` に「層を 2 枚目以降どう足すか」を 5 点で書いた: 置き場所（`src/layers/<name>.rb` と
`.mrb`、`pub mod layers` の `const`）、依存してよいもの（ホスト API だけ。**Rust は無し** — Rust が要るなら
それは層ではない）、名前（`Rubevy::` の下、`ask` の kind は `<thing>.<verb>`）、top level でしてはいけないこと
（park。読みは park なので、読むのは後で呼ばれるメソッドから）、そして `initialize` では読めないこと。

## 4. 文書

7 章から拾った 7 件。どれも「実物はそうなのに、どこにも書いていない」もの。

* **`initialize` の中ではワールドを読めない**（R9 の発見）。`Class#new` がネイティブなので、
  中の `e[:Transform]` は `blocking pop cannot be called from within a C function boundary` で
  タスクごと死に、`ScriptEnded` の `Failed` になる — **rescue できる例外として見えない**。
  「Why a read can wait inside `[]`」の隣に 1 段落と、2 段階に分ける書き方の例（`Rubevy::Camera.attach` の形）。
* **時計の粒度 4 ms** を「Time」に（R10 前半で入れた。`sleep 0` は「次のフレーム」ではなく
  「時計が次に進むまで」で、60 Hz なら結果的に次のフレームだが、240 Hz でも同じで、
  「次の 1 フレームだけ待つ」とは言えない — R11 の動機）。
* **`replace_script` は起動に失敗した印も外す**（R6b の気づいた点）。「Replacing and removing a script」に 1 文。
* **`reflect_cache` / `resource_cache` は型が登録し直されると古いまま**（R8 の気づいた点）。
  rustdoc に 1 段落。**見たことは無い**ことと、直す手が無い（レジストリは「変わった」と言わない）ことも書いた。
* **`$rubevy` と `Rubevy.resource("Time<Virtual>")` のどちらを勧めるか**（R8 の気づいた点）。
  ここで 1 つ書き直した: 最初「`$rubevy` はゲームのポーズを見ない」と書いたが、
  **bevy の `time_system` は `Update` の `Time` を `Time<Virtual>` から作る**
  （`bevy_time-0.19.1/src/lib.rs` の `update_virtual_time(&mut time, ...)`）ので、**`$rubevy` は既に仮想時計**である。
  推測で書いて、ソースを見て直した。勧めは「`$rubevy` を使え。resource は `Time<Real>` や
  `Time<Fixed>` のように `$rubevy` に無いものが要るときだけ」。
* **`ScriptWorld::publish` の rustdoc**に「誰も聞いていない名前への publish は購読の総数に依らない」（R1）。
  この rustdoc は前から「自由に publish してよい」と書いていて、R1 の前はその「自由に」が
  購読の総数ぶんかかっていた（R1 の担当が「文書のほうが先に正しかった例」として挙げている）。
  今は費用の方も正しいので、数字ごと書いた。
* **`cargo fmt` を使わない**旨を `docs/README.md` の「Working on this repository」に。
  理由つき（rustfmt の設定が無く、既存のコードは手で幅を揃えてあるので、走らせると全ファイルに
  無関係な差分が出る — 差分は変更の理由を言うためのもの）。ついでに `.rb`/`.mrb` の作り直しと、
  数を足したら `numbers.md` に 1 行、も同じ箇条書きに入れた。

## 5. `tests/embedded_host.rs` の生成物

計画書の宿題: 「生成物 `assets/scripts/helper.mrb` が古いと落ちる確認を足せるか見る。
`.rb` と `.mrb` の対応を確かめる安い方法があるか。無ければ無いと書く」。

**バイト列として比べる道は無い。** `.mrb` は Docker の参照 mrbc 4.1.0-rc が作ったもので、
テスト時に使える唯一のコンパイラは `sabiruby-compiler`（dev-dependency）。同じソースから同じバイトは出ない。
mtime を見る道も採らなかった（clone や checkout の順で意味が変わる）。

**振る舞いで比べる道はあった**ので、それを足した。`assets/scripts/helper.rb` を `include_str!` で
読んでテスト時にコンパイルし、コミットされた `.mrb` と**同じ質問**（`Helper.greet("x")`）を投げて
答えを比べる。`helper.rb` の文字列を `"hi, #{who}"` に変えて走らせ、

```
assertion `left == right` failed: assets/scripts/helper.mrb is out of date: run tools/compile_scripts.sh
  left: ["hello, x"]   right: ["hi, x"]
```

と落ちることを確かめてから戻した。**この質問で見えない変更は見えない**ので、そう書いた
（「見える範囲を広げるには、ここで訊くことを増やす」）。

## 6. 確認

```
cargo test --workspace          178 passed / 4 ignored
                                （R10 前 171。前半の `tests/max_depth.rs` で +4、後半で +3 — `camera_layer` に 2 件、`embedded_host` に 1 件）
cargo clippy --workspace --all-targets   新しい警告 0（既存の 2 件のみ）
cargo build --target wasm32-unknown-unknown --lib        通る
cargo test --test no_wasm_unsupported                    2 passed
cargo doc --no-deps                                      警告 0
tools/compile_scripts.sh        camera.mrb だけが変わった（md5 で確認）
```

計測は取っていない（機械が塞がっていた。前半の記録の §5 と同じ）。

## 7. 気づいた点

* **計画書 7 章の 1 行が実測と違っていた**（§2）。「カメラが despawn されると `move_to` が毎フレーム
  拒まれ続ける」は起きない — 子タスクはその場で終わり、消えたエンティティへの書きは拒否として数えられない。
  R9 の担当は層を読んで推測したのだと思われる。**属する先: 計画書（前提違い）**
* **消えたエンティティへの書きが `rejected_writes` に出ないことは、`host-api.md` に書いてある**
  （「A write to an entity that was despawned in the meantime is **not** here」）。
  そのとおりで正しいが、**「書いた側は黙って何も起きない」** という面は書かれていない。
  追いかけているカメラが消えたことにスクリプトが気づく道は、今は「書きが効かない」ことを
  自分で読みに行くしかない。R9 の `rejected_writes` の設計の穴ではなく、意図的な線引きの副作用。
  **属する先: rubevy（口の設計。欲しい利用者が出てから）**
* **`Rubevy::Camera#follower` という公開の口が 1 つ増えた**（§2）。層の API なので Rust の公開 API は
  増えていないが、層も外の利用者が使うものなので、`host-api.md` のカメラの節に 1 行入れた。
  「追っているタスクを持てる」こと自体は `follow` の戻り値で前からできていたので、足したのは
  「カメラの側から訊ける」道だけ。**属する先: 記録のみ**
* **`load_and_run` の戻り値を広げた**（`Result<(), String>` → `Result<bool, String>`）。
  crates.io に出ていない口なので壊れる利用者はいないが、**公開 API は足すだけ**という既定
  （計画書 2 章）からは外れる。未公開であることを理由に採った。**属する先: 記録のみ（著者の確認があれば）**
* **`tests/camera_layer.rs` が 13 件になった。** カメラ層 1 枚のテストとしては多く、
  層が 2 枚目・3 枚目と増えるとファイルの切り方を決める必要が出る（`tests/layers/` か、層ごとに 1 ファイルか）。
  今は 1 枚なので触っていない。**属する先: repo の作法（2 枚目を足すとき）**
