# 2026-09-20 resource を名前で読む口（R8）

計画 `docs/plans/generalize-plan.md` の段階 R8。到達点は `Rubevy.resource(:Name)` と書き、
R0 が見つけた 3 つ（enum の warn のコロン、`host-api.md` の tuple 変種、`DefaultPlugins does it`）の手当て。
ブランチ `generalize`、R0 `bf45bde` と R6 `cb8bf00` の上。

結論から: **resource の道は component の道の「エンティティを抜いたもの」で、新しい概念はほとんど入らなかった。**
入ったのは 2 つだけ — bevy 0.19 で resource がどこに居るかの引き方と、`#[reflect(Resource)]` を「resource だと
名乗っている」という 1 ビットとして使う判断。以下はその 2 つと、測って分かったこと、捨てた案。

## bevy 0.19.1 で resource がどこに居るか（ソースで確かめた）

最初は component と同じく `ReflectResource` に `reflect(&World)` のような関数が生えていると思っていた。違った。

`bevy_ecs-0.19.1/src/reflect/resource.rs:29-31` の `ReflectResource` は**フィールドも関数も持たない空の struct**で、
rustdoc がこう書いている: 「This struct does not provide any functionality. It implies the existence of a
reflected Component of the same type, **which is meant to be used instead**.」
`FromType::insert_dependencies`（同 `:37-39`）が `#[reflect(Resource)]` の型に `ReflectComponent` を自動で足す。

理由は `bevy_ecs-0.19.1/src/resource.rs:87` の 1 行、`pub trait Resource: Component {}`。
**0.19 の resource は component で、専用のエンティティに載っている。** どのエンティティかは
`World::resource_entities()`（`world/mod.rs:264`、`pub`）が `ComponentId → Entity` で持っている。
`ComponentId` は `World::components().get_valid_id(type_id)`（`component/info.rs:578`）。
`get_valid_resource_id` / `get_resource_id` もあるが 0.19.0 で **deprecated**（`:608`、`:686`。
「use get_valid_id」）なので使わなかった。

なので読みの道は

```
名前 → registry（短い型パス、なければ全パス）
     → ReflectResource があるか（= resource を名乗っているか）
     → TypeId → ComponentId → resource_entities → Entity
     → ReflectComponent::reflect(EntityRef) → reflect_to_ruby
```

で、最後の 2 段は component と 1 文字も違わない。書きも同じで、最後が `reflect_mut` + `apply_ruby`。
`unsafe` は 1 つも要らない（`World::get_resource_by_id` + `ReflectFromPtr` の道なら要ったはずで、
それは採らなかった。下の「捨てた案」）。

## `#[reflect(Resource)]` を何に使うか

`ReflectResource` が何もしないなら、無視して `ReflectComponent` だけで書いても動く。実際
`Rubevy.resource(:Transform)` は、`Transform` の `ComponentId` で `resource_entities` を引けば
`None` が返るので、無視しても結果は nil になる。

それでも**「resource を名乗っているか」を条件にした**。理由は 2 つ。

1. 口の意味がはっきりする。`Rubevy.resource(:X)` が nil を返したとき、「X は resource ではない」と
   「X という resource はまだ世界に無い」が両方 nil なのは変わらないが、**ホストが `#[reflect(Resource)]` を
   書き忘れた型は必ず nil** という規則にできる。偶然 resource 用エンティティに載っていた component を
   読んでしまう道が消える。
2. 文書に書けることが 1 行になる（「`#[derive(Resource, Reflect)]` + `#[reflect(Resource)]` + `register_type`」）。
   component の節がちょうど同じ 3 つを言っているので、並べて読める。

代償は、`#[reflect(Component)]` だけ書いた resource が見えないこと。0.19 では `#[reflect(Resource)]` が
`ReflectComponent` を連れてくる（逆は連れてこない）ので、素直に書いた型は必ず両方持つ。
`docs/host-api.md` の新しい節にこの規則を書いた。

## Ruby 側の形をどちらにするか

計画は「`Rubevy.set_resource(:Name, hash)` か、`Rubevy.resource(:Name)` が返すものに `[]=` を持たせるか、
`Entity#[]=` の今の形と `src/prelude.rb` の流儀に合わせて小さい方を選ぶ」と言っている。**`set_resource` を採った。**

