# 2026-09-20 Ruby からカメラを動かせるか（R0）

計画 `docs/plans/generalize-plan.md` の段階 R0。到達点は「**今の rubevy のまま**（`src/` を変えずに）、Ruby の
スクリプトから `Camera2d` のエンティティを見つけ、`cam[:Transform] =` で動かし、`Projection` の `scale` を書いて
ズームできるかどうかが**分かっている**こと」。結論から書くと **3 つとも書ける**。`src/` は 1 行も変えていない。
変えたのは `Cargo.toml` の `[dev-dependencies]` に bevy の `bevy_camera` feature を足した 1 行と、
新しい `tests/camera.rs`・`examples/camera_from_ruby.rs` だけ。

R9（任意の Ruby 層、`Rubevy::Camera`）の形はこの結果で決まる、というのが計画の建て付けなので、
「書ける／書けない」だけでなく **どう書けば通るか・読みで何が返るか・2D と 3D で何が違うか**を以下に残す。

## bevy 側の事実（ソースで確かめた）

bevy 0.19.1、`~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/` の中身を読んだ。

| 型 | 場所 | リフレクションの登録 |
|---|---|---|
| `Camera2d`（フィールドの無い unit struct） | `bevy_camera-0.19.1/src/components.rs:9-16` | `#[derive(Component, Default, Reflect, Clone)]` + `#[reflect(Component, Default, Clone)]` — **`ReflectComponent` あり** |
| `Camera3d`（`depth_load_op` と `depth_texture_usages` を持つ struct） | 同 `:21-30` | `#[reflect(Component, Default, Clone)]` — **あり** |
| `Projection`（enum。`Perspective(..)` / `Orthographic(..)` / `Custom(..)`、どれも tuple 変種） | `bevy_camera-0.19.1/src/projection.rs:214-220` | `#[derive(Component, Debug, Clone, Reflect, From)]` + `#[reflect(Component, Default, Debug, Clone)]` — **あり** |
| `OrthographicProjection`（`near`/`far`/`viewport_origin`/`scaling_mode`/`scale`/`area`） | 同 `:578-634` | `#[reflect(Debug, FromWorld, Clone)]` — `Component` は**無い**（0.19 では component ではなく `Projection` の中身） |
| `PerspectiveProjection`（`fov`/`aspect_ratio`/`near`/`far`/`near_clip_plane`） | 同 `:281-334` | `#[reflect(Default, Debug, Clone)]` — 同じく component ではない |

つまり `Rubevy.find(:Camera2d)` も `cam[:Projection]` も、rubevy が今持っている `ReflectComponent` の道に素直に乗る。
`OrthographicProjection` に `ReflectComponent` が無いのは問題にならない — 中身に降りるのは `apply_ruby` /
`reflect_to_ruby` の再帰で、レジストリを引くのは一番外の component 名 1 つだけ（`src/lib.rs:1928-1932`,
`:2113-2117`）。

`Camera2d` の `#[require(...)]` は `Camera`、`Projection::Orthographic(OrthographicProjection::default_2d())`、
`Frustum` を連れてくる。`Camera` はさらに `RenderTarget` などを要求するので、ヘッドレスでは `Camera2d` を
単独で spawn せず、**スクリプトが触る 3 つ（`Camera2d` / `Transform` / `Projection`）を手で載せた**。
どの 3 つかをコードに書いておく意味もある。

登録については、rubevy は `default-features = false` で bevy を引いているので bevy の自動登録
（`reflect_auto_register`、bevy 0.19.1 の `default` の中の `default_app` に入っている。`bevy-0.19.1/Cargo.toml:2749-2754`）
が効かない。`bevy_camera` にも `bevy_transform` にも `register_type` は 1 つも無い（grep で確認）ので、
テストと example は `register_type::<Camera2d>()` などを自分で書く。`examples/components.rs:34-37` が
`Transform` についてやっているのと同じ。

## 実際に書いた形

### パン（位置）

```ruby
cam = Rubevy.find(:Camera2d)[0]
tf = cam[:Transform]
tf[:translation][0] += 120.0
cam[:Transform] = tf
```

