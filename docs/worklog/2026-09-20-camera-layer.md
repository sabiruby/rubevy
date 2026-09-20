# 2026-09-20 任意の Ruby 層と、最初の 1 枚（カメラ）— R9

計画 `docs/plans/generalize-plan.md` の段階 R9。到達点は「rubevy が同梱するが**読み込むかどうかは app が決める**
`.mrb`」と、その最初の 1 枚 `Rubevy::Camera`。土台は R0（`docs/worklog/2026-09-20-camera-from-ruby.md`、
通る形と通らない形）と、この直前に足した拒まれた書きの口（`-rejected-writes.md`）。

## 何を足したか（3 つだけ）

1. `ScriptWorld::load_and_run(&[u8]) -> Result<(), String>` — prelude と同じ走らせ方
   （`Vm::load_and_run`）を公開しただけ。`vm` は元から `pub` なので**新しい力は増えていない**。
   名前が付いたことと、エラーが `VmError` ではなくログに出せる 1 文になることが足したもの。
2. `pub mod layers { pub const CAMERA: &[u8] = include_bytes!("layers/camera.mrb"); }` —
   `PRELUDE` の入れ方（`include_bytes!`）に合わせた。定数かメソッドかは計画が挙げていたが、
   `PRELUDE` が定数なので定数。app は `world.load_and_run(rubevy::layers::CAMERA)` と書く。
3. `src/layers/camera.rb`（+ `.mrb`）。`tools/compile_scripts.sh` に 3 行足して `prelude.rb` と
   同じ道で生成する。`cargo package --list` に `src/layers/camera.rb` と `.mrb` が入ることを確かめた（118 ファイル）。

Rust の依存は 1 つも増えていない。`bevy_camera` は R0 のときから dev-dependency のままで、
`[dependencies]` は 1 文字も変えていない。`src/` に camera という語が入るのは `layers::CAMERA` の
rustdoc と `include_bytes!` のパスだけ。

## 著者判断 1（`zoom 2` は 2 倍に寄る）の実装と式

2D（`OrthographicProjection::scale`）は「画面に入るワールドの量」なので見かけの大きさは `1/scale`。
2 倍に寄る = `scale` を `factor` で割る。

3D（`PerspectiveProjection::fov`、ラジアン）は角度。見かけの大きさを決めるのは角度そのものではなく
**その正接**で、単位距離で画面の半分に入るのは `tan(fov/2)`。だから 2 倍に寄るのは
`tan(fov/2)` を半分にすることで、新しい角度は

```
fov' = 2 * atan( tan(fov/2) / factor )
```

`fov/2` としなかった理由を 2 つ書いた（rustdoc と `docs/host-api.md` にも）: (a) 角度が大きいほど
誤差が増える（小さい角度でだけ `tan x ≈ x` なので近似が成り立つ）、(b) `fov/factor` は `factor < 1`
で π を超えうるが、`atan` を通せば `factor` が正である限り必ず `0 < fov' < π` に落ちる。
後者は「クランプという根拠の無い数」を置かずに済ませる理由でもある。
テスト `zooming_a_3d_camera_halves_the_tangent_of_half_its_fov` が式そのものと、
`zoom 2` → `zoom 0.5` で元の角度にぴったり戻ることを主張する。

`Rubevy::Camera#scale` は**倍率**（`zoom 2` の後は 2.0）であって bevy の `scale` ではない。
名前が紛らわしいのは承知のうえで計画書の語（「`scale`（今の倍率）」）に合わせ、
rustdoc と文書の両方で「bevy の `scale` ではない」と言っている。

## 著者判断 3（答え手のいない問い）

