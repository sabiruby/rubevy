# rubevy: 「終わるまで待つ動作」の口と、止まった場所を持つ `ScriptEnded` — 実装指示書

作成 2026-09-21。**2026-09-22 に著者が「この形で F4 の前に入れる」と承認**（名前は案のまま:
`hold_requests` / `Held` / `ScriptEnded::at`）。実装の記録は
`docs/worklog/2026-09-22-held-requests.md`。
出どころは rubevy_games の 3 本目 Factory の段階 F3（インサータの Ruby）。担当が「rubevy の話」として挙げた 2 件で、記録は
rubevy_games `docs/worklog/2026-09-21-factory-F3.md`（`move` の待ち方の 3 案の比較、気づいた点 1・2）と `docs/plans/factory-plan.md` §7 の 09-21 F3 の最初の 2 行。
利用者の側の実物は rubevy_games `factory/src/inserters.rs` の `Arms`（`waiting: HashMap<usize, Request>` とその後始末）と
`factory/ruby/prelude.rb` の `run_inserter` の `rescue`・`Inserter.said_at`（約 30 行）。
対象: rubevy main `abfc875`。

---

## 1. 何が足りないのか

### H1. スクリプトが「終わるまで待つ」動作を頼んだとき、待っている request の置き場所がゲームのものになっている

`Rubevy.ask("factory.move").pop` は「腕を振り終わるまで返らない」。今の rubevy でこれは書ける —
`take_requests` で受け取り、`Request` を持ち続け、何フレームか後に `ScriptWorld::answer` で答える（`Request` の rustdoc がそう言っている）。
足りないのはその間の**置き場所と後始末**で、どのゲームも同じものを書くことになる:

- **誰の request かの索引**（Factory は `HashMap<タイル, Request>`）。答えを出すのは「その entity の動作が終わった」と知った system なので、entity から request を引けなければならない。
- **後始末**: entity が despawn された、スクリプトが差し替えられた（`replace_script`）、タスクが壊れて終わった — のどれでも、持っている `Request` は宛先の無い紙になる。
  ゲームが自分の表から消さなければ溜まり、消し忘れても何も起きない（黙って漏れる）。

`answer_with` は future を取るので「毎フレーム Bevy の世界を見て、満たされたら答える」には使えない（future は `World` を見られない）。
`answer_in_tick` は tick の中で即答する口で、待てない。F4 の control stage の「納品されるまで待つ」も同じ形になる。

### H2. 終わったスクリプトが「どこで止まったか」をゲームから読めない

`ScriptEnded { entity, status, value }` の `value` は例外の `inspect` だけで、場所が無い。終わったタスクには `ScriptWorld::stats` のフレームも残っていない
（実測 `frames=[] location=None finished=true`）。Factory は prelude の `rescue` が backtrace の先頭を切り出して `@broke_at` に置き、Rust が `ivar_get` で読む 30 行を書いた。
Ruby を書かせるゲームは全部「壊れたスクリプトの上に印と `ファイル:行` を出す」が要るので、全部がこの 30 行を書くことになる。

## 2. 形の案

### H1 の案: **待っている request は、頼んだ entity の component**（推奨）

```rust
// 登録: この種類の問いは、答えが出るまで頼んだ entity に載せておく
app.add_plugins(RubevyPlugin::default())
   .hold_requests("factory.move");           // M つきの VM には hold_requests_for::<Mods>(…)

// 着いたフレームに気づく: Bevy の普通のやり方で
fn start_the_swing(arms: Query<(Entity, &Held), Added<Held>>, mut grid: ResMut<Grid>) { … }

// 終わったら答える: component から取り出して答える（取り出した時点で component から消える）
fn finish_the_swing(mut arms: Query<&mut Held>, mut scripts: ResMut<ScriptWorld>, done: …) {
    for (entity, placed) in done {
        if let Ok(mut held) = arms.get_mut(entity) {
            held.answer(&mut scripts, "factory.move", Answer::Bool(placed));
        }
    }
}
```

- **置き場所が entity なので、索引は `Query` で、後始末の半分（despawn）は Bevy がやる。** 残りの半分 — スクリプトの差し替え、タスクが終わった／壊れた —
  は rubevy が知っていることなので rubevy が `Held` から消す。ゲームの側に表も後始末も残らない。
- `Held` は「この entity が今待っている問い」の一覧（1 つの entity が `on` のハンドラを複数持てば、同時に複数待ちうる）。中身は今の `Request` そのまま。
  空になったら component ごと外す（`Added<Held>` / `RemovedComponents<Held>` が「待ち始めた／待ち終わった」になる）。
- 登録していない種類の問いは今までどおり `take_requests` に出る。**今の API は 1 つも変えない**（足すだけ）。entity を持たないスクリプトの問いも今までどおり。
- 載せるのは `RubevySet::Answer`（tick の後）。同じフレームの後ろの system から見える。
- セーブは今と同じ: 待っている問いは保存できない（タスクの途中を持てないのと同じ理由）。ゲームは「頭から始め直せる形」にしておく。

**比べた案:**

