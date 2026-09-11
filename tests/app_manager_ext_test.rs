//! タスク #9b: Manager 拡張（set 別 BehaviorTable / disabledBehaviors 状態 /
//! request_spawn / restore_windows / behavior_menu_items）の契約テスト（RED）。
//!
//! Java 正本（.tmp/java-ref/）を仕様として、公開契約の振る舞いのみを検証する
//! （tests/app_test.rs 書式踏襲・自己完結・tests/common 不使用）。
//! 実装（#9b 分）未着手のため cargo test は compile error = RED が正常。
//!
//! pin する API 契約（coder への指示・シグネチャは tests が固定する）:
//!
//! ```text
//! // ---- src/app/environment.rs ----
//! pub struct SpawnRequest {                        // #9b: behavior_name を Option 化
//!     pub image_set_name: String,
//!     pub anchor: (i32, i32),
//!     pub look_right: bool,
//!     pub behavior_name: Option<String>,           // None = buildNextBehavior(null) 経路 /
//!                                                  // Some(name) = buildBehavior(name) 経路
//!                                                  //（Breed は Some を送る・Some("") は
//!                                                  // Java 同様 buildBehavior("") → Err → スキップ）
//! }
//!
//! pub trait OsSource {                             // 2 メソッド追加（default 実装なし・実装は #10）
//!     ...既存 4 メソッド（monitors / cursor_position / active_window / move_window）...
//!     fn windows(&self) -> Vec<(i64, Rect)>;
//!     // 可視・非アイコン化・非最大化の interactive 窓 = OUT_OF_BOUNDS 判定前の集合
//!     //（INVALID / IGNORED 選別の詳細は #10 の管轄）
//!     fn raise_window(&self, id: i64);             // BringWindowToTop 相当
//! }
//!
//! impl Environment {
//!     // Main.setMascotBehaviorEnabled L526-544 逐語のリスト変異:
//!     // contains && enabled → remove / !contains && !enabled → add /
//!     // 空リスト → エントリ削除 / else put。重複追加しない・無関係 set 無傷。
//!     pub fn set_behavior_enabled(&mut self, image_set: &str, name: &str, enabled: bool)
//!     // #9c 永続化用の読み出し（set → 無効リスト群のスナップショット・順序不問）。
//!     pub fn disabled_behaviors(&self) -> Vec<(String, Vec<String>)>
//! }
//!
//! impl EnvironmentView for Environment {
//!     // WindowsEnvironment.restoreWindows L292-347 逐語（DPI 逆スケールは不適用・
//!     // offset = 25 固定）:
//!     // - source.windows() のうち rect が screen() と intersects しない窓のみ処理
//!     // - 移動先 = (0,0) を含む monitor（Java プライマリ相当・無ければ先頭）の
//!     //   work area 左上 + offset（初期 25・移動 1 回ごとに +25・呼び出し毎にリセット）
//!     // - move_window(id, x, y) + raise_window(id)・境界内 / 交差する窓は無傷
//!     fn restore_windows(&self)
//! }
//!
//! // ---- src/mascot/mod.rs（EnvironmentView 追加 2 メソッド・default todo!()）----
//! fn queue_spawn_next(&self, image_set_name: &str, anchor: (i32, i32), look_right: bool)
//!     // behavior_name: None の SpawnRequest をキューへ積む
//!     //（#9b・Manager::request_spawn 経路・default todo!("app impl at #9b")）
//! fn restore_windows(&self)
//!     // default todo!("app impl at #9b")
//!
//! // ---- src/app/manager.rs ----
//! pub struct BehaviorMenu {                        // Java Mascot ポップアップ L523-553 相当
//!     pub selectable: Vec<String>,                 // 有効 && 名前に "/" を含まない行動（挿入順）
//!     pub toggleable: Vec<(String, bool)>,         // (行動名, checked = enabled)（挿入順）
//! }
//! impl Manager {
//!     // set 名 → BehaviorTable の上書き登録（未登録 set は既定 table にフォールバック）。
//!     // 全構築経路（spawn drain / set_behavior_all / set_behavior_all_of_set /
//!     // メニュー / mascot.tick の次行動構築）でマスコット自身の set の table を使う。
//!     pub fn set_behavior_table(&mut self, image_set_name: &str, table: BehaviorTable)
//!     // Java Manager.setBehaviorAll(Configuration, name, imageSet) L320-340 逐語:
//!     // 該当 set のマスコットのみ構築+setBehavior・他 set 無傷・構築 Err → log + dispose。
//!     pub fn set_behavior_all_of_set(&mut self, image_set_name: &str, name: &str)
//!     // tray(#9c) 用 passthrough（Environment::set_behavior_enabled へ委譲）。
//!     pub fn set_behavior_enabled(&mut self, image_set: &str, name: &str, enabled: bool)
//!     // Main.createMascot(String) L480-505 相当: rng 消費は呼び出し時
//!     //（setLookRight(Math.random() < 0.5)）・queue_spawn_next(set, (-4000,-4000), rng<0.5)。
//!     pub fn request_spawn(&mut self, image_set_name: &str)
//!     // Environment::restore_windows への passthrough。
//!     pub fn restore_windows(&mut self)
//!     // Java Mascot ポップアップ分類（L523-553）を table 挿入順で走査:
//!     // hidden → 完全スキップ / 名前に "/" 含む → 完全スキップ /
//!     // 有効な非 toggleable → selectable のみ / toggleable → toggleable に
//!     // (name, checked = enabled) かつ有効なら selectable にも /
//!     // 無効な非 toggleable → 両方に出ない。
//!     // 有効判定は behavior.rs の is_behavior_enabled と同一式（二重実装禁止）。
//!     // 未知 set は base table で動作。
//!     pub fn behavior_menu_items(&self, image_set_name: &str) -> BehaviorMenu
//! }
//! ```

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use simeji::app::environment::{Environment, OsSource};
use simeji::app::manager::{BehaviorMenu, Manager};
use simeji::config::script::Variable;
use simeji::config::{BehaviorDef, BehaviorEntry, BehaviorsConfig, SequenceChild, VarMap};
use simeji::mascot::behavior::{
    Action, ActionError, BehaviorError, BehaviorFactory, BehaviorTable,
};
use simeji::mascot::{EnvironmentView, Mascot, Rect, Rng};
use simeji::render::imageset::ImageSet;