`world_at x, y` は `Rubevy.ask("camera.world_at", @entity, x, y)` を投げて**キューを返す**。
`pop` しない。計画の綴りは `Rubevy.ask("camera.world_at", x, y)` だったが、**カメラのエンティティを
第 1 引数に足した** — `Request::entity` は聞いたスクリプトのエンティティであって
カメラではないので、これが無いとゲームは「どのカメラの話か」を知れない（複数のカメラは普通にある）。
答える側（共有 crate、F5）との約束はこの順: `("camera.world_at", camera_entity, x, y)`。
誰も答えなければ `pop` は永久に park する、と rustdoc・`docs/host-api.md`・テストの 3 か所に書いた。
答え手の有無を聞ける口は足していない（計画 6 章の判断どおり）。

## R0 の「通らない形」3 つへの対処

| R0 で通らなかった形 | 層の対処 | テスト |
|---|---|---|
| 変種の切り替え | `attach` / `reload` で最初に読んだ変種を覚え、そこにしか書かない。ホストが Rust 側で変えた場合のために `reload` | `reload_takes_the_projection_as_it_now_stands` |
| tuple 変種を名前で書く | 層が `{ 変種 => [ { field => value } ] }` を組む。スクリプトは形を知らない | 全部のズームのテスト |
| 同じ tick の 2 回のズーム | 倍率を Ruby 側（`@zoom`）に持ち、絶対値で書く | `..._two_zooms_in_one_tick_multiply` |

3 つ目は `pan` にも同じ問題があることに気づいたので、位置についても同じ手当てをした。ただし位置は
ホストが動かすこともあるので、ivar を無条件に信じるのではなく **`$rubevy[:frame]` で「自分がこの
フレームに書いた値か」を見分ける**: 同じフレームなら覚えている値、違うフレームならワールドを読む。
`$rubevy` の読みは往復を伴わない（グローバルの Hash）ので費用はほぼ 0。

拒まれた書きについては、**層が自分で見に行くのはやめた**。理由は 2 つ: (a) 書いた行は 1 フレーム前に
走り終わっているので、そこで例外を投げることはできず、投げれば関係のない後の呼びから飛び出す。
(b) 層が予想できる唯一の拒否（変種違い）は、層が**起こさないように作られている**。代わりに
`Rubevy::Camera#rejected_writes` を 3 行で足して、スクリプトが自分で見られるようにした
（`Rubevy.rejected_writes` のうち、このカメラのエンティティ宛のもの）。

## 途中で分かった VM の制約 — `initialize` では待てない

最初の版は `Rubevy::Camera.new(entity, kind)` の `initialize` の中で `reload`（＝ component の読み）を
していた。テストが全滅し、スクリプトの終了値が理由を言っていた:

```
SCRIPT ENDED 35v0 Failed #<RuntimeError: blocking pop cannot be called from within a C function boundary>
```

`Rubevy::Proxy` が最初に踏んだのと同じ壁。どこが境界なのか分からなかったので、使い捨てのテストで
7 通りを機械的に試した（`each` / `map` / `find` のブロック、`initialize`、普通のメソッド、`times`、`while`）:

```
== each-block: asked 1        == map-block: asked 1      == find-block: asked 1
== initialize: asked 0        ← Failed: blocking pop cannot be called from within a C function boundary
== method-then: asked 1       == times-block: asked 1    == while-loop: asked 1
```

**ブロックは全部通り、`initialize` だけが通らない。** `Class#new` がネイティブで、`initialize` は
その中で呼ばれるから。つまり rubevy のスクリプトでは「コンストラクタでワールドを読む」が書けない。
対処は `Rubevy::Camera.attach(entity, kind)` = `new`（何も読まない）+ `reload`（普通の Ruby フレームで読む）で、
`find` / `find_all` はこれを使う。`initialize` の rustdoc にこの理由を書いた。
これは層の都合ではなく **rubevy の口の性質**なので、下の「気づいた点」にも挙げた
（`docs/host-api.md` の「Why a read can wait inside `[]`」はブロックとネイティブの話をしているが、
`initialize` には触れていない）。

## example が短くなったか

