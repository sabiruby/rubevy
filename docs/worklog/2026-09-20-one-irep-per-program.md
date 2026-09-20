# 同じ `.mrb` は 1 つの irep（R2）

2026-09-20。計画書 `docs/plans/generalize-plan.md` の段階 R2、ブランチ `generalize`（`c66d220` の上）。
調査は `docs/worklog/2026-09-20-factory-survey.md`（「コードから分かったこと」の最後の 2 行と「実測 2」の 6）。
計測の手順と罠は前の担当の `docs/worklog/2026-09-20-subscription-index.md`。

## 前提の確認

計画書 3.2 は `src/lib.rs:1560-1588` を指しているが、これは調査時点（main `fb4f398`）の行番号で、
R6・R8・R1 が載った着手時点では `start_scripts` は `:1693-1721` に動いていた。中身は計画書の記述どおり:

```rust
for (entity, script) in &pending {
    let Some(asset) = assets.get(&script.source) else { continue };
    let name = script.name.clone().unwrap_or_else(|| format!("{entity}"));
    let vm = &mut world.vm;
    let irep = match vm.load(&asset.bytes) { … };
    match vm.task_spawn(irep, script.priority, Some(&name)) { … }
}
```

エンティティ 1 台につき `vm.load` が 1 回。`Vm::load`（sabiruby 0.5.2 `src/vm.rs:1972-1995`）は
RITE を parse して、その中の irep レコードを 1 つずつ `self.ireps` に **push する**。同じ `.mrb` を
N 台に載せれば同じ命令列が N 部積まれる（調査の実測で 1 台 2.2 kB・2.5 µs）。
`Vm::task_spawn` は `IrepId` を取る（`src/vm.rs:845`）ので、VM 側は 1 つの irep から N 本のタスクを
作れる。VM には手を入れない。

## 先に確かめた 2 つ（確かめてから書いた）

**(1) irep は走っても変わらないか。** 共有する以上、VM が irep に書き込むもの — インラインキャッシュ、
実行回数、デバッグ情報 — があってはいけない。`VmIrep`（`src/vm.rs:19-32`）のフィールドは
`nlocals` / `nregs` / `iseq` / `catch` / `pool` / `syms` / `reps` / `lv` / `lines` / `filename` の 10 個で、
どれもロード時に決まる静的なもの。キャッシュも計数器も無い。
`&mut vm.ireps[..]` を取る場所を `src/` 全体で探すと **1 か所だけ** — `ext_binding.rs:125` の
`merge_lvar`（`mrb_proc_merge_lvar`: `Binding#local_variable_set` が、そのスコープに無かった名前を
ローカル変数表に足す）。ここが触る irep は `wrap_lvspace`（`:42-49`）がその場で `push` した
「ローカル変数の置き場」専用の irep で、`Binding` を作る 4 か所（`:71` `:185` `:215` `:219-220`）は
どれもこれを通る。プログラムの irep ではない。
もう 1 つ、文字列リテラル。`OP_STRING` は `pool` の中身を `s.clone()` してから Ruby の String を作る
（`src/vm.rs:3795`。`OP_SYMBOL` も同じ）ので、`s = "abc"; s << "!"` を 2 本のタスクが走らせても
互いに見えない。これはテストにした（`tasks_that_share_an_irep_do_not_share_its_literals`）。

**(2) 表の鍵は `AssetId` か、内容か。** 計画書は「games の Apply が毎回新しいアセットを足しているかを
見て決める」と言っている。実物を読んだ:

* sabibots `src/main.rs:1644`: `platform::compile(&src, name).map(|bytes| (assets.add(MrbAsset { bytes }), prelude_lines))`。
  コンパイルのたびに `Assets::add` で**新しい `AssetId`**。同じ本文でも別の id。
* garden `src/main.rs:2737`: 同じ形。
* さらに garden は **1 匹ごとに**コンパイルしている。`give_mind`（`:2660-2685`）は
  生まれた個体それぞれについて `compile` か `compile_source` を呼び、その戻りの `Handle` を
  `Script::new(handle)` に渡す。同じ種の 20 匹は「同じ本文・別の `AssetId`」20 個になる。

