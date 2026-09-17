# 2026-09-18 ブラウザで最初のフレームが panic する — 答えループの時計

## 症状

公開中のブラウザ版 <https://sabiruby.github.io/rubevy_games/garden/> を Chrome で開くと、
ページの「loading the game…」の文字までは出るのに、そこから先が真っ黒のまま動かない。
著者が Chrome の console を貼ってくれた 1 行がすべてだった。

```
panicked at .../library/std/src/sys/time/unsupported.rs:13:9:
time not implemented on this platform
```

rubevy_games 側の worktree（`rubevy_games-wt-web`、main `b1e5ec1`）で `web/build.sh garden` を
組み、`python3 -m http.server` で配って headless Chromium（playwright-core、SwiftShader の
ソフトウェア WebGL）で開いたら、同じものが同じ順番で出た。手元の実文は次のとおり。

```
[console.log] INFO bevy_render/renderer/mod.rs:288 AdapterInfo { name: "ANGLE (Google, Vulkan 1.3.0
  (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver)", ... backend: Gl, ... }
[console.log] WARN bevy_egui-0.42.0/src/render/mod.rs:263 Feature TEXTURE_BINDING_ARRAY is not
  supported on this device.
[console.error] panicked at /rustc/2d8144b7.../library/std/src/sys/time/unsupported.rs:13:9:
time not implemented on this platform
[pageerror] RuntimeError: unreachable
    at .../pkg/game_bg.wasm:wasm-function[144417]:0x1f7e1a4
== status {"hidden":true,"text":"Garden loading the game…",
           "canvas":{"w":300,"h":150,"cw":1280,"ch":800}}
== pixels {"lit":0,"of":16000,"meanRGBsum":0}
```

最後の 2 行は、黒画面を「黒い」と言えるようにするために足した計測である。canvas を 160×100 に
縮めて `getImageData` で読み、R+G+B が 30 を超える画素を数える。16000 画素のうち **0**、平均も
0 — 文字どおり真っ黒だった。canvas の内部解像度が `300×150` のまま（CSS 上は 1280×800）なのも
同じことを別の側から言っている。Bevy の `fit_canvas_to_parent` は毎フレーム走る系なので、
**1 フレームも完走していない**。

bevy の起動ログ（アダプタの選択、egui のフィーチャ警告）はすべて出ているから、描画側は生きて
いた。死んだのはその直後、最初の `Update` の中である。

## 当たり

`grep -rn "std::time::Instant\|Instant::now" src/` は 2 か所しか出さない。

```
src/lib.rs:1233:    static ORIGIN: std::sync::OnceLock<bevy::platform::time::Instant> = ...
src/lib.rs:1234:    ORIGIN.get_or_init(bevy::platform::time::Instant::now).elapsed()...
src/lib.rs:1676:        let started = std::time::Instant::now();
```

1233〜1234 は `clock_ns`、VM に `task_set_clock` で渡している時計で、rustdoc にも
「Bevy の `Instant`（ブラウザにもあるほう）」と書いてある。1676 は `tick_scripts` の**答えループ**の
頭で、そこだけが `std::time::Instant` だった。`wasm32-unknown-unknown` には std の時計の実装が
なく、`Instant::now()` は `sys/time/unsupported.rs` の `panic!` に落ちる。console の 1 行の
ファイル名がそのまま答えになっている。

`git log -S "std::time::Instant::now" -- src/lib.rs` は 1 件だけ返す。`d0e9b85`
（*sync reads: the tick answers its scripts' component reads between two runs of the VM*）、
S1 のコミットである。`git show d0e9b85` の該当箇所を見ると、S1 の前は

```rust
time_ns: world.frame_time.map(|d| d.as_nanos() as u64),
```

の 1 行で、VM を 1 回走らせるだけだったから**フレームの経過時間を数える必要がなかった**。
S1 が「走らせる → 答える → また走らせる」のループにしたときに、残り時間を引き算するための
時計が要って、そこで手近な `std::time::Instant` が入った。ネイティブでは正しく動くので、
rubevy のテストも rubevy_games のネイティブのチェックも全部通っていた。ブラウザで動かして
初めて出る種類の間違いで、S1〜S4 の間ずっと入っていた。

`cargo build --target wasm32-unknown-unknown` は**この状態でも通る**（後述）。コンパイル時には
何も分からず、走らせて初めて `unreachable` になる。CI に wasm ビルドを足しても、これは捕まら
なかった。

## 直し

`src/lib.rs` の答えループの時計を、**VM 自身に渡してある時計**（`clock_ns`）に替えた。
候補は 2 つあった。

1. `bevy::platform::time::Instant::now()` にする。1 行で済む。
2. `clock_ns()` の差を取る。`clock_ns` は `bevy::platform::time::Instant` の
   `OnceLock` を起点にした経過ナノ秒で、**`task_set_clock` で VM に渡しているのと同じ関数**。

2 を選んだ。理由は依存でも行数でもなく、このループが `RunLimits::time_ns` に渡す「残り時間」を
VM が測り直す側の時計と、ループ自身が「もう何ナノ秒使ったか」を測る時計が、同じ源であるべき
だからである。1 でも `Instant` の実装は同じだが、起点が別になり、2 つの時計が同じものを指して
いるという保証はコードのどこにも書かれないままになる。新しい依存は足していない
（`bevy::platform` は既に使っている）。

```rust
let started_ns = clock_ns();
let mut spent = 0u64;
loop {
    let left = scripts.budget.saturating_sub(spent);
    if left == 0 { break; }
    let time_left = scripts
        .frame_time
        .map(|t| (t.as_nanos() as u64).saturating_sub(clock_ns().saturating_sub(started_ns)));
    if time_left == Some(0) { break; }
    let limits = sabiruby::RunLimits { instructions: Some(left), time_ns: time_left, ... };
```