`examples/camera_from_ruby.rs` を層で書き直した。行数だけ言うと Ruby は 26 行 → 24 行でほぼ同じだが、
**やっていることが増えている**（拒否の確認と `world_at` の往復が増え、変種切り替えの失敗例は
層が起こさないので消えた）。同じ仕事どうしで比べると:

| | 前（component 直書き） | 後（層） |
|---|---|---|
| パン | 4 行（読む・2 要素を足す・書き戻す） | `cam.move_to(100.0, 0.0)` + `cam.pan(20.0, -40.0)` の 2 行。しかも 1 tick に 2 回書ける |
| ズーム | `cam[:Projection] = { Orthographic: [ { scale: 2.0 } ] }`（変種名を知っている必要がある。1 tick に 1 回だけ効く） | `cam.zoom 2` ×2 で 4 倍 |
| 3D に変える | スクリプトを書き換える（`Perspective` と `fov`） | 同じ 1 行のまま |

出力（`cargo run --example camera_from_ruby`）:

```
[script] camera: #<Rubevy::Camera #<Rubevy::Entity 32v0> Camera2d Orthographic 1.0x>, components Camera2d, Projection, Transform
[script] now at [120.0, -40.0, 0.0] at 4.0x
[script] the world says [120.0, -40.0, 0.0] at scale 0.25
[script] refused: 0
[script] the middle of the window is "(120, -40)"
host: the camera is at Vec3(120.0, -40.0, 0.0) with scale 0.25
host: script on 33v0 ended: Finished nil
```

`4.0x`（層の倍率）と `scale 0.25`（bevy の数）が同じフレームの同じカメラの話であること、
`move_to` + `pan` の 2 回が両方効いて 120 になっていることが、この 2 行に出ている。

## 数について

層に埋め込んだ数は無い。`follow` の速さ・遅れは引数（`every`、`offset`）で、既定は
`every = 0`（「スクリプトが走るたび」）と `offset = nil`（ずれ無し）。どちらも「測って決めた既定値」
ではなく**単位元**（待たない、ずらさない）なので、共通 CLAUDE.md の言う「根拠の要る数」ではない。
カメラの目印の名前（`:Camera2d` / `:Camera3d`）と変種ごとの場（`:scale` / `:fov`）は bevy 0.19 の
名前そのもので、設定にはせず**メソッドにして開いてある**（`Rubevy::Camera.markers` を開き直せば
ゲーム独自の目印でも動く。Ruby なので設定より reopen の方が小さい）。

## 試して捨てたもの

- **`initialize` で読む**（上記）。VM の境界で不可能。
- **`follow` の既定を「今のずれを保つ」にする。** 最初はそう書いた（カメラが今いる場所と相手の
  差を覚えて維持する）。テストで「カメラが相手のところへ行かない」ことに気づいた —
  原点にいるカメラが x=100 の相手を追うと、ずれ -100 を保って原点に留まる。
  `follow` の平叙文の意味は「相手を写す」なので、既定はずれ無しにして `offset` を引数にした。
- **層が `Rubevy.rejected_writes` を毎回見て例外を投げる**（上記の理由で却下。口だけ足した）。
- **`Rubevy::Camera#scale` を bevy の数そのものにする。** 2D と 3D で意味も向きも違う数を
  そのまま見せることになり、「同じ意味にする」という著者判断 1 と矛盾する。
- **キューを `pop` する `world_at`。** 著者判断 3 のとおり投げっぱなし。

## 確認

同じ機械（13th Gen Intel Core i7-13700、WSL2、rustc 1.97.0、debug ビルド）。**この段階は計測の段階では
ないので時間の数字は取っていない**。機械は作業中ずっと別の担当（rubevy_games S6）の garden selftest が
動いていて load average 155 だったので、時間に依る数字は取るべきでないと判断した。