// =====================================================================
// 合成データヘルパ（自己完結）
// =====================================================================

/// 矩形ヘルパ。
fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect {
        left,
        top,
        right,
        bottom,
    }
}

fn empty_image_set(name: &str) -> Arc<ImageSet> {
    Arc::new(ImageSet {
        name: name.to_string(),
        frames: BTreeMap::new(),
        warnings: Vec::new(),
    })
}

/// 名前から Arc<ImageSet> を引く resolver（固定 map・テスト用）。
fn resolver_for(names: &[&str]) -> impl FnMut(&str) -> Option<Arc<ImageSet>> {
    let mut map = HashMap::new();
    for name in names {
        map.insert(name.to_string(), empty_image_set(name));
    }
    move |requested: &str| map.get(requested).cloned()
}

// =====================================================================
// Environment — fake source（windows / raise_window 記録つき・契約 6）
// =====================================================================

/// OS 供給の状態（テストから RefCell 経由で差し替える）。
#[derive(Default)]
struct FakeState {
    monitors: Vec<(Rect, Rect)>,
    cursor: Option<(i32, i32)>,
    active_window: Option<(i64, Rect)>,
    windows: Vec<(i64, Rect)>,
    moved: Vec<(i64, i32, i32)>,
    raised: Vec<i64>,
}

struct FakeSource {
    state: Rc<RefCell<FakeState>>,
}

impl OsSource for FakeSource {
    fn monitors(&self) -> Vec<(Rect, Rect)> {
        self.state.borrow().monitors.clone()
    }

    fn cursor_position(&self) -> Option<(i32, i32)> {
        self.state.borrow().cursor
    }

    fn active_window(&self) -> Option<(i64, Rect)> {
        self.state.borrow().active_window
    }

    fn move_window(&self, id: i64, x: i32, y: i32) {
        self.state.borrow_mut().moved.push((id, x, y));
    }

    fn windows(&self) -> Vec<(i64, Rect)> {
        self.state.borrow().windows.clone()
    }

    fn raise_window(&self, id: i64) {
        self.state.borrow_mut().raised.push(id);
    }
}

type EnvHandle = Rc<RefCell<FakeState>>;

/// モニタ構成つき Environment（handle も返す・tick 間に状態差し替え可能）。
fn env_with_monitors(monitors: Vec<(Rect, Rect)>) -> (Environment, EnvHandle) {
    let state = Rc::new(RefCell::new(FakeState {
        monitors,
        cursor: None,
        active_window: None,
        windows: Vec::new(),
        moved: Vec::new(),
        raised: Vec::new(),
    }));
    (
        Environment::new(FakeSource {
            state: state.clone(),
        }),
        state,
    )
}

/// 単一モニタ構成: monitor (0,0,1920,1080) / work area (0,0,1920,1040)。
fn single_monitor_env() -> (Environment, EnvHandle) {
    env_with_monitors(vec![(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1040))])
}

// =====================================================================
// Manager fixture（scripted action + 合成 config）
// =====================================================================

/// Java Math.random 相当の [0,1) 乱数。キューを順に返し、枯渇 = 過剰消費として panic する。
struct BoxedRng {
    values: Vec<f64>,
    consumed: usize,
}

impl Rng for BoxedRng {
    fn unit(&mut self) -> f64 {
        let v = *self
            .values
            .get(self.consumed)
            .unwrap_or_else(|| panic!("BoxedRng 枯渇（{} 回要求）", self.consumed + 1));
        self.consumed += 1;
        v
    }
}

fn fixed_rng(values: Vec<f64>) -> Box<BoxedRng> {
    Box::new(BoxedRng {
        values,
        consumed: 0,
    })
}

/// 既定乱数（すべて 0.5・64 回分）。
fn unit_rng() -> Box<BoxedRng> {
    fixed_rng(vec![0.5; 64])
}

/// スクリプト付き action（has_next の true 回数を制御する）。
/// 既定は常時 true（= 遷移しない安定動作・rng を消費しない）。
struct ScriptedAction {
    has_next_true_times: usize,
    has_next_calls: usize,
}