型が `Option<Duration>` から `Option<u64>`（ナノ秒）に変わり、`time_ns` に渡すところの
`.map(|d| d.as_nanos() as u64)` が消えた。`RunLimits::time_ns` はもともと `Option<u64>` の
ナノ秒なので、変換が 1 回減っている。`saturating_sub` を 2 段にしてあるのは、`clock_ns` が
単調だとしても引き算の向きをコードの上で見えるようにしておくため。

`ScriptWorld::frame_time` の rustdoc は前から「on the VM's clock (Bevy's `Instant`)」と
書いてあった。**文書のほうが正しく、コードが違っていた**。直したのはコードのほうである。

## 確かめたこと

`cargo build --target wasm32-unknown-unknown`（rubevy 単体、既定フィーチャ）:

```
   Compiling rubevy v0.0.1 (/home/kishima/book/kishima/rubevy-wt-wasm)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 31.57s
```

通る。`web-time` と `bevy_platform` が依存に並んでいるのがログに見える。つまり
`bevy::platform::time::Instant` はこのターゲットでも本物の時計であって、std のほうだけが
`unsupported` に落ちていた。

`cargo test`: **73 passed, 0 failed, 2 ignored**（18 のテストバイナリ）。

1 回目の測定では `tests/pause.rs` の `a_pause_does_not_spend_a_sleep` が落ちた。

```
thread 'a_pause_does_not_spend_a_sleep' panicked at tests/pause.rs:78:5:
assertion `left == right` failed: it said it was starting
  left: 2
 right: 1
```

落ちたのは「ポーズ」の assertion ではなく、**その前の 4 フレームの assertion**（78 行目、
`marks.len() == 1`）である。この 4 フレームは `sleep 0.1` に届くまでを走らせるだけの助走で、
1 フレームあたり `thread::sleep(2ms)` しか入れていないから、機械が空いていれば 0.1 秒には
まったく届かない。このとき同じ機械で別の担当が release ビルドを回していて（`uptime` の
load average が 6.17）、`app.update()` 自体が数十 ms かかり、4 フレームで Bevy の `Time` が
0.1 秒を越えてスリープが明けてしまっていた。**機械の混み具合の flake で、この変更とは関係が
ない。** 機械が空いてから `cargo test --test pause` を 5 回回して 5/5 通り、その後の
`cargo test` 全体も 0 failed だった。ポーズの機構には手を触れていない。

`cargo build --examples`: 通る。
`cargo clippy --all-targets`: 警告 2 件。`src/lib.rs:1849` の `collapsible_if` と
`examples/headless.rs:47` の `type_complexity` で、どちらも `main` に元からあり
（`git show main:src/lib.rs` の 1841 行目、同 `examples/headless.rs` の 47 行目が同じもの）、
今回触った行ではない。**増えても減ってもいない。**

## ゲーム側で見たもの

rubevy_games の worktree で、`.cargo/config.toml` に

```toml
[patch."https://github.com/sabiruby/rubevy"]
rubevy = { path = "/home/kishima/book/kishima/rubevy-wt-wasm" }
```

を一時的に置いて `web/build.sh garden` を組み直し、同じ headless Chromium で開いた結果は
rubevy_games 側の `docs/worklog/2026-09-18-web-black-screen.md` に書いた。要点だけ:

* 45 秒の普通の走行で **pageerror 0、panic 0、404 も 0**。canvas は `300×150` から
  **1280×800** に伸び（1 フレームも走っていなければ伸びない）、スクリーンショットには
  12 匹・43 株の庭と HUD と `world.rb` を出したエディタが写っている。
* 160 秒の `?selftest` で **`selftest: ok` が 40 行、`FAIL` が 0 行**。W3 の
  `F3` まわりの 11 個（*the day is what world.rb says it is*、*Ctrl+Enter: the garden is
  running the edited rules, without stopping* など）も入っている。
* ネイティブも壊していない。`GARDEN_SELFTEST=1 garden --headless 90` を 2 回（13 ok / 0 FAIL、
  panic 0）、`SABIBOTS_SELFTEST=1 sabibots --headless 90` を 1 回（51 ok / 0 FAIL、panic 0）。

`.cargo/config.toml` と `Cargo.lock` は後始末で消してある（ゲーム側にはコミットしない。
rubevy が main に入ってから本体が `cargo update -p rubevy` する）。

## 残したいこと

ネイティブで全部通るのにブラウザで最初のフレームが死ぬ、という形の間違いが 1 つ入っていた
のだから、同じ形のものをこれから入れないための道はあったほうがいい。ただし
`cargo build --target wasm32-unknown-unknown` は**この panic を捕まえなかった**（上のとおり
バグのあるコードでも通る）ので、ビルドを CI に足すだけでは足りない。捕まえるには
`std::time::Instant` / `SystemTime::now` / `std::thread` を src/ で禁じる grep か、
wasm でヘッドレスに 1 フレーム回す仕掛けのどちらかが要る。前者は 5 行で書けて今日の 1 件は
確実に止められ、後者は本当のことを測るが道具が要る。**どちらを採るかは著者の判断待ちで、
この作業では入れていない。** 今日の時点で src/ にある非 wasm な std の呼び出しは、この
1 か所だけだった（`SystemTime`、`thread::sleep`、`std::thread` は src/ に無い。tests/ には
あるが、テストはネイティブでしか走らない）。
