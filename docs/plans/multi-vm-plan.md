# VM を複数持つ（実装指示書・案）

作成 2026-09-17。著者の要望「rubevy はサンプルゲームだけでなく色々な使い方を想定したいので、VM を複数起動することもサポートしたい」。
調査は implementer が行い（コード変更なし。rubevy `ec6f0ce`、sabiruby 0.5.1、実測つき）、本体が計画に起こした。**着手は著者の判断待ち**（下の「決めること」1 が先）。

## 結論

できる。土台はほぼ揃っている。sabiruby の `Vm` は完全に自己完結（ヒープ・シンボル・irep・スタック・スケジューラ・ホスト状態を所有、`src/` にグローバル可変状態 0、
`Vm: Send + Sync`）。実測: 64 本の VM が互いに完全に独立、4 スレッド × 各自の VM の同時実行 OK。生成 0.6 ms/本（ほぼ mrblib の読み込み）、常駐 0.5 MB/本、
何もしない VM を 1 フレーム回す 5 ns/本。**本数を増やしてもフレーム時間はほぼ増えず、効くのはメモリと起動時間。**

rubevy 側の「1 VM 前提」は 6 か所だけ（本質は 1 つ = `ScriptWorld` が `Resource`）:
`ScriptWorld: Resource`（`lib.rs:467`）／`stop_removed_task` が型でリソースを引く（144-153）／`ScriptTask { task: ObjId }` にどの VM かが無い（**別 VM に渡すと黙って別物を指す**、131-135）／
`Script` に VM の指定が無い（104）／システム群が `ResMut<ScriptWorld>` 固定（1130/1164/1173/1260/1283、排他 2 本 1355/1463）／`RubevySet` が 1 組（1065）。
README・`lib.rs:46`・`outlook.md:100` に「2 本目の VM はまだ作っていない」と明記されている＝予定済み・未着手の項目。

## 設計案

| | A. VM をコンポーネントに | **B. 型マーカーで generic** | C. 1 リソースに N 本 |
|---|---|---|---|
| 形 | `ScriptWorld` を Component に | `ScriptWorld<M>`/`Script<M>`/`ScriptTask<M>`/`RubevySet<M>`/`RubevyPlugin<M>`、既定 `M = ()` | `Vec<VmSlot>` + `VmId` |
| 本数が実行時に決まる | ◎ | ✕（コンパイル時） | ◎ |
| ObjId 取り違えの防止 | △ 実行時 | **◎ コンパイルエラー** | △ 実行時 |
| 既存 API の互換 | ✕ | **◎ 既定型引数で 1 行も壊れない** | △ アクセサ化 |
| VM 間の並列 | ○ | **◎ 別リソースなので Bevy が並列化** | ✕ |

**推奨: B を土台に、実行時生成が本当に要るなら後から C を重ねる。** 想定用途（mod/ユーザースクリプトの隔離、ツール用・UI 用・デバッグコンソール用 VM）は本数がコンパイル時に決まる。
隔離は `RubevyPlugin::<Mods>::with_asset_root("assets/mods")` + 小さい `budget` の 2 行で書け（`Host`・ロードパス・予算・`frame_time` は既に VM ごと）、`RubevySet<Mods>::Answer` が独立するので
「mod には `ask` の答えを一部しか返さない」も書ける。

## 段階

| 段階 | 内容 | 状態 |
|---|---|---|
| 0 | 互換のまま: `ScriptTask`/`Script`/`Request`/`ScriptEnded` に「どの VM か」を持たせる下準備（1 VM なら意味は変わらない） | 未着手 |
| 1 | `ScriptWorld<M>` 化（既定 `M = ()`）、`RubevyPlugin::<M>`。テストで 2 本立て、グローバル・定数・例外・GC が混ざらないことを確認 | 未着手 |
| 2 | `publish` の宛先、`stop_removed_task` の型付け、`answer_components`/`apply_component_writes` を VM ごとに | 未着手 |
| 3 | `examples/two_vms.rs`（本体 VM + mod 用 VM: mod から本体のクラスが見えない、mod の `loop{}` が本体のフレームを食わない、mod が `require` で本体のファイルを読めない）と docs | 未着手 |
| 4 | 実行時生成（A か C）— 用途が出てから | 未着手 |

## 決めること（著者）

1. **本数はコンパイル時に決まるか、実行時か**（接続したプレイヤーごと・読み込んだ mod ごとに 1 本、なら実行時 → 段階 4 が要る）。これが先。
2. 予算の合算: `frame_time` が VM ごとだと N 本で最悪 N×8 ms。フレーム全体の上限を足すか、VM ごとの取り分か。
3. `publish` の既定: 全 VM に配るか、宛先 VM を指定するか。エンティティ指定との組み合わせ。
4. `Rubevy.ask` の答え手: VM ごと（B で自然）か、1 本で全 VM 分か（C で自然）。
5. 同じ `.mrb` を 2 本に載せると irep も 2 倍。共有するか割り切るか。
6. sabiruby 側に要るものは今のところ無し。VM をまたいで値を写すヘルパは `sabiruby-serde` で書ける。

## 影響範囲（参考）

rubevy 内 `ScriptWorld` 71 行・`self.vm`/`world.vm` 48 行・examples/tests 16 ファイル。rubevy_games 側 `ScriptWorld` 39 行・`ScriptTask` 29 行・`.vm` 直接参照 13 行（5 ファイル）。
B なら既定型引数のおかげでこれらは無変更で通る見込み。