impl Action for ScriptedAction {
    fn init(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        Ok(())
    }

    fn has_next(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.has_next_calls += 1;
        Ok(self.has_next_calls <= self.has_next_true_times)
    }

    fn next(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        Ok(())
    }
}

/// config 駆動ファクトリ。`transition_once` に含まれる名前の action は
/// has_next を 1 回だけ true で返す（次の has_next で遷移 → tick 経路の
/// 次行動構築を発火させる）。
struct ScriptedFactory {
    transition_once: Vec<String>,
}

impl ScriptedFactory {
    fn new() -> Self {
        ScriptedFactory {
            transition_once: Vec::new(),
        }
    }

    fn with_transitions(names: &[&str]) -> Self {
        ScriptedFactory {
            transition_once: names.iter().map(|n| (*n).to_string()).collect(),
        }
    }
}

impl BehaviorFactory for ScriptedFactory {
    fn build_action(&mut self, child: &SequenceChild) -> Result<Box<dyn Action>, BehaviorError> {
        match child {
            SequenceChild::Ref { name, .. } => {
                let times = if self.transition_once.iter().any(|n| n == name) {
                    1
                } else {
                    usize::MAX
                };
                Ok(Box::new(ScriptedAction {
                    has_next_true_times: times,
                    has_next_calls: 0,
                }))
            }
            SequenceChild::Inline(_) => Err(BehaviorError::UnknownBehavior(
                "(inline は本テストで未使用)".to_string(),
            )),
        }
    }
}

fn row_entry(name: &str, frequency: i32, hidden: bool, toggleable: bool) -> BehaviorEntry {
    BehaviorEntry::Single(BehaviorDef {
        name: name.to_string(),
        frequency,
        hidden,
        toggleable,
        action: SequenceChild::Ref {
            name: name.to_string(),
            attrs: VarMap::new(),
        },
        next: None,
    })
}

/// 非 toggleable・非 hidden の行 1 つ。
fn row(name: &str, frequency: i32) -> BehaviorEntry {
    row_entry(name, frequency, false, false)
}

/// Condition ノード 1 つ（AND 積み上げの Group）に行 1 つを束ねたエントリ。
fn group_entry(condition: &str, name: &str, frequency: i32) -> BehaviorEntry {
    BehaviorEntry::Group {
        conditions: vec![Variable::parse(condition)],
        behaviors: vec![BehaviorDef {
            name: name.to_string(),
            frequency,
            hidden: false,
            toggleable: false,
            action: SequenceChild::Ref {
                name: name.to_string(),
                attrs: VarMap::new(),
            },
            next: None,
        }],
    }
}

fn table(entries: Vec<BehaviorEntry>) -> BehaviorTable {
    BehaviorTable::new(&BehaviorsConfig { entries })
}

fn make_manager_with_rng(
    env: Environment,
    table: BehaviorTable,
    factory: ScriptedFactory,
    rng: Box<dyn Rng>,
) -> Manager {
    let mut manager = Manager::new(env, table, Box::new(factory), rng);
    manager.set_exit_on_last_removed(false);
    manager
}

fn make_manager(env: Environment, table: BehaviorTable, factory: ScriptedFactory) -> Manager {
    make_manager_with_rng(env, table, factory, unit_rng())
}

/// 指定 set の新規 Mascot（fresh・behavior 無し）。
fn mascot_of_set(image_set: &str, anchor: (i32, i32)) -> Mascot {
    Mascot::new(image_set, empty_image_set(image_set), anchor)
}

/// マスコット集合の観測スナップショット（apply_all 経由の公開 API のみ）。
#[derive(Debug, Clone)]
struct MascotView {
    image_set: String,
    behavior: Option<String>,
    anchor: (i32, i32),
    look_right: bool,
    remove_pending: bool,
}

fn snapshot(manager: &mut Manager) -> Vec<MascotView> {
    let mut out: Vec<MascotView> = Vec::new();
    manager.apply_all(|m| {
        out.push(MascotView {
            image_set: m.image_set_name().to_string(),
            behavior: m.behavior_name().map(|n| n.to_string()),
            anchor: m.anchor(),
            look_right: m.look_right(),
            remove_pending: m.remove_pending(),
        });
    });
    out
}

fn find_set<'a>(snap: &'a [MascotView], set: &str) -> &'a MascotView {
    snap.iter()
        .find(|v| v.image_set == set)
        .unwrap_or_else(|| panic!("set {set} の mascot が見つからない"))
}