| 案 | 形 | 採らなかった理由 |
|---|---|---|
| (B) request ごとのクロージャ `answer_when(request, impl FnMut(&World) -> Option<Answer>)` | F3 の担当の案。使う側は最も短い | クロージャは `&World` しか見られないので**動作を始める側（格子を書き換える）は結局別の system と別の索引が要る**。request ごとに Box が 1 つ、3,000 台なら毎フレーム 3,000 回の呼び出し。中が見えない（HUD に「何台待っているか」を出すのにも別の口が要る） |
| (C) 種類ごとのクロージャ `answer_when_ready(kind, Fn(&World, &Request) -> Option<Answer>)` | `answer_in_tick` の `None` つき版。確保は種類ごとに 1 つ | 始める側の問題は (B) と同じ。「着いた」をゲームが知る道が別に要る |
| (D) rubevy の中の表 `ScriptWorld::held(entity)` | component にしない | 索引と後始末は消えるが、「着いた／終わった」に気づく道を rubevy が別に作ることになる。Bevy が既に持っている（`Added` / `RemovedComponents`） |

### H2 の案: `ScriptEnded` に場所を足す

```rust
pub struct ScriptEnded<M = ()> {
    pub entity: Entity,
    pub status: ScriptStatus,
    pub value: String,
    /// `Failed` のとき、backtrace のうち**著者の行**（`in_the_authors_lines` と同じ線引き: prelude や rubevy の Ruby 層ではなく、
    /// そのスクリプトのファイルの中）の先頭。`"inserter.rb:27"`。デバッグ情報が無い、著者の行が 1 つも無い、正常に終わった、なら `None`。
    pub at: Option<String>,
    …
}
```

- `_m` が非公開なので外から構築はできず、フィールドを足しても構築は壊れない。`..` なしのパターン分解だけが壊れる（games の中に無いことを grep で確かめる）。
- prelude の行数を引く計算（Factory の `at - prelude_lines`）は `Program` が既に知っている（`in_the_authors_lines`）ので、ゲームはそれを書かなくてよくなる。
- 文字列 1 つにするか `(file, line)` に分けるかは、`in_the_authors_lines` が今返している形に合わせる（着手時に確かめる）。

## 3. 段階（案が通ったら）

| 段階 | 到達点 | 確認 |
|---|---|---|
| **H0** | 調査: 箱庭と Battle に同じ形（request を持ち続けて後で答える、壊れた場所を自前で拾う）があるか。あればその形でも案が成り立つか。`Held` の中身・複数の待ち・`M` つきの VM での名前を決める | 報告して、形が変わるなら著者に |
| **H1** | `hold_requests`・`Held`・差し替えと終了での後始末 | 単体テスト（着く、答える、despawn、`replace_script`、壊れて終わる、の各々で `Held` が残らない）。`examples/` に 10 行の実例。`how_many_scripts` の計測が前後で変わらない（登録しなければ費用 0） |
| **H2** | `ScriptEnded::at` | テスト（prelude つきのスクリプトで著者の行が出る、正常終了は `None`） |
| **H3** | docs（`docs/README.md` の目次、設計の文書、`CHANGELOG`）。**rubevy はブラウザでも動く**: `web/build.sh garden` + Playwright で pageerror 0 と `?selftest`（`/home/kishima/book/CLAUDE.md`）。版は上げない（公開は著者） | — |

games の側（Factory の `Arms::waiting` と prelude の 30 行を消す、F4 の「納品まで待つ」がこの口を使う）はこの計画の外で、rubevy の main が push された後。

## 4. 状況

| 段階 | 状況 |
|---|---|
| H0 | **済**（2026-09-22、ブランチ `held`）。箱庭と Battle に「request を持ち続ける」形は**無かった**（Factory だけ）。「壊れた場所を自前で拾う」形は Factory に 30 行、Battle には生きているタスクについての引き算がある。決めた 8 点は worklog §1.3 |
| H1 | **済**（`d5bc62e`）。`hold_requests` / `hold_requests_for::<M>` / `ScriptWorld::hold_requests` / `Held<M>`、後始末は component の removal hook 1 本。`tests/held.rs` 15 本、`examples/walk_to.rs`。登録しなければ費用 0 は前後 6 巡で確認（版の中のばらつきの方が大きい） |
| H2 | **済**（`ba5ed0e`）。`ScriptEnded::at: Option<(String, u32)>` と `Script::prelude_lines`。`tests/script_ended_at.rs` 9 本。**案と違う点**: `prelude_lines` は `Program` が持っていて rubevy は持っていなかったので `Script` に足した（worklog §4.1）。形は文字列 1 つではなく `(file, line)`（§4.2） |
| H3 | **済**。`docs/README.md`・`docs/host-api.md`・`CHANGELOG`（Unreleased）・`docs/numbers.md`（足した数は 0）。games をコピーした木で `cargo check --workspace --all-targets` が通り、ブラウザの箱庭が 48 行・FAIL 0・pageerror 0・requestfailed 0、Factory のヘッドレスが 19 行。**版は上げていない**（公開は著者） |

games の側（Factory の `Arms::waiting` と prelude の 30 行を消す、F4 の「納品まで待つ」）は
この計画の外で、rubevy の main が push された後。