これは component 書きそのもので、何も特別なことは無い。`Camera2d` は unit struct なので `cam[:Camera2d]` は
**空の Hash `{}`**（nil ではない）。`Rubevy.find(:Camera2d)` は `Rubevy::Entity` の Array を返す。
`cam.components` は `Camera2d, Projection, Transform` を返した（登録した 3 つ。`Camera` などは登録していないので見えない）。

### ズーム（`Projection` の `scale`）

**書ける。** 形はこれ:

```ruby
cam[:Projection] = { Orthographic: [ { scale: 2.5 } ] }
```

3 段になっているのには 3 段ぶんの理由がある。外側の 1 要素 Hash は **enum の変種名**（`src/reflect.rs`
`apply_enum`）。その値が Array なのは `Projection::Orthographic(..)` が **tuple 変種**で、Array が変種の
フィールドを位置で並べたものだから。いちばん内側の Hash は `OrthographicProjection` という struct への
**部分書き**で、名前を挙げなかった `near`・`far`・`viewport_origin`・`scaling_mode`・`area` はそのまま残る。
テスト（`tests/camera.rs` の `a_script_zooms_by_writing_scale_into_the_current_variant`）で、書いた後の
`far` が 1000.0、`scaling_mode` が `WindowSize` のままであることを確かめた。

### 読み

`cam[:Projection]` は次の形で返る（`reflect_to_ruby` の `VariantType::Tuple` の枝）:

```ruby
{ Orthographic: [ { near: -1000.0, far: 1000.0, viewport_origin: [0.5, 0.5],
                    scaling_mode: :WindowSize, scale: 1.0,
                    area: { min: [...], max: [...] } } ] }
```

- 鍵 `:Orthographic` は **Symbol**、値は長さ 1 の Array（tuple 変種のフィールドは 1 つ）。
- `viewport_origin` は `glam::Vec2` なので Array（`AS_ARRAY`、`src/reflect.rs:50`）。`area` は `Rect` で、
  `min` / `max` の 2 つの Vec2 を持つ Hash。
- `scaling_mode` は `ScalingMode` という別の enum で、既定の `WindowSize` は unit 変種なので **`:WindowSize`**
  という Symbol 1 つ。enum の入れ子が 2 段あっても規則は同じ。

**読みの形と書きの形は 1 箇所だけ違う。** 読みは `{Orthographic: [ {...} ]}`、書きも同じ形で通るので
「読んで、1 つ書き換えて、書き戻す」は素直に書ける。違うのは、読んだ Hash をそのまま書き戻しても
`area` まで含めて全部書くことになるので、部分書きの利点を使いたければ `{ Orthographic: [ { scale: x } ] }` と
最小の形を自分で組むこと。

### 通らない形（3 つ、全部テストにした）

1. **変種の切り替えは拒まれる。** `cam[:Projection] = { Perspective: [ { fov: 1.0 } ] }` は
   `Orthographic` のときには効かない。`cam[:Projection] = :Perspective` も効かない。これは
   `src/reflect.rs` の `apply_enum` の意図した規則で、bevy は `try_apply` に渡した値から新しい変種を丸ごと
   組み立てるため、全フィールドが揃っていて型も合っている必要があり、Ruby の Hash は型について何も言わない。
   ログに warn が出て、その書きだけが飛ぶ。**同じフレームの他の書きは通る**（テストで確認）。
2. **tuple 変種のフィールドを名前では書けない。** `{ Orthographic: { "0" => { scale: 4.0 } } }` は通らない。
   `apply_enum` の Hash の枝は `Enum::field_mut(name)` を呼ぶが、bevy_reflect の tuple 変種のフィールドには
   名前が無く（`"0"` も名前ではない）、`no such field` で飛ぶ。**tuple 変種は Array でしか書けない。**
   これは最初「どちらでも書けるだろう」と思って書いたテストが落ちて分かった（当初の名前
   `the_variants_field_can_also_be_named_by_its_index` を `a_tuple_variants_field_cannot_be_named` に直した）。
   `docs/host-api.md` の表は読みについて「enum, tuple or struct variant → a one-entry Hash」としか書いておらず、
   書きの側のこの区別（struct 変種なら名前の Hash、tuple 変種なら Array）は文書に無い。