/// disabled_behaviors() getter の set 検索ヘルパ（順序不問の契約のため）。
fn list_of(disabled: &[(String, Vec<String>)], set: &str) -> Vec<String> {
    disabled
        .iter()
        .find(|(s, _)| s == set)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

// =====================================================================
// 契約 1 + 3 + 5: set 別 BehaviorTable（spawn drain / tick 経路）+ request_spawn
// =====================================================================

/// 契約 1（AF）+ 契約 5:
/// - request_spawn(set) は rng 消費を呼び出し時に行い（Main.createMascot L490 の
///   setLookRight(Math.random() < 0.5) 逐語）、tick で set の table の
///   buildNextBehavior(None) 経路で構築される
/// - spawn drain / mascot.tick の次行動構築はマスコット自身の set の table を使う
///   （契約 1）。未登録 set は base table にフォールバックする
/// - 候補がある限り (-4000,-4000) から再配置しない
#[test]
fn manager_per_set_table_used_by_spawn_drain_and_tick_transition() {
    let (env, _) = single_monitor_env();
    let mut values = vec![0.6, 0.4];
    values.extend(vec![0.9; 32]);
    let mut manager = make_manager_with_rng(
        env,
        table(vec![row("Walk", 100)]),
        ScriptedFactory::with_transitions(&["Walk", "AltWalk"]),
        fixed_rng(values),
    );
    manager.set_image_set_resolver(resolver_for(&["TestSet", "AltSet"]));
    manager.set_behavior_table("AltSet", table(vec![row("AltWalk", 100)]));

    // rng pin: v0 = 0.6 → AltSet の look_right = false（0.6 は 0.5 未満でない）
    //          v1 = 0.4 → TestSet の look_right = true
    //          以降 0.9 → buildNextBehavior の頻度選択（単一候補なので選択は不変）
    manager.request_spawn("AltSet");
    manager.request_spawn("TestSet");
    manager.tick(Instant::now());

    assert_eq!(manager.count(), 2, "2 件の spawn 要求が反映される");
    assert_eq!(manager.count_of("AltSet"), 1);
    assert_eq!(manager.count_of("TestSet"), 1);

    // scripted action（has_next 1 回のみ true）により、spawn 同一 tick の tick ループで
    // 次行動構築が発火する: drain 経路（buildNextBehavior(None)）と tick 経路
    //（buildNextBehavior(Some(現行名))）の両方がこの 1 tick で走る。
    // 結果が "AltWalk" のままであることが両経路とも set の table を使うことの pin
    //（どちらか一方でも base table を渡していたら次行動は Walk に変わる:
    // "AltWalk" は base table に存在しないため）。
    let snap = snapshot(&mut manager);

    let alt = find_set(&snap, "AltSet");
    assert_eq!(
        alt.behavior.as_deref(),
        Some("AltWalk"),
        "spawn drain と mascot.tick の次行動構築の両方が set の table から構築する"
    );
    assert!(
        !alt.look_right,
        "rng v0=0.6 → look_right = false（rng.unit() < 0.5 の極性・呼び出し時消費）"
    );
    assert_eq!(
        alt.anchor,
        (-4000, -4000),
        "候補がある限り (-4000,-4000) から再配置しない"
    );

    let base = find_set(&snap, "TestSet");
    assert_eq!(
        base.behavior.as_deref(),
        Some("Walk"),
        "未登録 set は base table にフォールバックする"
    );
    assert!(base.look_right, "rng v1=0.4 → look_right = true");
    assert_eq!(base.anchor, (-4000, -4000));
}

/// 契約 3: set_behavior_all（Java L291-310）の強化。各マスコットは自身の set の
/// table で構築する（getConfiguration(mascot.getImageSet()) 相当）。base table に
/// 無い名前では、base set のマスコットだけが構築失敗 → dispose（L301-306 逐語）。
#[test]
fn manager_set_behavior_all_builds_from_each_mascots_own_table() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.set_behavior_table("AltSet", table(vec![row("AltWalk", 100)]));

    manager.add(mascot_of_set("AltSet", (100, 500)));
    manager.add(mascot_of_set("TestSet", (200, 500)));
    manager.tick(Instant::now());

    manager.set_behavior_all("AltWalk");
    manager.tick(Instant::now());

    assert_eq!(
        manager.count(),
        1,
        "TestSet 側は構築失敗 → dispose（次 tick で削除）"
    );
    let snap = snapshot(&mut manager);
    let alt = find_set(&snap, "AltSet");
    assert_eq!(
        alt.behavior.as_deref(),
        Some("AltWalk"),
        "AltSet のマスコットは自身の set の table で構築される"
    );
}

/// 契約 5: request_spawn の resolver 未設定 / 未知 set → スキップ（既存 spawn 挙動踏襲）。
#[test]
fn manager_request_spawn_skips_when_unresolved() {
    // resolver 未設定
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.request_spawn("TestSet");
    manager.tick(Instant::now());
    assert!(manager.is_empty(), "resolver 未設定 → スキップ");
    manager.tick(Instant::now());
    assert!(manager.is_empty(), "キューは drain で空・再処理されない");

    // 未知 set
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.set_image_set_resolver(resolver_for(&["TestSet"]));
    manager.request_spawn("MissingSet");
    manager.tick(Instant::now());
    assert!(manager.is_empty(), "未知 set → スキップ");
}