`Entity#[]=` が小さいのは、`Entity` という**受け手のオブジェクトが先にある**からで、`[]=` はそこに生える。
resource 側には受け手が無い。`Rubevy.resource(:Score)[:points] = 1.0` を成立させるには、読みが返す Hash を
「どの resource から来たか」を覚えている別のクラスにして `[]=` を捕まえる必要があり、

- 読みの戻り値が素の Hash でなくなる（component の読みと形が違ってしまう）。`reflect_to_ruby` は
  入れ子の struct も Hash で返すので、どの階層を特別扱いするかという問題も出る。
- 部分書きの規則（「名前を挙げたフィールドだけ」）が `[]=` 1 回ごとのコマンドになり、
  1 tick に 2 回書くと 2 コマンドになる。今の component の書きは「Hash を 1 回渡す = 1 コマンド」。
- prelude に新しいクラスが 1 つ増える。

3 つとも「component と同じ」から離れる方向なので採らなかった。`Rubevy.set_resource(name, hash)` は
`Rubevy.set_component(entity, name, hash)` の引数を 1 本減らしたものそのもので、規則の説明が要らない。

**置き場所**は prelude と native で分かれた。これは既存の流儀そのまま:

- 読み（`Rubevy.resource`）は**答えを待つ**ので `src/prelude.rb`。native は park できない
  （`blocking pop cannot be called from within a C function boundary`）。`Rubevy.find` と同じ。
- 書き（`Rubevy.set_resource`）は待たないので **native**（`install_host_api`）。`Rubevy.set_component` と同じ。

native 側は Symbol を `vm.sym_name` で受ける枝を先に置いた（`subscribe` と同じ形）。prelude 経由の読みは
`name.to_s` を通るので、どちらの書き方でも同じ名前になる。

## 名前の決め方 — ジェネリックで 1 つ測った

component と同じ `get_with_short_type_path` → `get_with_type_path` にした。衝突時の挙動も component と同じ
（bevy_reflect の `TypeRegistry` が、短い名前が 2 つの型に当たると `ambiguous_names` に入れて短い名前からは
引けなくする。`type_registry.rs:312-322`）。

ジェネリックは **`TypePath::short_type_path` が型引数ごと短くしたもの**（`type_path.rs:94-99`:
「For `Option<Vec<usize>>`, this is `"Option<Vec<usize>>"`」）なので、`Time<Virtual>` はそのままの綴りで引ける。
**ここで 1 つ間違えた**: 「`Time` は既定の型引数が `()` だから `Time` で引けるだろう」と思ってテストを書いたら落ちた。

```
assertion `left == right` failed: and so is `Time`
  left: Some("NilClass")   right: Some("Hash")
```

既定の型引数も**書き出される**ので `Time` は `Time<()>` で、`Rubevy.resource(:Time)` は nil、
`Rubevy.resource("Time<()>")` が当たる。テストは落ちた側を残して「`Time<()>` は当たり、`Time` は当たらない」を
主張する形に直した（`tests/resources.rs::a_generic_resource_is_named_with_its_parameters`）。
文書（`docs/host-api.md` の新しい節）にも 1 行書いた。Ruby 側では `<` を含む名前は引用符が要る
（`"Time<Virtual>"` か `:"Time<Virtual>"`）ことも。

**できないこと**として文書に残したのはこれだけ。型引数の一部だけを書く、`Time` で「どれでもいい」を表す、
といった省略は無い。rubevy が名前を組み立てていないので、足すなら rubevy が bevy の綴りを知ることになり、
それは component の側でもやっていない。

## 「同じ tick で答える」を守るための置き場所

読みは `RESERVED_KINDS` に `"resource.get"` を足して `answer_reflect_requests` の枝にした（4 → 5 本）。
`answer_reflect_requests` は `&World` しか取らないので、`resource_entities()` も `components()` も
そのまま引ける。フレームを食わないこと、tick を跨いだ持ち越し（予算切れ）が component と同じ道を通ること、
ゲームの `take_requests` に漏れないことは、どれも既存の仕掛けにそのまま乗った
（`tests/resources.rs::a_resource_question_never_reaches_the_game`）。