3. **同じ tick の中で 2 回ズームすると 1 回ぶんしか効かない。** これは今回の発見ではなく既存の規則
   （`tests/components.rs::a_read_after_a_write_in_the_same_tick_is_still_the_old_value`）の帰結で、
   書きはフレームの終わりに落ちるので、`scale` を読んで掛けて書く、を 1 tick に 2 回やると 2 回目の読みが
   古い値を返す。R9 の層は**倍率を Ruby 側の ivar に持って絶対値で書く**べき、ということ。

### 2D と 3D の違い

`tests/camera.rs::a_perspective_camera_is_the_same_write_with_another_variant` で確かめた。

| | 2D | 3D |
|---|---|---|
| 目印の component | `Camera2d`（unit struct、読みは `{}`） | `Camera3d`（`{depth_load_op: {Clear: [0.0]}, depth_texture_usages: [...]}`） |
| `Projection` の既定の変種 | `Orthographic`（`Camera2d` の `#[require]` が `default_2d()` を入れる） | `Perspective`（`Projection::default()`、`projection.rs:274-278`） |
| 変種の中身の鍵 | `near`(-1000.0) `far`(1000.0) `viewport_origin` `scaling_mode` `scale`(1.0) `area` | `aspect_ratio` `far` `fov` `near` `near_clip_plane` |
| **ズームの数** | `scale`。大きくすると**写るものが小さくなる**（`projection.rs` の rustdoc「As scale increases, the apparent size of objects decreases」） | `fov`（ラジアン、既定 π/4）。大きくすると視野が広がる＝写るものが小さくなる |
| パン | `Transform` を動かす。2D では `translation[2]`（z）が描画順なので**触らない**のが安全 | `Transform` を動かす。3 軸とも意味がある |

書き方（Hash の 3 段）はまったく同じで、変種の名前とフィールドの名前だけが違う。
`Camera3d` の `depth_load_op` が `{Clear: [0.0]}` と読めることも確かめた（tuple 変種が 1 段下でも同じ形）。

## R9（`Rubevy::Camera`）の形の提案

R0 の結果から言えること。決めていないので、着手前に著者の判断が要るところは印を付けた。

1. **層は「どの変種か」を最初の 1 回だけ読んで覚えてよい。** 変種は Ruby からは切り替えられない（上の 1）ので、
   `@variant`（`:Orthographic` / `:Perspective`）を最初の読みで決めて持ち回れば、以後のズームは
   「読み 1 + 書き 1」で済む。ホストが Rust 側で切り替える可能性はあるので、`reload` 相当の口は残す。
2. **`zoom` は変種ごとに別の数を触る。** 素直なのは `@variant` で分ける表
   （`Orthographic → :scale`、`Perspective → :fov`）を層に持ち、`zoom(factor)` は「見かけの大きさ」を基準に
   `scale /= factor`（`fov` も近似的に `/= factor`）と決めること。**向き（factor > 1 が寄るのか引くのか）は
   決めの問題なので著者に確認したい。** 数は層の中の表なので、共通 CLAUDE.md の「数は利用者が変えられる場所に」
   に照らしても定数ではなく引数・属性で出す。
3. **倍率は Ruby 側に持つ。** 上の 3 の理由で、`@scale` を ivar に持って絶対値を書く。`scale` の getter は
   ivar を返し、`sync` したいときだけ読み直す。これが無いと 1 tick に 2 回 `zoom` を呼んだゲームが黙って壊れる。
4. **`pan` は `translation[0]`/`[1]` だけを書く。** 2D の z を残すため、読んだ `Transform` の
   `translation` を丸ごと書き戻すのではなく `{ translation: [x, y, z_のまま] }` を組む。
5. **`world_at` は層からは `Rubevy.ask("camera.world_at", x, y)` を投げるだけ**（計画 3.9 のとおり）。
   **答える人がいないときの振る舞いは未決。** 今の rubevy では、誰も `answer` しない `ask` のキューには
   何も来ないので `pop` したタスクは**永久に park する**。案は 3 つ:
   (a) 層は投げっぱなしにして、待つかどうかは呼ぶ側に任せる（`Rubevy::Camera#world_at_queue`）。
   (b) 層が `Rubevy.ask` の前に「答え手がいるか」を聞ける口を rubevy に足す（R8 の resource か、
       `answer_in_tick` の登録名の一覧を返す問い）。
   (c) 層は待つが、ホストが答えなかったときに nil を返す仕掛けを rubevy 側に足す（タイムアウトはフレーム数
       — つまり根拠の要る数になるので、既定値を決める材料が要る）。
   **(a) が今の rubevy のままでできる唯一の案**なので R9 は (a) で書き始め、(b)/(c) は別に出すのが良いと思う。