つまり **`AssetId` を鍵にすると、いちばん台数が出るゲームで共有が 1 回も起きない**。
鍵は内容にする。ただし「内容のハッシュ」ではなく**内容そのもの**を鍵にした
（`HashMap<Box<[u8]>, IrepId>`）。ハッシュだけを鍵にすると、衝突したときに
**別のプログラムのタスクが走る**（静かで、再現しなくて、どこを見ても分からない類のバグ）。
`HashMap` はバケットが当たってもキーを比較するので、この形なら衝突が起こりうるのは比較の時間だけで、
別のプログラムが同じ irep を共有することは**構造上ありえない**。
代償は「違うプログラム 1 本につきそのバイト列 1 部」（調査の実測で 1 本 2.2 kB）で、
これは共有によって消える irep 1 部とほぼ同じ大きさ。

## 作ったもの

`ScriptWorld<M>` に非公開のフィールドを 1 つ:

```rust
programs: std::collections::HashMap<Box<[u8]>, sabiruby::object::IrepId>,
```

と、その引き手:

```rust
fn irep_of(&mut self, bytes: &[u8]) -> Result<sabiruby::object::IrepId, VmError> {
    if let Some(&irep) = self.programs.get(bytes) { return Ok(irep); }
    let irep = self.vm.load(bytes)?;
    self.programs.insert(bytes.into(), irep);
    Ok(irep)
}
```

`start_scripts` は `vm.load(&asset.bytes)` を `world.irep_of(&asset.bytes)` に替えただけ。
`Box<[u8]>` を鍵にすると `get(&[u8])` がそのまま通る（`Box<[u8]>: Borrow<[u8]>`）ので、
引くときにバイト列を複製しない。入れるときだけ 1 部持つ。

公開に足したのは `ScriptWorld::loaded_programs() -> usize` 1 つだけ。
この VM がいくつ別のプログラムを読んだか = スクリプトの起動が VM に残した irep の本数で、
エディタの Apply を繰り返すゲームが見る数。`vm.ireps` は `#[doc(hidden)]` なので、
文書から「これを見ろ」と言える数が要る。既存の名前・意味は 1 つも変えていない。依存も増えていない。

## `AssetEvent` で表から外す口は**作らなかった**

計画書と依頼文は「アセットが差し替わったら表から外す（`AssetEvent::Modified` / `Removed`）」と
言っている。これは鍵が `AssetId` なら**必須**（外さないと古いコードが走る）。
鍵が内容ならどうなるかを詰めると、作らないほうが正しかった:

* **正しさのためには要らない。** 鍵がプログラムそのものなので、変わったプログラムは別の鍵になり、
  必ず外れる（= `vm.load` が走る）。古い irep が新しい本文に対して返ることは起こりようがない。
  「差し替え後に spawn したスクリプトが新しいコードで動く」ことは、`AssetId` を据え置いたまま
  中身だけ差し替える形（`Assets::insert(id, …)`、ホットリロードがやること）でテストにした。
* **外すと損をする。** 外して得られるのは鍵のバイト列 2.2 kB。失うのは「同じ本文が戻ってきたときに
  irep を作り直さない」という性質で、作り直した irep は **VM から消せない**（下）。
  つまり外すのは、良くて中立、悪ければ漏れを倍にする。
* **外すには索引がもう 1 本要る。** `AssetId → 鍵` を別に持たないと、イベントの `AssetId` から
  どのエントリを消すか分からない。正しさに効かない表のために、合わせ続ける表がもう 1 つ増える。

なので R2 は `AssetEvent` を読まない（毎フレームのシステムも 1 つ増えない）。
判断の是非は本体のレビューに委ねる。**これは依頼文の 1 行を実装しなかった箇所なので報告に明記する。**

## 古い irep が消えないこと（直せない。数は測った）

sabiruby 0.5.2 の `src/` に `ireps` から取り除く場所は 1 つも無い（`remove` / `truncate` / `clear` /
`pop` を grep して 0 件。`push` は `vm.rs:1981` と `ext_binding.rs:44` の 2 か所だけ）。
`IrepId` は `Vec` の添字（`object.rs:40` で `pub type IrepId = usize`）なので、追加だけなら既存の id は
ずっと有効で、これは共有の前提でもある。

差し替えを M 回繰り返したときの増え方を `tests/shared_irep.rs` で測った（測って、そのまま assert にした）:

| 差し替え 10 回 | 前（`c66d220`） | 後 |
|---|---|---|
| 毎回**同じ本文**（Apply を押し直す、変更を戻す） | +2 irep/回 | **0**（測って assert にした） |
| 毎回**違う本文** | +2 irep/回 | +2 irep/回（測って assert にした） |

「後」の 2 行は `tests/shared_irep.rs::applying_the_same_text_again_costs_nothing_and_a_new_one_costs_an_irep`
が実際に数えている値をそのまま assert にしたもの。「前」の列は計測器の `ireps` 欄（下の「測ったこと」）
から来ている — 前は台数 N に対して `ireps` がちょうど 2N なので、`vm.load` 1 回が 2 本。
「2」はこのテストのスクリプト（`loop { Rubevy.ask('alive').pop }`）が持つ irep の数 —
トップレベルが 1 つと `loop` のブロックが 1 つ — で、プログラムの形で変わる。
言えるのは「**新しい本文 1 つにつきそのプログラムの irep の数だけ増え、二度と減らない**」こと。
`ScriptWorld::loaded_programs()` がその本数で、rustdoc と `docs/host-api.md` に書いた。

## 測ったこと

計測器は worktree `rubevy-wt-factory-survey` の `examples/factory_machines.rs`（未コミット、調査の担当が
書いたもの）を両方の worktree の `examples/` にコピーして使った。**R2 のコミットには入れていない**
（常設は R5）。条件は調査と同じ — 13th Gen Intel Core i7-13700、WSL2、`--release`、`taskset -c 2`、
sabiruby 0.5.2 / bevy 0.19.1、`factory_machines 90 3`（90 フレーム × 3 回）、メモリは
`factory_machines mem <台数> 60` を台数ごとに別プロセスで。前は `git worktree add --detach
../rubevy-wt-r2-before c66d220`、**target ディレクトリを別にして**建てた
（前の担当が踏んだ罠 1。`md5sum` が `de36f853…` 対 `b424dd78…` で別物であることを確かめてから測った）。
罠 2（並行実行）は、隣の担当の Playwright の Chromium と `cargo run -p garden --headless` が
`pgrep -af "cargo|rustc|docker run|chrome"` に出ている間は測らず、空になってから前・後の順に取った。
実装とテストはその待ち時間に全部済ませてある。

### irep の数（R2 の核）

`ireps` は「スクリプトが起動した後 − 起動する前」で、計測器が `vm.ireps.len()` から取っている。

| 台数 | 前 | 後 |
|---|---|---|
| 10 | 20 | **2** |
| 100 | 200 | **2** |
| 300 | 600 | **2** |
| 1000 | 2000 | **2** |
| 3000 | 6000 | **2** |

前は 1 台につき 2（このスクリプトはトップレベルと `loop` のブロックで 2 つの irep を持つ）。
後は台数に依らず 2。40 行すべてで同じ。

### 起動フレーム（`start_frame`、µs）

計測器は起動のフレームだけ `budget = 0` にするので、この数は `Vm::load` + `task_spawn` そのもの。
同じ台数の 8 行（sleep 2 通り × limits 4 通り）はどれも同じものを測っているので、8 個を 1 つの標本の束として
中央値と最小〜最大を出した。

| 台数 | 前 中央値 (min..max) | 後 中央値 (min..max) | 比 |
|---|---|---|---|
| 10 | 259.2 (104.9..316.2) | 181.6 (92.9..299.5) | 0.70 |
| 100 | 438.3 (246.9..711.3) | 300.4 (146.2..402.3) | 0.69 |
| 300 | 846.2 (684.0..1623.6) | 546.7 (311.9..763.9) | 0.65 |
| 1000 | 1858.5 (1650.5..2246.3) | 905.2 (828.9..1551.6) | 0.49 |
| 3000 | 7557.3 (6988.3..9679.7) | 5114.7 (4757.6..6412.8) | 0.68 |

1000 台で **1.86 ms → 0.91 ms**、3000 台で **7.56 ms → 5.11 ms**。
調査の「起動は 1 台約 2.5 µs、3000 台を 1 フレームで起こすと 8 ms」は、
1 台約 1.7 µs・3000 台 5.1 ms になった。残っている 1.7 µs は `task_spawn`（コンテキストとスタック）で、
R2 は触っていない。

**バイト列のハッシュは起動フレームに見えない。** 1 台につき 294 バイトを SipHash-1-3 で 1 回ハッシュして
比較する代わりに `Vm::load` の parse と `ireps.push` が消えているので、差し引きで速い。
「`AssetId` の速い道を別に持つ」案（下の「捨てた案」）は要らなかった。