書きは **`apply_resource_writes` という別のシステム**にして、`apply_component_writes` の直後に繋いだ。
`apply_component_writes` の中にもう 1 ループ足す案と迷ったが、

- 名前が嘘になる（`apply_component_writes` が resource も書く）。この関数名は `docs/host-api.md` と
  rustdoc の 4 か所から参照されている。
- 2 つは「対象をどこから見つけるか」が違う（スクリプトが名指ししたエンティティ / 世界が持っているエンティティ）。
  同じループに入れるとその分岐が中に入る。

ので別にした。どちらも排他システムなので、`RubevySet::Tick` の鎖が 3 本から 4 本になっただけ。
公開 API は増えていない（どちらも private な fn）。

`ScriptWorld<M>` には `resource_writes` と `resource_cache` を足した。キャッシュを component と分けたのは、
resource の側は `ReflectResource` を確かめてからでないと入れたくないのと、持つものが
`(TypeId, ReflectComponent)` の組で型が違うため。書きの 2 システムはどちらも `&mut World` を持っていて
`ScriptWorld` を同時に借りられないので、キャッシュを使わず `resource_data()`（registry だけを見る半分）を呼ぶ。
これは `apply_component_writes` が前からやっていることと同じ。

## R0 が見つけた 3 つ

### (a) enum の warn がコロンで始まる

`src/reflect.rs` が問題を `format!("{path}: ...")` で組んでいて、component / resource の直下では `path` が
空文字列だったので `not written whole: : the fields of …` になっていた。`join` が既に持っている
「空なら接頭辞を付けない」規則を `at(path, what)` として足し、`{path}:` で始まっていた 18 か所を全部それに替えた
（`{here}:` の側は `join` を通っているので空にならず、触っていない）。`{e}` の 1 か所だけ
`at(path, e)` と Display をそのまま渡す形。

直った証拠（`cargo run --example camera_from_ruby` の同じ行、R0 の worklog の出力と比べられる）:

```
WARN rubevy: Projection was not written whole: the fields of Perspective cannot be written while the value is Orthographic
```

### (b) tuple 変種の書きの規則が文書に無い

`docs/host-api.md` の読みの表の「enum, tuple or struct variant」を 2 行に割り、
書きの説明に「struct 変種は名前の Hash、tuple 変種は Array、tuple 変種のフィールドは名前では書けない」を
R0 の実例（`{ Orthographic: [ { scale: 2.5 } ] }`）つきで足した。`src/reflect.rs` のモジュール rustdoc にも
同じ表と同じ 1 文がある（実物の隣に置いておきたいので両方直した）。

### (c) 「`DefaultPlugins` does it」を確かめた

R0 は「`bevy_camera` にも `bevy_transform` にも `register_type` は無い。どちらが効いているかは未確認」で
止まっていた。確かめた結果:

| 見たもの | 結果 |
|---|---|
| `register_type` の数（`src/` を grep、bevy 0.19.1） | `bevy_transform` 0、`bevy_camera` 0、`bevy_sprite` 0、`bevy_ui` 0、`bevy_input` 0、`bevy_window` 0、`bevy_pbr` 4、`bevy_time` 4 |
| `bevy_app-0.19.1/src/app.rs:113-120` | `reflect_auto_register` が**有る**と `AppTypeRegistry::new_with_derived_types()`、無いと `init_resource::<AppTypeRegistry>()`（空） |
| `bevy-0.19.1/Cargo.toml:2746-2754` | `default_app` に `reflect_auto_register`。`2d` / `3d` / `ui` が `default_app` を引き、それらが `default` |
| `bevy_time-0.19.1/src/lib.rs:76-79` | `TimePlugin` は今も手で `register_type::<Time<Virtual>>()` などを呼ぶ |

つまり **「`DefaultPlugins` does it」は違う**。効いているのは `reflect_auto_register` feature で、
それが bevy の既定 feature に入っているから既定の構成のゲームでは登録済みに見える。rubevy は
`default-features = false` なので切れている。`register_type` を今も持つプラグインは例外的に残っている。