6. **層は `Camera2d` と `Camera3d` の両方を `find` する。** `Rubevy::Camera.find` は
   `Rubevy.find(:Camera2d)` → 空なら `Rubevy.find(:Camera3d)` の順で探し、見つかった方で `@kind` を決める。
   どちらも複数ありうるので、`find_all` も要る。

## 変えたもの

- `Cargo.toml` の `[dev-dependencies]` の bevy に **`bevy_camera` feature** を追加。
  `[dependencies]` は 1 文字も変えていない（計画の指示どおり）。この feature は bevy_internal 0.19.1 の
  `Cargo.toml:79-83` で `bevy_mesh` と `bevy_window` を連れてくる（`bevy_window` は元から入っていた）。
  実際にビルドされるようになったのは `bevy_camera` / `bevy_mesh` / `bevy_image` の 3 つ。
  `Cargo.lock` には `bevy_gizmos` / `bevy_gizmos_macros` の行も増えるが、
  `cargo tree -i bevy_gizmos --target all` が「nothing to print」と言うとおり**どのビルドにも入っていない**
  （解決の表に載っただけ）。テストのビルドは 1 分。
- `tests/camera.rs`（新規、6 テスト）。
- `examples/camera_from_ruby.rs`（新規）。ヘッドレス、型は自分で登録。
  **スクリプトはアセットから読まず、`sabiruby-compiler`（dev-dependency）でその場でコンパイルする。**
  `assets/scripts/*.mrb` は `tools/compile_scripts.sh` が Docker の mruby で作る生成物で、
  今回のために 1 本足すと `prelude.mrb` まで含めて全部を作り直すことになる（共通 CLAUDE.md は
  Docker Desktop を起動しないと定めている。今回は既に動いていたが、生成物を触らない方を選んだ）。
  example の中にスクリプトが書いてある方が、出力と並べて読めるという利点もある。

## 確認

同じ機械（13th Gen Intel Core i7-13700、WSL2、rustc 1.97.0、debug ビルド）。R0 は計測をしない段階なので数値は取っていない。

- `cargo test --test camera` → 6 passed。
- `cargo test`（全部）→ 各テストバイナリの合計で 70 passed, 0 failed, 2 ignored（`tests/read_cost.rs` は
  元から ignore）、doc-test 11 passed。追加前の 64 に今回の 6 が乗った数。
  落ちたものは無い。計画 5 章が警告している `tests/child_task.rs`・
  `tests/pause.rs` の揺れは今回は出ていない。
- `cargo clippy --all-targets` → 新しい 2 ファイルからの警告は 0。既存の警告 2 件（lib の 1 件と
  `examples/headless.rs:47` の `type_complexity`）はそのまま。
- `cargo build --target wasm32-unknown-unknown --lib` → 通る。`tests/no_wasm_unsupported.rs` は 2 passed。
  `src/` を触っていないので当然だが、dev-dependency の feature 追加が lib のビルドに影響しないことの確認になる。
- `cargo run --example camera_from_ruby` の出力:

```
[script] camera: #<Rubevy::Entity 32v0>, components Camera2d, Projection, Transform
[script] projection: Orthographic 1.0x
[script] now at 120.0, -40.0 at 2.0x
WARN rubevy: Projection was not written whole: : the fields of Perspective cannot be written while the value is Orthographic
[script] still Orthographic
host: the camera is at Vec3(120.0, -40.0, 0.0) at 2x
host: script on 33v0 ended: Finished nil
```

## 試して捨てたもの

- 最初、example もほかの example に倣って `assets/scripts/camera.rb` + `.mrb` を置こうとした。
  `.mrb` を作るのが Docker の mruby だけで、`tools/compile_scripts.sh` が `src/prelude.mrb` まで含めて
  全部を作り直す作りだったので、生成物に差分を出さない方を選んで in-process コンパイルにした（上記）。
- `{ Orthographic: { "0" => { scale: 4.0 } } }` が通るという仮定でテストを 1 本書いたが落ちた。
  理由は上の「通らない形」の 2。消さずに「通らないこと」を主張するテストとして残した。