### 常駐メモリ（`mem` モード、台数ごとに別プロセス）

`started − with_vm` を台数で割った値（起動直後の 1 台あたり kB）と、`running − with_vm` を割った値。

| 台数 | 起動直後 前 | 起動直後 後 | 走らせた後 前 | 走らせた後 後 |
|---|---|---|---|---|
| 10 | 3.20 | 2.40 | 133.60 | 130.00 |
| 100 | 2.44 | 1.68 | 21.24 | 20.44 |
| 300 | 2.39 | 1.64 | 11.37 | 10.56 |
| 1000 | 2.20 | 1.45 | 8.90 | 8.11 |
| 3000 | 2.21 | 1.46 | 8.95 | 8.38 |

3000 台の絶対値では、起動直後が 28.9 MB → 26.3 MB、60 フレーム走らせた後が 49.1 MB → 47.1 MB
（ピーク `VmHWM` は 49.6 → 47.1 MB）。

ここで**調査の読み方が 1 つ間違っていたことが分かった**。調査は「起動で 1 台 2.2 kB（irep のコピー）」と
書いていたが、irep を共有してもまだ 1.46 kB 残る。**2.2 kB のうち irep は 0.75 kB で、残りはタスク側**
（コンテキスト、スタック、Task オブジェクト）だった。R2 が消せるのはその 0.75 kB のほうで、
3000 台で 2.2 MB。表の鍵が抱えるのは、同じプログラムの `.mrb` 1 部 = **294 バイト**（この計測スクリプト）で、
台数に依らない。

### 定常のフレーム時間（悪くなっていないこと）

`frame_median` を 40 セル全部で比べた。3 回の繰り返しの幅（計測器が `(lo..hi)` で出す）が前後で重なるセルが
**27/40**。重ならない 13 セルは両方向に散っていて（10 台の 5 セルは後が 100〜230 µs 速く、
300 台 sleep 0.25 の 2 セルは後が 60〜250 µs 遅い）、系統的なずれではない。
いちばん大きく見えるのは `300 / 0.25 / generous` の 200.9 → 450.7 µs だが、前のこのセルは
同じ 300 台の他の 3 セル（299〜410 µs）より低く出ていて、そこだけが外れている。

**コードの側から言えることのほうが強い**: R2 が変えたのは `start_scripts` の中だけで、
そこはすべてのエンティティが `ScriptTask` を持ったあとは空のクエリを回すだけになる。
定常のフレームでは `irep_of` は 1 度も呼ばれない。数字が散るのは計測器のぶれで、
大きいセル（1000 台 8.2 ms、3000 台 15.7 / 49.5 ms）はどれも ±5% 以内に収まっている。

再現の手順:

```bash
# 後
cd rubevy-wt-generalize && CARGO_TARGET_DIR=../rubevy/target cargo build --release --example factory_machines
# 前（target を別にする。同じにすると 2 つ目が建たず 1 つ目のバイナリが残る）
git -C rubevy worktree add --detach ../rubevy-wt-r2-before c66d220
cd rubevy-wt-r2-before && CARGO_TARGET_DIR=<別の場所> cargo build --release --example factory_machines
md5sum <前> <後>          # 別物であること
taskset -c 2 <各バイナリ> 90 3
for n in 10 100 300 1000 3000; do taskset -c 2 <各バイナリ> mem $n 60; done
```

## 捨てた案

* **鍵を `AssetId` にする。** 計画書の既定。差し替えの検知が正確（イベントの id で引ける）で、
  鍵が 8 バイトなのでハッシュが安い。採らなかったのは上の実物調査の結果 —
  garden が 1 匹ごとにコンパイルしているので、いちばん効いてほしい場面で 1 回も共有されない。
* **`AssetId` の表と内容の表の 2 段**（`AssetId` で引いて外れたら内容で引く）。速い道（id のハッシュ）を
  残しつつ内容でも共有できる。採らなかったのは、表が 2 つになると無効化の規則も 2 つになり、
  片方だけ消し忘れる形のバグが入るため。まず 1 つで測り、バイト列のハッシュが起動フレームに
  効いていたら戻ってくる、という順にした（測った結果は下）。