/// 契約 5: queue_spawn 経路の Some 化。Some(name) は buildBehavior(name) 経路
///（buildNextBehavior(None) の頻度選択に任せない）・Some("") は Java Breed L93 と
/// 同様 buildBehavior("") → Err → スキップ。
#[test]
fn manager_spawn_some_name_uses_build_behavior_and_empty_name_is_skipped() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(
        env,
        table(vec![row("Walk", 100), row("Stare", 100)]),
        ScriptedFactory::new(),
    );
    manager.set_image_set_resolver(resolver_for(&["TestSet"]));

    // rng pin: もし Some が buildNextBehavior(None) 経路なら rng 0.5 → Stare が選ばれる。
    // 正しくは buildBehavior("Walk") → Walk（rng を消費しない）。
    manager
        .environment_view()
        .queue_spawn("TestSet", (0, 500), false, "Walk");
    // BornBehaviour ""（Java 既定）→ Some("") → buildBehavior("") → Err → スキップ
    manager
        .environment_view()
        .queue_spawn("TestSet", (0, 500), false, "");
    manager.tick(Instant::now());

    assert_eq!(
        manager.count(),
        1,
        "Some(\"\") はスキップ・Some(\"Walk\") は追加"
    );
    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(
        m.behavior.as_deref(),
        Some("Walk"),
        "Some(name) は buildBehavior 経路（buildNextBehavior 経路なら Stare になる）"
    );
}

/// 契約 5: queue_spawn_next（None 経路のキューイング）と queue_spawn（Some 化）の
/// 格納形。drain で SpawnRequest を直接検査する。
#[test]
fn environment_spawn_queue_stores_none_for_next_and_some_for_named() {
    let (mut env, _) = single_monitor_env();
    {
        let view: &dyn EnvironmentView = &env;
        view.queue_spawn_next("TestSet", (-4000, -4000), true);
        view.queue_spawn("TestSet", (100, 200), false, "Walk");
    }
    let drained = env.drain_spawns();
    assert_eq!(drained.len(), 2, "FIFO 順保持");
    assert_eq!(drained[0].image_set_name, "TestSet");
    assert_eq!(drained[0].anchor, (-4000, -4000));
    assert!(drained[0].look_right);
    assert_eq!(
        drained[0].behavior_name, None,
        "queue_spawn_next → behavior_name: None（buildNextBehavior 経路）"
    );
    assert_eq!(drained[1].image_set_name, "TestSet");
    assert_eq!(drained[1].anchor, (100, 200));
    assert!(!drained[1].look_right);
    assert_eq!(
        drained[1].behavior_name,
        Some("Walk".to_string()),
        "queue_spawn → behavior_name: Some(name)（buildBehavior 経路）"
    );

    // drain 後は空（クリアされる）
    assert!(env.drain_spawns().is_empty());
}

// =====================================================================
// 契約 2: set_behavior_all_of_set（Java 3 引数 overload L320-340）
// =====================================================================

/// 該当 set のマスコットのみ構築+setBehavior・他 set は無傷・構築 Err →
/// そのマスコットのみ dispose（次 tick の retain で除去）。
#[test]
fn manager_set_behavior_all_of_set_targets_only_that_set() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.set_behavior_table("AltSet", table(vec![row("AltWalk", 100)]));

    manager.add(mascot_of_set("AltSet", (100, 500)));
    manager.add(mascot_of_set("TestSet", (200, 500)));
    manager.add(mascot_of_set("AltSet", (300, 500)));
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 3);

    // 該当 set のみ構築・他 set は無傷
    manager.set_behavior_all_of_set("AltSet", "AltWalk");
    let snap = snapshot(&mut manager);
    for view in &snap {
        if view.image_set == "AltSet" {
            assert_eq!(
                view.behavior.as_deref(),
                Some("AltWalk"),
                "該当 set は構築される"
            );
        } else {
            assert_eq!(view.behavior, None, "他 set は無傷（setBehavior されない）");
        }
        assert!(!view.remove_pending, "この呼び出しでは dispose しない");
    }

    // 構築 Err → その set のマスコットのみ dispose
    manager.set_behavior_all_of_set("TestSet", "Nope");
    let snap = snapshot(&mut manager);
    let base = find_set(&snap, "TestSet");
    assert!(base.remove_pending, "構築失敗 → dispose（削除は次 tick）");
    assert!(
        snap.iter()
            .filter(|v| v.image_set == "AltSet")
            .all(|v| !v.remove_pending && v.behavior.as_deref() == Some("AltWalk")),
        "失敗しても他 set は無傷"
    );

    manager.tick(Instant::now());
    assert_eq!(manager.count(), 2);
    assert_eq!(manager.count_of("AltSet"), 2);
    assert_eq!(manager.count_of("TestSet"), 0);
}

// =====================================================================
// 契約 4: disabledBehaviors 状態（Environment 保持）+ Manager passthrough
// =====================================================================