- `OrthographicProjection::default_2d()` の `near` を 0.0 だと思って書いたテストが落ちた。
  実際は **-1000.0**（`projection.rs:771-776`。`default_3d()` の 0.0 を上書きしている。2D ではカメラの
  後ろにあるものも描くため）。読みの表はこの実測値に直した。
- `PerspectiveProjection` のフィールドを rustdoc の説明から `clip_from_view_offset` だと推測して書いたテストが
  落ちた。実際は `near_clip_plane`（`projection.rs:333`）。**推測をテストに書かない**という当たり前のことを
  2 回やった。

## 気づいた点（仕事の範囲の外。直していない）

1. **enum の書きが失敗したときの warn がコロンで始まる。** 何を: `rubevy: Projection was not written whole:
   : the fields of Perspective cannot be written while the value is Orthographic` の「`: :`」。
   どこで: `src/reflect.rs:460-462`（および `:449`、`:456`、`:486`、`:489`）が `format!("{path}: ...")` と
   書いていて、component の直下では `path` が空文字列。なぜ気になるか: 読み手が「何かの名前が抜けた」と思う。
   どこに属するか: rubevy の小さな表示のバグ。再現手順: `cargo run --example camera_from_ruby`（上の出力の
   WARN の行）。直すなら `join` と同じく「`path` が空なら接頭辞を付けない」1 行。
2. **書きが拒まれたことがスクリプトからは分からない。** 何を: `cam[:Projection] = {Perspective: ...}` は
   ホスト側に warn を出すだけで、Ruby 側には何も返らない（`Entity#set` は渡された値をそのまま返す。
   `src/prelude.rb:40-43`）。どこで: `src/lib.rs:2124-2127` の `warn!`。なぜ気になるか: R9 のカメラ層で
   `zoom` が黙って効かない場合が出る（変種が想定と違うとき）。どこに属するか: rubevy の口の設計。
   足すとしたら「最後の書きの問題」を読める口か、`Rubevy.ask` 経由の同期の書き。R9 の前に著者判断が要ると思う。
3. **`docs/host-api.md` の書きの説明に tuple 変種の規則が無い。** 何を: `docs/host-api.md:428-436` は
   「A Symbol switches an enum to that variant, as long as the variant has no fields」とあり、
   変種のフィールドを書く形が **struct 変種なら名前の Hash、tuple 変種なら Array** で、後者は名前では
   書けないことが書かれていない。読みの表（`:422-423`）は両方を「a one-entry Hash」と一括りにしている。
   なぜ気になるか: 今回いちばん時間を使ったのがここ。外の利用者は同じところで詰まる。
   どこに属するか: 文書と実物のずれ。R9 か R8 のついでに 1 文足すのが自然。
4. **`docs/host-api.md:406-408` の「`DefaultPlugins` does it」が、同じ段落の括弧書きと噛み合っていない。**
   何を: bevy 0.19.1 では `bevy_camera` にも `bevy_transform` にも `register_type` が 1 つも無い（grep で確認）ので、
   登録しているのはプラグインではなく `reflect_auto_register` feature（`bevy-0.19.1/Cargo.toml:2749-2754` の
   `default_app` にある）。括弧書きは正しいが、本文の「`DefaultPlugins` does it」は違う仕組みを指している。
   なぜ気になるか: 「`MinimalPlugins` だから登録されない」ではなく「feature を切っているから登録されない」なので、
   ゲーム側が `MinimalPlugins` のまま既定 feature で組んだときに話が合わなくなる。
   実際にどちらが効いているかは**未確認**（feature を入れたビルドを試していない）。
   どこに属するか: 文書と実物のずれ（小）。
5. **リポジトリに rustfmt の設定が無く、`cargo fmt --check` が既存のコードで落ちる。** 何を: `rustfmt.toml` /
   `.rustfmt.toml` がどこにも無く（`find` で確認）、既定の rustfmt は `examples/async.rs:31` などを今と違う形に
   整形しようとする。なぜ気になるか: 新しいファイルを足す担当が `cargo fmt` を走らせると、関係のない差分が
   大量に出る。今回は走らせず、周りと同じ幅（100 桁）に手で揃えた。どこに属するか: repo の作法。
   設定を 1 つ置くか「fmt は使わない」と書くかは著者の判断。