* **ハッシュ値だけを鍵にする**（`HashMap<u64, IrepId>`）。バイト列を持たずに済む。採らなかったのは
  衝突が「別のスクリプトが走る」になるから。2.2 kB を 1 部持つほうが安い。
* **`Handle<MrbAsset>` を鍵にする。** `Handle` は等値比較できるが `AssetId` と同じ話で、
  加えて強い参照を握るとアセットが解放されなくなる。

## 確かめたこと

`cargo test --workspace`: **116 passed / 3 ignored**（着手前 110 / 3。増えた 6 本が下の新しいテスト）。
`cargo clippy --workspace --all-targets`: 警告は既存の 2 件のみ（`src/lib.rs` の `collapsible_if`、
`examples/headless.rs` の `type_complexity`）。新しい警告 0。
`cargo doc --no-deps`: 警告 0。
`cargo build --target wasm32-unknown-unknown --lib` 通る（足したのは `std::collections::HashMap` だけで、
これは `src/lib.rs` が既に 4 か所で使っているもの）。`cargo test --test no_wasm_unsupported` 2 passed。

新しいテストは `tests/shared_irep.rs` の 6 本:

1. `a_hundred_entities_of_one_program_load_it_once` — 1 台ぶんと 100 台ぶんで `vm.ireps.len()` の増分が
   同じで、100 台とも実際に動いている。
2. `the_same_text_in_another_asset_is_the_same_irep` — 同じ本文を `Assets::add` で 2 回足して
   `AssetId` が違うことを確かめたうえで、irep は 1 つ。**これが garden の形。**
3. `tasks_that_share_an_irep_do_not_share_its_literals` — 5 本のタスクが `s = "abc"; 3.times { s << "!" }`
   を走らせて全部が 6 を返す（プールを共有していたら 6, 9, 12, … になる）。
4. `a_script_replaced_with_another_text_runs_it` — `replace_script` で別の本文にすると新しいほうが走り、
   `loaded_programs()` が 2。
5. `a_reloaded_asset_starts_the_new_code` — **同じ `AssetId` のまま中身だけ差し替える**
   （`Assets::insert(id, …)`、ホットリロードがやること）。その後に spawn したスクリプトが新しい本文で走る。
6. `applying_the_same_text_again_costs_nothing_and_a_new_one_costs_an_irep` — 上の「M 回」の表。

既存の `tests/replace.rs`（4 本）と `tests/restart_burst.rs` は 1 行も変えずに通る。

## 気づいた点

* **`.mrb` を 2 回 parse している。** `MrbLoader::load`（`src/lib.rs:129`）が読み込み時に
  `sabiruby::rite::parse` で検証し、`Vm::load` が起動時にもう 1 回 parse する。R2 で 2 回目は
  「プログラム 1 本につき 1 回」になったので実害はほぼ消えたが、形としては残っている。
  rubevy（小さな重複）。
* **garden は 1 匹ごとに `Assets::add` している**（`garden/src/main.rs:2660-2685` の `give_mind` が
  `spawn_creature` のたび・出産のたびに呼ばれる）。R2 で VM の中の irep は 1 部になったが、
  `Assets<MrbAsset>` の中には同じバイト列が匹数ぶん残る（1 本 2.2 kB）。種ごとに 1 つの `Handle` を
  持てば両方消える。rubevy_games（S2 の範囲）。
* **`ScriptWorld::loaded_programs()` はプログラムの本数で、irep の本数ではない。**
  1 プログラムがいくつの irep になるかはそのプログラムの形（ブロックの数）による。
  irep の総数を見たいホストは `vm.ireps.len()` を読むしかなく、`Vm::ireps` は `#[doc(hidden)]`。
  VM（外から読める数え口が無い）／R5 の `FrameStats` の材料。
* **壊れた `.mrb` は毎フレーム・毎エンティティで load をやり直す。** `start_scripts` は
  失敗を `error!` して `continue` するだけで、そのアセットを覚えない（R2 の表も成功したものしか
  入れない）。ログが毎フレーム流れる。着手前からの性質で、R2 は変えていない。rubevy（既存の性質）。
* **`Vm::ireps` が `#[doc(hidden)] pub` であることに、rubevy のテストと計測器が依存している。**
  sabiruby がここを閉じると `tests/shared_irep.rs` と R5 の example が建たなくなる。
  VM（公開の約束が曖昧な場所）。