/// Main.setMascotBehaviorEnabled L526-544 逐語のリスト変異 +
/// behavior_disabled の Environment 実装がその map を読む（含有 = true・
/// 非含有 / 未知 set = false）+ 永続化(#9c)用の読み出し getter。
#[test]
fn environment_disabled_behaviors_state_is_verbatim_mutation() {
    let (mut env, _) = single_monitor_env();
    {
        let view: &dyn EnvironmentView = &env;
        // 既定 = Java 既定（無効リスト空 = 全行動有効）
        assert!(!view.behavior_disabled("TestSet", "Walk"));
        assert!(!view.behavior_disabled("UnknownSet", "Walk"));
    }

    // 無効化 → リスト追加
    env.set_behavior_enabled("TestSet", "Walk", false);
    {
        let view: &dyn EnvironmentView = &env;
        assert!(
            view.behavior_disabled("TestSet", "Walk"),
            "含有 = true（トグル OFF）"
        );
        assert!(
            !view.behavior_disabled("TestSet", "Stare"),
            "無関係行動は無傷"
        );
        assert!(
            !view.behavior_disabled("OtherSet", "Walk"),
            "無関係 set は無傷"
        );
    }

    // 重複追加しない（contains && !enabled → 何もしない）:
    // 二重無効化しても 1 回の有効化で消える
    env.set_behavior_enabled("TestSet", "Walk", false);
    env.set_behavior_enabled("TestSet", "Walk", true);
    {
        let view: &dyn EnvironmentView = &env;
        assert!(
            !view.behavior_disabled("TestSet", "Walk"),
            "重複追加がなければ 1 回の有効化で消える"
        );
    }

    // 2 行動を無効化 → getter に両方入る（重複しない）
    env.set_behavior_enabled("TestSet", "Walk", false);
    env.set_behavior_enabled("TestSet", "Stare", false);
    let disabled = env.disabled_behaviors();
    let list = list_of(&disabled, "TestSet");
    assert_eq!(
        list.len(),
        2,
        "重複追加しない（contains && !enabled → 何もしない）"
    );
    assert!(list.iter().any(|n| n == "Walk"));
    assert!(list.iter().any(|n| n == "Stare"));
    assert!(
        list_of(&disabled, "OtherSet").is_empty(),
        "無関係 set はエントリ無し"
    );

    // 1 件のみ有効化 → エントリは残る（リスト非空）
    env.set_behavior_enabled("TestSet", "Walk", true);
    let disabled = env.disabled_behaviors();
    assert_eq!(
        list_of(&disabled, "TestSet"),
        vec!["Stare".to_string()],
        "有効化で除去・残り 1 件のためエントリは存続"
    );

    // 残り 1 件も有効化 → リスト空 → エントリ消滅（L540-541 逐語）
    env.set_behavior_enabled("TestSet", "Stare", true);
    assert!(
        list_of(&env.disabled_behaviors(), "TestSet").is_empty(),
        "空リスト → エントリ削除"
    );
    assert!(env.disabled_behaviors().is_empty(), "map 全体も空");

    // 非含有への enabled = true は no-op（L534 の else if 不成立逐語）
    env.set_behavior_enabled("TestSet", "Walk", true);
    assert!(env.disabled_behaviors().is_empty());
}

/// 契約 4: Manager passthrough。toggleable 行動を無効化すると Manager 経路の
/// build_behavior も再配置 + Fall フォールバックする（behavior.rs L545-571・#9a 式）。
#[test]
fn manager_set_behavior_enabled_passthrough_gates_toggleable_behavior() {
    let gate_table = || table(vec![row("Fall", 100), row_entry("Pose", 100, false, true)]);

    // 無効化あり: Pose → 再配置 + Fall（rng 0.5・Java L545-548 逐語）
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, gate_table(), ScriptedFactory::new());
    manager.add(mascot_of_set("TestSet", (500, 500)));
    manager.tick(Instant::now());

    manager.set_behavior_enabled("TestSet", "Pose", false);
    manager.set_behavior_all("Pose");
    let snap = snapshot(&mut manager);
    let m1 = find_set(&snap, "TestSet");
    assert_eq!(
        m1.behavior.as_deref(),
        Some("Fall"),
        "無効化された toggleable → Fall フォールバック"
    );
    assert_eq!(
        m1.anchor,
        (960, -256),
        "再配置式（multiscreen 既定 → screen・(0.5*(1920-2)) as i32 + left + 1, top - 256）"
    );

    // 無効化なし: Pose のまま
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, gate_table(), ScriptedFactory::new());
    manager.add(mascot_of_set("TestSet", (500, 500)));
    manager.tick(Instant::now());
    manager.set_behavior_all("Pose");
    let snap = snapshot(&mut manager);
    assert_eq!(find_set(&snap, "TestSet").behavior.as_deref(), Some("Pose"));
}

// =====================================================================
// 契約 6: restore_windows（WindowsEnvironment L292-347 逐語）
// =====================================================================

/// screen() と intersects しない窓のみ、(0,0) を含む monitor の work area 左上 +
/// offset（25,50,75…移動 1 回ごとに +25・呼び出しごとにリセット）へ move + raise。
/// 境界内 / 部分交差する窓は無傷。
#[test]
fn environment_restore_windows_moves_only_offscreen_with_cascade() {
    let (env, state) = single_monitor_env();
    env.tick(); // screen union を更新（restore の intersects 判定は tick 済みの値を使う）

    // 境界内 / 画面外 / 部分交差
    state.borrow_mut().windows = vec![
        (11, rect(100, 100, 400, 300)),   // 画面内 → 無傷
        (22, rect(5000, 100, 5400, 300)), // 画面外 → 移動
        (33, rect(1900, 100, 2600, 300)), // 部分交差 → 無傷
    ];
    env.restore_windows();
    assert_eq!(
        *state.borrow().moved,
        [(22, 25, 25)],
        "画面外のみ WorkArea 左上 + 25（MoveWindow・サイズ維持）"
    );
    assert_eq!(*state.borrow().raised, [22], "BringWindowToTop 相当");

    // 2 回目の呼び出し: offset は呼び出しごとに 25 に戻る（Java L294 のローカル変数）+
    // 移動 1 回ごとに +25（列挙順にカスケード）
    state.borrow_mut().windows = vec![
        (22, rect(5000, 100, 5400, 300)),
        (44, rect(5000, 400, 5400, 600)),
    ];
    env.restore_windows();
    assert_eq!(
        *state.borrow().moved,
        [(22, 25, 25), (22, 25, 25), (44, 50, 50)],
        "移動 1 回ごとに offset +25"
    );
    assert_eq!(*state.borrow().raised, [22, 22, 44]);
}