- `cargo test --workspace` → **171 passed, 0 failed, 4 ignored**（着手前 150 passed / 3 ignored。
  足したのは拒否の口の 7 + doc-test 1、カメラ層の 11 + ignored の計測器 1、doc-test 2）。
  計画 5 章が警告している `tests/child_task.rs` / `tests/pause.rs` / `tests/frame_time.rs` の揺れは
  この混み具合でも出なかった。
- `cargo clippy --workspace --all-targets` → 新しい警告 0。既存の 2 件
  （`src/lib.rs:2705` の `collapsible_if`（`HostCommand::SetPosition` の入れ子）と
  `examples/headless.rs` の `type_complexity`）だけ。
- `cargo build --target wasm32-unknown-unknown --lib` → 通る。`cargo test --test no_wasm_unsupported` → 2 passed。
  層は `.mrb` のバイト列なので wasm でもそのまま載る（`std::fs` も時計もスレッドも使っていない）。
- `cargo doc --no-deps` → 警告 0。
- `cargo package --list --allow-dirty` → 118 ファイル。`src/layers/camera.rb` と `src/layers/camera.mrb` が入っている。
- `cargo run --example camera_from_ruby` → 上の出力。

## 気づいた点（仕事の範囲の外。直していない）

1. **`initialize` の中ではワールドを読めない。** 何を: `Class#new` がネイティブなので、
   `initialize` の中の `e[:Transform]` は `blocking pop cannot be called from within a C function
   boundary` で落ちる（ブロックの中は落ちない — 上の 7 通りの表）。どこで: sabiruby の `Class#new`、
   rubevy 側は `docs/host-api.md` の「Why a read can wait inside `[]`」がこの話をしていない。
   なぜ気になるか: 「コンストラクタで初期状態を読む」は普通に書きたくなる形で、外の利用者は必ず踏む。
   落ち方が「スクリプトが黙って Failed で終わる」なので気づくのも遅れる。どこに属するか:
   rubevy（文書）と sabiruby（`Class#new` を Ruby 側に出せるか）。**文書に 1 段落足すのが一番安い。**
2. **`follow` の子タスクは、スクリプト本体が終わっても走り続ける。** 何を: `Task.new` で作った
   タスクは、作った側のスクリプトが最後の行まで行っても止まらない（エンティティが despawn されれば
   購読は閉じられるが、このタスクは component の読み書きをしているだけなので止まらない）。
   どこで: `src/layers/camera.rb` の `follow`、`src/lib.rs` の `stop_removed_task`。
   なぜ気になるか: カメラが despawn されたエンティティを追い続けることはない（相手が消えれば止まる）が、
   **カメラ自身**が despawn されると `move_to` は毎フレーム拒まれ続ける（`rejected_writes` に毎フレーム出る）。
   どこに属するか: rubevy（子タスクの寿命の設計）。今回は相手側の停止条件だけ実装した。
3. **`Rubevy::Camera#scale` と bevy の `Projection::scale` が同じ名前で逆の意味。** 何を: 層の
   `scale` は倍率（大きいほど寄る）、bevy の `scale` は画角（大きいほど引く）。どこで:
   `src/layers/camera.rb`。なぜ気になるか: 同じ画面に両方の数が出る（example の出力がまさにそれ:
   `4.0x` と `scale 0.25`）。どこに属するか: 層の命名。計画書の語に合わせたが、
   `magnification` / `zoom_level` のような別の名前にする手もある — **著者判断が要ると思う**。
4. **層を 2 枚目以降どう足すかの決まりが無い。** 何を: `layers::CAMERA` は定数 1 つで、
   層どうしの依存（ある層が別の層を前提にする）や、同じ層を 2 回読んだときの扱い（今は 2 回走る）に
   ついて何も決めていない。どこで: `src/lib.rs` の `pub mod layers`。なぜ気になるか: 2 枚目
   （入力、UI）が出たときに決めることになる。どこに属するか: 計画書（R10 以降）。