**この区別を推測で書かずに済むように、テストで 1 つ確かめた**:
`tests/resources.rs::a_generic_resource_is_named_with_its_parameters` は `MinimalPlugins` のもとで
`Time<Virtual>` を**どこにも `register_type` を書かずに**読む。これが通るのは `TimePlugin` が手で登録して
いるからで、`reflect_auto_register` が効いているなら `Transform` も `Camera2d` も読めるはず（読めない。
`tests/components.rs` は `Transform` を自分で登録している）。文書にはこの 2 つを根拠として書いた。

## 測ったこと — 読み 1 回の費用

`tests/read_cost.rs` の流儀（`#[ignore]`、assert しない、機械の数字だと断る）で
`a_resource_read_beside_a_component_read` を足した。component の読みと resource の読みを**同じ run の中で
続けて**測るのは、差が機械の速さではなく道の差だと言えるようにするため。値の形も揃えた
（どちらも `f32` を 2 つ持つ struct）。

測る前に `pgrep -af "cargo|rustc"` で 0 件（並行実行なし）を確かめた。release ビルド、同じ機械、3 回。

| | 開始したフレームの中で読めた回数 | 1 回あたり（最長フレーム ÷ 回数） |
|---|---|---|
| component（`e[:Hp]`） | 2667、2667、2667 | 2.452 / 2.380 / 2.416 µs |
| resource（`Rubevy.resource(:Score)`） | 2899、2899、2899 | 2.294 / 2.235 / 2.661 µs |

**回数は 3 回とも 1 命令も動かない。** これは、この条件で読みを止めているのが時計ではなく
**命令の予算**だから（同じファイルの `one_task_that_does_nothing_but_read` が、`frame_time` を外しても
2667 のままだと前から言っている）。200,000 ÷ 2667 = **75.0 命令/読み**、200,000 ÷ 2899 = **69.0 命令/読み**。
差の 6 命令は Ruby 側の frame 1 つぶんで、`e[:Hp]` が `Entity#[]` → `Entity#get` → `Rubevy.ask` と
2 段送るのに対し `Rubevy.resource` は `Rubevy.ask` を直接呼ぶ、その差。

**つまりこの計測器が解像できる差は Ruby の側にある。** 時間の方（2.2〜2.7 µs）は 3 回のぶれが
component で 0.07 µs、resource で 0.43 µs あり、2 つの差（0.1〜0.2 µs）はぶれの中に入っている。
ホスト側で resource が余計にやっているのは `Components::get_valid_id`（`TypeId` の HashMap）と
`ResourceEntities::get`（`SparseArray` の添字）の 2 回だけなので、**「component の読みと同じ桁で、
差は測れない」**が今言えることの全部。R5 で `FrameStats` が入れば、答えループの周回数から
もう少し細かく言えるはず。

## 捨てた案・試して駄目だったもの

- **`World::get_resource_by_id` + `ReflectFromPtr`**。`Ptr` から `&dyn Reflect` を作る所が `unsafe` で、
  共通 CLAUDE.md が禁じている（rubevy は「なるべく避ける」側だが、`ReflectComponent` で同じことが
  安全にできる以上「他に書き方が無い場合」に当たらない）。採らなかった。
- **`world.resource_entities().iter()` を走査して型を突き合わせる**。`ResourceEntities::iter` の rustdoc
  （`resource.rs:94-98`）自身が「component の配列を全部走査するので、resource が少なくても遅いことがある」と
  言っている。`ComponentId` 1 つで引けるのにこれを毎回やる理由が無い。
- **`Rubevy.resource(:Name)[]=`**（上の「Ruby 側の形」）。
- **`Rubevy.resource` を native にする**。答えを待つので park が要り、native では park できない。
- **`Time` で `Time<()>` を引けるようにする**（別名を足す）。rubevy が bevy の綴りを知ることになるので見送り、
  文書に「既定の型引数も書き出される」と書くだけにした。

## 確認

同じ機械（13th Gen Intel Core i7-13700、WSL2、debug ビルド）。着手前 101 passed。

- `cargo test --workspace` → **109 passed, 0 failed**（`tests/resources.rs` の 8 本が増えた）。
- `cargo clippy --workspace --all-targets` → 警告 2 件、**どちらも既存**（`src/lib.rs:1940` の
  `collapsible_if`（`drain_commands` の `SetPosition` の枝、今回触っていない）と
  `examples/headless.rs` の `type_complexity`）。新しい警告 0。