/// 契約 6: 移動先は (0,0) を含む monitor（Java プライマリ相当・無ければ先頭）の
/// work area。先頭モニタがセカンダリの構成でも (0,0) 含有モニタの work area を使う。
#[test]
fn environment_restore_windows_targets_primary_monitor_work_area() {
    let (env, state) = env_with_monitors(vec![
        (rect(1920, 0, 3840, 1080), rect(1920, 0, 3840, 1040)), // 先頭 = セカンダリ
        (rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1040)),       // (0,0) を含む = プライマリ
    ]);
    env.tick();
    state.borrow_mut().windows = vec![(99, rect(4000, 100, 4400, 300))];
    env.restore_windows();
    assert_eq!(
        *state.borrow().moved,
        [(99, 25, 25)],
        "プライマリ work area (0,0) + 25（先頭モニタの (1945,25) でないこと）"
    );
}

/// 契約 6: Manager passthrough。
#[test]
fn manager_restore_windows_passthrough() {
    let (env, state) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    state.borrow_mut().windows = vec![(7, rect(5000, 100, 5400, 300))];
    manager.tick(Instant::now()); // env 更新（screen union）
    manager.restore_windows();
    assert_eq!(*state.borrow().moved, [(7, 25, 25)]);
    assert_eq!(*state.borrow().raised, [7]);
}

// =====================================================================
// 契約 7: behavior_menu_items（Java Mascot ポップアップ L523-553 の分類）
// =====================================================================

/// table 挿入順で走査: hidden → 完全スキップ / 名前に "/" 含む → 完全スキップ /
/// 有効な非 toggleable → selectable のみ / toggleable → toggleable に
/// (name, checked = enabled) かつ有効なら selectable にも / 無効な非 toggleable →
/// 両方に出ない。無効判定式は `!toggleable || !behavior_disabled(set, name)`
///（behavior.rs と同一式・Java L583-588 短絡）のため、無効リストは
/// toggleable 行にのみ効く（非 toggleable は常に有効 = 「無効な非 toggleable」は到達不能）。
/// 未知 set は base table で動作。無効化は Manager passthrough 経由
///（契約 4 の passthrough も同時に pin）。
#[test]
fn manager_behavior_menu_items_classify_rows_in_insertion_order() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(
        env,
        table(vec![
            row("Walk", 100),                         // 有効な非 toggleable → selectable
            row_entry("Stare", 100, false, true),     // toggleable + 無効 → toggleable のみ
            row("Transform/Dance", 100),              // "/" 含む → 完全スキップ
            row_entry("HiddenOne", 100, true, false), // hidden → 完全スキップ
            row_entry("Jump", 100, false, true),      // toggleable + 有効 → 両方
            row_entry("Spin", 100, false, true),      // toggleable + 無効 → toggleable のみ
        ]),
        ScriptedFactory::new(),
    );
    manager.set_behavior_enabled("TestSet", "Stare", false); // passthrough 経由で無効化
    manager.set_behavior_enabled("TestSet", "Spin", false);

    let menu: BehaviorMenu = manager.behavior_menu_items("TestSet");
    assert_eq!(
        menu.selectable,
        ["Walk", "Jump"],
        "有効 && \"/\" を含まない → selectable（挿入順）"
    );
    assert_eq!(
        menu.toggleable,
        [
            ("Stare".to_string(), false),
            ("Jump".to_string(), true),
            ("Spin".to_string(), false)
        ],
        "toggleable は (名前, checked = enabled)・無効でも行自体は出る"
    );

    // 未知 set: base table で動作・無効状態は set 単位のため全行動有効
    let menu = manager.behavior_menu_items("UnknownSet");
    assert_eq!(menu.selectable, ["Walk", "Stare", "Jump", "Spin"]);
    assert_eq!(
        menu.toggleable,
        [
            ("Stare".to_string(), true),
            ("Jump".to_string(), true),
            ("Spin".to_string(), true)
        ]
    );

    // 注入 set の table が使われる（契約 1 のメニュー経路）
    manager.set_behavior_table("AltSet", table(vec![row("AltWalk", 100)]));
    let menu = manager.behavior_menu_items("AltSet");
    assert_eq!(menu.selectable, ["AltWalk"]);
    assert!(menu.toggleable.is_empty());
}

// =====================================================================
// タスク #17: mascot.totalCount の配線（Java Mascot.getTotalCount L986-988 =
// manager.getCount() live 参照）。公開経路 Mascot::eval_snapshot().total_count で
// 各マスコットから見た生存数を検証する（実装詳細フィールドには触れない）。
// =====================================================================