- `cargo build --target wasm32-unknown-unknown --lib` → 通る。
- `cargo test --test no_wasm_unsupported` → 2 passed。
- `cargo run --example camera_from_ruby` → warn のコロンが消えた（上の (a)）。
- `cargo test --release --test read_cost -- --ignored --nocapture` → 3 本とも通り、数字は上の表
  （測る前に `pgrep` で並行実行 0 を確認）。
- 依存は 1 つも増えていない（`[dependencies]` も `[dev-dependencies]` も無変更）。公開 API は
  `Rubevy.resource` / `Rubevy.set_resource`（Ruby 側）だけの追加で、Rust 側は 1 つも足していない。
- `src/prelude.mrb` は `tools/compile_scripts.sh` で作り直した（Docker は既に動いていた。**起動していない**）。
  スクリプト全体を回して、差分が出たのは `src/prelude.mrb` 1 つだけ＝生成器は決定的。

## 気づいた点（仕事の範囲の外。直していない）

1. **`Rubevy.set_resource` が拒まれたことも、やはりスクリプトからは分からない。** 何を: 存在しない resource、
   resource でない型、書けないフィールドは、どれもホスト側に `warn!` が出るだけで Ruby には nil が返る。
   どこで: `src/lib.rs` の `apply_resource_writes`。なぜ気になるか: R0 の「気づいた点 2」とまったく同じ問題が
   1 つ増えた（component と resource の 2 か所になった）。**著者判断待ちの「最後の書きの問題を読める口」は、
   もし足すなら component だけでなく resource も見る形が要る。** どこに属するか: rubevy の口の設計。
2. **`resource_cache` は型が登録し直されても古いままになりうる。** 何を: `reflect_cache` と同じく、
   一度当たった名前の `(TypeId, ReflectComponent)` を持ち続ける。`reflect_cache` の rustdoc は
   「登録されていない名前は覚えない」と書いてあるが、**登録された後に別の型で登録し直された**場合は
   どちらのキャッシュも古い。どこで: `src/lib.rs` の `reflect_cache` / `resource_cache`。
   なぜ気になるか: 普通のゲームでは起きないが、ホットリロードや mod のロード順で起きうる。
   どこに属するか: rubevy（既存の性質。resource で増えたわけではない）。見送りでよいと思うが、
   R10 で数を棚卸しするときにキャッシュの寿命も 1 行書く場所があるかもしれない。
3. **resource の読みは `$rubevy` と役割が重なる。** 何を: `$rubevy[:frame]` は rubevy がフレームごとに
   VM へ書き込んでいる Hash で、今回の `Rubevy.resource` があれば「フレーム数を resource にして
   `register_type` する」ホストは同じものを 2 つの道で読めることになる。なぜ気になるか: 重複ではないが、
   R10 で `$rubevy` のキーを棚卸しするときに「どちらを勧めるか」を決める材料になる。
   どこに属するか: 計画書（R10 の範囲）。
4. **`docs/host-api.md` の「Time」の節はまだ `Time<Virtual>` を名前で読めることを知らない。** 何を:
   ゲームがスクリプトに時間を見せるときの選択肢が 1 つ増えたのに、その節は `$rubevy` と `sleep` の話だけ。
   どこで: `docs/host-api.md` の「Time」。なぜ気になるか: R4 がこの節を書き直す予定なので、
   そのときに 1 行足すのが自然。どこに属するか: 文書（R4 のついで）。
5. **`Components::get_valid_resource_id` / `valid_resource_id` / `get_resource_id` は bevy 0.19.0 で
   deprecated。** 何を: resource を扱う古い書き方が bevy 側で畳まれつつある（`resource: Component` に
   なったため）。どこで: `bevy_ecs-0.19.1/src/component/info.rs:608`、`:632`、`:686`。
   なぜ気になるか: 次の bevy で消えると、rubevy 以外（ゲーム側）のコードが当たる可能性がある。
   rubevy 自身は `get_valid_id` を使っているので影響なし。どこに属するか: 本（移植の章）の素材／games への注意。