/// 全マスコットが現在解決する `mascot.totalCount`（Mascot::eval_snapshot の公開経路）。
fn total_counts(manager: &mut Manager) -> Vec<i32> {
    let mut out = Vec::new();
    manager.apply_all(|m| out.push(m.eval_snapshot().total_count));
    out
}

/// 契約 1: add / spawn の次 tick 反映後、各マスコットの `mascot.totalCount` が
/// 現在の生存数（`Manager::count()`）を返す。既存実装は既定値 1 固定で FAIL。
#[test]
fn manager_tick_wires_total_count_to_live_survivor_count() {
    // add 経路
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    for i in 0..3 {
        manager.add(mascot_of_set("TestSet", (100 + i * 50, 500)));
    }
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 3);
    assert_eq!(
        total_counts(&mut manager),
        vec![3, 3, 3],
        "add 反映後は各マスコットから生存数 3 が見える"
    );

    // spawn drain 経路でも同じ（drain 反映の後・全員 tick の前に配線される）
    manager.set_image_set_resolver(resolver_for(&["TestSet"]));
    manager.request_spawn("TestSet");
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 4);
    assert_eq!(
        total_counts(&mut manager),
        vec![4, 4, 4, 4],
        "spawn 反映後は新規を含む全員が生存数 4 を見る"
    );
}

/// 契約 2: 除去（dispose → 次 tick の retain）後は生存数が減少して見える。
/// 増加方向だけでなく減少方向も最新化されることを pin する（増加時のみ更新する
/// 実装では残存マスコットが古い値のままになり FAIL）。
#[test]
fn manager_tick_refreshes_total_count_downward_after_removal() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.add(mascot_of_set("TestSet", (100, 500)));
    manager.add(mascot_of_set("TestSet", (200, 500)));
    manager.add(mascot_of_set("TestSet", (300, 500)));
    manager.tick(Instant::now());
    assert_eq!(
        total_counts(&mut manager),
        vec![3, 3, 3],
        "除去前は生存数 3"
    );

    // (100,500) のみ dispose → 次 tick の retain 後、残りは減少した生存数を見る
    manager.apply_all(|m| {
        if m.anchor() == (100, 500) {
            m.dispose();
        }
    });
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 2);
    assert_eq!(
        total_counts(&mut manager),
        vec![2, 2],
        "除去反映後は生存数が減少して見える（減少方向も最新化）"
    );
}

/// 契約 3: 増殖上限の縮小再現。条件 `#{mascot.totalCount < N}`（N=2）を持つ
/// Behavior が、生存数 N 未満では候補になり、N 以上では候補外になること
/// （= conf/behaviors.xml の `#{mascot.totalCount < 50}` が機能すること）を pin。
///
/// 現在行動 "Loop" は has_next 1 回で完了し、次 tick の遷移で buildNextBehavior が
/// 走る（遷移時の条件評価は配線済みの生存数を使う）。生存数 2 では Rare が候補外に
/// なるため、乱数値 0.0（先頭候補に落ちる値）でも Rare は選ばれず Loop のままになる。
#[test]
fn manager_total_count_gates_threshold_condition_behavior_selection() {
    let crowd_table = || {
        table(vec![
            // 条件 `#{mascot.totalCount < 2}` 付き（Rare は条件成立時のみ候補）
            group_entry("#{mascot.totalCount < 2}", "Rare", 100),
            // 現在行動（遷移を発火させる）
            row("Loop", 100),
            // 候補無しフォールバック先（frequency 0 なので候補にはならない）
            row_entry("Fall", 0, false, false),
        ])
    };

    // 生存 1 体（< 2）: 条件成立 → Rare が候補になり選択される
    let (env, _) = single_monitor_env();
    let mut manager = make_manager_with_rng(
        env,
        crowd_table(),
        ScriptedFactory::with_transitions(&["Loop"]),
        fixed_rng(vec![0.0; 16]),
    );
    manager.add(mascot_of_set("TestSet", (500, 500)));
    manager.tick(Instant::now());
    manager.set_behavior_all("Loop");
    manager.tick(Instant::now());
    let snap = snapshot(&mut manager);
    assert_eq!(
        find_set(&snap, "TestSet").behavior.as_deref(),
        Some("Rare"),
        "生存数 1 < 2 → 条件成立で Rare が選択される"
    );

    // 生存 2 体（>= 2）: 条件不成立 → Rare は候補外 → Loop のまま（増殖上限が機能）
    let (env, _) = single_monitor_env();
    let mut manager = make_manager_with_rng(
        env,
        crowd_table(),
        ScriptedFactory::with_transitions(&["Loop"]),
        fixed_rng(vec![0.0; 16]),
    );
    manager.add(mascot_of_set("TestSet", (500, 500)));
    manager.add(mascot_of_set("TestSet", (700, 500)));
    manager.tick(Instant::now());
    manager.set_behavior_all("Loop");
    manager.tick(Instant::now());
    let snap = snapshot(&mut manager);
    for view in &snap {
        assert_eq!(
            view.behavior.as_deref(),
            Some("Loop"),
            "生存数 2 >= 2 → Rare は候補外（totalCount 条件が増殖上限として機能）"
        );
    }
}
