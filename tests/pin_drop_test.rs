//! 機能 #30 / タスク 30-4: ドラッグドロップした窓の最前面固定
//! （pin フロー / holder ライフサイクル / 落下 clamp / holder 隔離）の契約テスト（RED）。
//!
//! 設計正本: `.tmp/design.md` §1.10(z)（item 3・4・5）。計画: `.tmp/plan.md` 30-4。
//! 既存 `tests/pin_window_test.rs`（30-3・Environment 側）とは別ファイルで、
//! Manager / Mascot 側の公開契約のみを FakeSource で検証する。
//!
//! 本テストが固定する公開 API（30-4 未実装のため cargo test は compile error = RED が正常）:
//!
//! ```text
//! // ---- src/mascot/mod.rs ----
//! impl Mascot {
//!     pub fn pinned_window(&self) -> Option<i64>;         // ミラー・非 pin は None
//!     pub fn set_pinned_window(&mut self, id: Option<i64>);
//! }
//!
//! // ---- src/app/environment.rs ----
//! impl Environment {
//!     pub fn window_at_point(&self, x: i32, y: i32) -> Option<(i64, Rect)>; // OsSource への passthrough
//! }
//!
//! // ---- src/app/manager.rs ----
//! impl Manager {
//!     pub fn set_pin_dropped_window_allowed(&mut self, allowed: bool);  // 既定 false・OFF で即 unpin
//!     pub fn mouse_released_at(&mut self, index: usize, point: (i32, i32))
//!         -> Result<(), BehaviorError>;                                 // point 追加
//!     pub fn unpin_pinned_window(&mut self);                            // 解除 + ミラークリア
//! }
//! ```
//!
//! 実 Win32 依存（SetWindowPos / EnumWindows）は自動テスト不能のため、FakeSource で
//! 記録・再現できる範囲のみを検証する（実機確認は design §1.10(z) 末尾に委ねる）。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use shimeji::app::environment::{Environment, OsSource};
use shimeji::app::manager::Manager;
use shimeji::config::{BehaviorDef, BehaviorEntry, BehaviorsConfig, SequenceChild, VarMap};
use shimeji::mascot::behavior::{
    Action, ActionError, BehaviorError, BehaviorFactory, BehaviorTable,
};
use shimeji::mascot::{EnvironmentView, Mascot, Rect, Rng};
use shimeji::render::imageset::ImageSet;

// =====================================================================
// 合成ヘルパ
// =====================================================================

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
        scale: 1.0,
    })
}

/// 指定 set の新規 Mascot（fresh・behavior 無し）。
fn mascot_of_set(image_set: &str, anchor: (i32, i32)) -> Mascot {
    Mascot::new(image_set, empty_image_set(image_set), anchor)
}

// =====================================================================
// FakeSource（window_at_point / window_frame / topmost 記録つき）
// =====================================================================

/// FakeSource の可変状態（テストから `Rc<RefCell<..>>` 経由で差し替える）。
#[derive(Default)]
struct FakeState {
    monitors: Vec<(Rect, Rect)>,
    cursor: Option<(i32, i32)>,
    active_window: Option<(i64, Rect)>,
    windows: Vec<(i64, Rect)>,
    moved: Vec<(i64, i32, i32)>,
    raised: Vec<i64>,
    /// `window_at_point` の応答（None = 該当窓なし）。
    at_point: Option<(i64, Rect)>,
    /// `window_at_point` に渡された点の記録（解放点がそのまま届くことの確認）。
    at_point_queries: Vec<(i32, i32)>,
    /// `window_frame` の応答（存在しない id は None）。
    frames: Vec<(i64, Rect)>,
    /// `is_window_topmost` が返す現在値（既定 false）。
    topmost: Vec<(i64, bool)>,
    /// `set_window_topmost` 成功可否（UIPI 保護窓相当 = false）。
    set_topmost_ok: bool,
    /// `set_window_topmost` の呼び出し列。
    set_topmost_calls: Vec<(i64, bool)>,
}

impl FakeState {
    fn frame(&self, id: i64) -> Option<Rect> {
        self.frames
            .iter()
            .find(|(fid, _)| *fid == id)
            .map(|(_, r)| *r)
    }

    fn set_frame(&mut self, id: i64, r: Rect) {
        match self.frames.iter_mut().find(|(fid, _)| *fid == id) {
            Some((_, slot)) => *slot = r,
            None => self.frames.push((id, r)),
        }
    }

    fn clear_frame(&mut self, id: i64) {
        self.frames.retain(|(fid, _)| *fid != id);
    }

    fn is_topmost(&self, id: i64) -> bool {
        self.topmost
            .iter()
            .find(|(tid, _)| *tid == id)
            .map(|(_, v)| *v)
            .unwrap_or(false)
    }

    fn set_topmost_state(&mut self, id: i64, value: bool) {
        match self.topmost.iter_mut().find(|(tid, _)| *tid == id) {
            Some((_, slot)) => *slot = value,
            None => self.topmost.push((id, value)),
        }
    }
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

    fn window_at_point(&self, x: i32, y: i32) -> Option<(i64, Rect)> {
        let mut state = self.state.borrow_mut();
        state.at_point_queries.push((x, y));
        state.at_point
    }

    fn window_frame(&self, id: i64) -> Option<Rect> {
        self.state.borrow().frame(id)
    }

    fn set_window_topmost(&self, id: i64, topmost: bool) -> bool {
        let mut state = self.state.borrow_mut();
        state.set_topmost_calls.push((id, topmost));
        if state.set_topmost_ok {
            state.set_topmost_state(id, topmost);
            true
        } else {
            false
        }
    }

    fn is_window_topmost(&self, id: i64) -> bool {
        self.state.borrow().is_topmost(id)
    }
}

/// 単一モニタ (0,0,1920,1080) / work area (0,0,1920,1040)。
/// `set_window_topmost` は既定で成功・`window_at_point` は既定 None。
fn env() -> (Environment, Rc<RefCell<FakeState>>) {
    let state = Rc::new(RefCell::new(FakeState {
        monitors: vec![(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1040))],
        set_topmost_ok: true,
        ..Default::default()
    }));
    (
        Environment::new(FakeSource {
            state: state.clone(),
        }),
        state,
    )
}

// =====================================================================
// Manager fixture（合成 config + scripted action）
// =====================================================================

/// Java Math.random 相当の [0,1) 乱数。キューを順に返し、枯渇 = panic。
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

fn unit_rng() -> Box<BoxedRng> {
    fixed_rng(vec![0.5; 64])
}

/// 常に has_next=true・next no-op の action（マスコットの anchor を動かさない）。
struct ScriptedAction;

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
        Ok(true)
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

/// 全 Ref を ScriptedAction にするファクトリ。
struct ScriptedFactory;

impl BehaviorFactory for ScriptedFactory {
    fn build_action(&mut self, child: &SequenceChild) -> Result<Box<dyn Action>, BehaviorError> {
        match child {
            SequenceChild::Ref { .. } => Ok(Box::new(ScriptedAction)),
            SequenceChild::Inline(_) => Err(BehaviorError::UnknownBehavior(
                "(inline は本テストで未使用)".to_string(),
            )),
        }
    }
}

/// 構築された Behavior 名を記録するファクトリ
/// （「既に下端掴み 3 種なら再遷移しない」= 再構築されないことの唯一の観測点）。
struct RecordingFactory {
    built: Rc<RefCell<Vec<String>>>,
}

impl BehaviorFactory for RecordingFactory {
    fn build_action(&mut self, child: &SequenceChild) -> Result<Box<dyn Action>, BehaviorError> {
        match child {
            SequenceChild::Ref { name, .. } => {
                self.built.borrow_mut().push(name.clone());
                Ok(Box::new(ScriptedAction))
            }
            SequenceChild::Inline(_) => Err(BehaviorError::UnknownBehavior(
                "(inline は本テストで未使用)".to_string(),
            )),
        }
    }
}

/// tick 中の `EnvironmentView::active_window_id()` を記録する action
/// （holder スコープ隔離の観測点・Action は env を受け取る公開シーム）。
struct ProbeAction {
    name: String,
    log: Rc<RefCell<Vec<(String, i64)>>>,
}

impl Action for ProbeAction {
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
        Ok(true)
    }

    fn next(
        &mut self,
        _mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.log
            .borrow_mut()
            .push((self.name.clone(), env.active_window_id()));
        Ok(())
    }
}

struct ProbeFactory {
    log: Rc<RefCell<Vec<(String, i64)>>>,
}

impl BehaviorFactory for ProbeFactory {
    fn build_action(&mut self, child: &SequenceChild) -> Result<Box<dyn Action>, BehaviorError> {
        match child {
            SequenceChild::Ref { name, .. } => Ok(Box::new(ProbeAction {
                name: name.clone(),
                log: self.log.clone(),
            })),
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

fn row(name: &str, frequency: i32) -> BehaviorEntry {
    row_entry(name, frequency, false, false)
}

fn table(entries: Vec<BehaviorEntry>) -> BehaviorTable {
    BehaviorTable::new(&BehaviorsConfig {
        entries,
        ..Default::default()
    })
}

/// Idle + 下端掴み 3 種（clamp の強制遷移先が構築可能な table）。
fn clamp_table() -> BehaviorTable {
    table(vec![
        row("Idle", 100),
        row("ClimbIEBottom", 100),
        row("GrabIEBottomLeftWall", 100),
        row("GrabIEBottomRightWall", 100),
    ])
}

fn make_manager(
    env: Environment,
    table: BehaviorTable,
    factory: Box<dyn BehaviorFactory>,
) -> Manager {
    let mut manager = Manager::new(env, table, factory, unit_rng());
    manager.set_exit_on_last_removed(false);
    manager
}

/// マスコット集合の観測スナップショット（apply_all 経由の公開 API のみ）。
#[derive(Debug, Clone)]
struct MascotView {
    image_set: String,
    behavior: Option<String>,
    anchor: (i32, i32),
    pinned: Option<i64>,
}

fn snapshot(manager: &mut Manager) -> Vec<MascotView> {
    let mut out: Vec<MascotView> = Vec::new();
    manager.apply_all(|m| {
        out.push(MascotView {
            image_set: m.image_set_name().to_string(),
            behavior: m.behavior_name().map(|n| n.to_string()),
            anchor: m.anchor(),
            pinned: m.pinned_window(),
        });
    });
    out
}

fn find_set<'a>(snap: &'a [MascotView], set: &str) -> &'a MascotView {
    snap.iter()
        .find(|v| v.image_set == set)
        .unwrap_or_else(|| panic!("set {set} の mascot が見つからない"))
}

fn is_bottom_behavior(name: Option<&str>) -> bool {
    matches!(
        name,
        Some("ClimbIEBottom") | Some("GrabIEBottomLeftWall") | Some("GrabIEBottomRightWall")
    )
}

/// index 0 のマスコットを holder として窓 30 を pin させる（トグル ON 経由）。
fn pin_index_zero(manager: &mut Manager, state: &Rc<RefCell<FakeState>>, w: Rect) {
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((30, w));
        st.set_frame(30, w);
    }
    manager.set_pin_dropped_window_allowed(true);
    manager
        .mouse_released_at(0, (w.left + 10, w.top + 10))
        .expect("mouse_released_at ok");
}

/// 窓 30 を pin 済みの Manager（holder = index 0・anchor は窓の水平範囲外）。
fn pinned_manager(anchor: (i32, i32)) -> (Manager, Rc<RefCell<FakeState>>, Rect) {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(
        env,
        table(vec![row("Idle", 100)]),
        Box::new(ScriptedFactory),
    );
    manager.add(mascot_of_set("TestSet", anchor));
    manager.tick(Instant::now());
    pin_index_zero(&mut manager, &state, w);
    (manager, state, w)
}

// =====================================================================
// 0. Environment::window_at_point の passthrough
// =====================================================================

#[test]
fn environment_window_at_point_delegates_to_source() {
    let (env, state) = env();
    let w = rect(10, 20, 110, 120);
    state.borrow_mut().at_point = Some((7, w));

    assert_eq!(
        env.window_at_point(33, 44),
        Some((7, w)),
        "OsSource::window_at_point の結果をそのまま返す"
    );
    assert_eq!(
        state.borrow().at_point_queries,
        vec![(33, 44)],
        "問い合わせ点がそのまま渡る"
    );
}

// =====================================================================
// 1. pin フロー（トグル ON / OFF / 窓なし / topmost 失敗）
// =====================================================================

#[test]
fn manager_pin_on_release_sets_topmost_and_mirror() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((30, w));
        st.set_frame(30, w);
    }
    let mut manager = make_manager(
        env,
        table(vec![row("Idle", 100)]),
        Box::new(ScriptedFactory),
    );
    manager.add(mascot_of_set("TestSet", (50, 100)));
    manager.tick(Instant::now());

    manager.set_pin_dropped_window_allowed(true);
    manager
        .mouse_released_at(0, (200, 250))
        .expect("release returns Ok");

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "解放点直下の窓 W を最前面固定する"
    );
    assert_eq!(
        state.borrow().at_point_queries,
        vec![(200, 250)],
        "解放点が window_at_point に渡る"
    );
    let snap = snapshot(&mut manager);
    assert_eq!(
        find_set(&snap, "TestSet").pinned,
        Some(30),
        "当該 Mascot のミラーが Some(W.id) になる"
    );
}

#[test]
fn manager_pin_disabled_by_default_does_not_pin() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((30, w));
        st.set_frame(30, w);
    }
    let mut manager = make_manager(
        env,
        table(vec![row("Idle", 100)]),
        Box::new(ScriptedFactory),
    );
    manager.add(mascot_of_set("TestSet", (50, 100)));
    manager.tick(Instant::now());

    // トグル既定 false のまま解放。
    manager
        .mouse_released_at(0, (200, 250))
        .expect("release returns Ok");

    assert!(
        state.borrow().set_topmost_calls.is_empty(),
        "トグル OFF では固定しない"
    );
    assert_eq!(find_set(&snapshot(&mut manager), "TestSet").pinned, None);
}

#[test]
fn manager_pin_skipped_without_window_under_point() {
    let (env, state) = env();
    // window_at_point は既定 None（該当窓なし）。
    let mut manager = make_manager(
        env,
        table(vec![row("Idle", 100)]),
        Box::new(ScriptedFactory),
    );
    manager.add(mascot_of_set("TestSet", (50, 100)));
    manager.tick(Instant::now());

    manager.set_pin_dropped_window_allowed(true);
    manager
        .mouse_released_at(0, (200, 250))
        .expect("release returns Ok");

    assert!(
        state.borrow().set_topmost_calls.is_empty(),
        "解放点直下に窓が無ければ固定しない"
    );
    assert_eq!(find_set(&snapshot(&mut manager), "TestSet").pinned, None);
}

#[test]
fn manager_pin_skipped_when_set_topmost_fails() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((30, w));
        st.set_frame(30, w);
        st.set_topmost_ok = false; // UIPI 保護窓相当
    }
    let mut manager = make_manager(
        env,
        table(vec![row("Idle", 100)]),
        Box::new(ScriptedFactory),
    );
    manager.add(mascot_of_set("TestSet", (50, 100)));
    manager.tick(Instant::now());

    manager.set_pin_dropped_window_allowed(true);
    manager
        .mouse_released_at(0, (200, 250))
        .expect("release returns Ok");

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "TOPMOST 化は試みられる"
    );
    assert_eq!(
        find_set(&snapshot(&mut manager), "TestSet").pinned,
        None,
        "失敗（false）時は pin を立てない"
    );
}

// =====================================================================
// 2. holder ライフサイクルによる解除
//    （a トグル OFF / b dragging / c 削除 / d 窓クローズ）
// =====================================================================

#[test]
fn manager_toggle_off_unpins_immediately() {
    let (mut manager, state, _w) = pinned_manager((50, 100));

    manager.set_pin_dropped_window_allowed(false);

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "トグル OFF で即 unpin（topmost を戻す）"
    );
    manager.tick(Instant::now());
    assert_eq!(
        find_set(&snapshot(&mut manager), "TestSet").pinned,
        None,
        "次 tick でミラーがクリアされる"
    );
}

#[test]
fn manager_tick_unpins_while_holder_dragging() {
    let (mut manager, state, _w) = pinned_manager((50, 100));

    manager.apply_all(|m| m.set_dragging(true));
    manager.tick(Instant::now());

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "保持マスコットがドラッグ中になったら解除する"
    );
    assert_eq!(find_set(&snapshot(&mut manager), "TestSet").pinned, None);
}

#[test]
fn manager_tick_unpins_when_holder_removed() {
    let (mut manager, state, _w) = pinned_manager((50, 100));

    manager.apply_all(|m| m.dispose());
    manager.tick(Instant::now());

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "保持マスコットの削除で解除する"
    );
    assert_eq!(manager.count(), 0, "削除は次 tick の retain で反映される");
}

#[test]
fn manager_tick_unpins_when_pinned_window_closes() {
    let (mut manager, state, _w) = pinned_manager((50, 100));

    // 窓クローズ / 不可視 / 最小化 / クローク相当。
    state.borrow_mut().clear_frame(30);
    manager.tick(Instant::now());

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "env.tick の window_frame=None → 自動 unpin"
    );
    assert_eq!(
        find_set(&snapshot(&mut manager), "TestSet").pinned,
        None,
        "Manager がミラーをクリアする"
    );
}

#[test]
fn manager_unpin_pinned_window_releases_and_clears_mirror() {
    let (mut manager, state, _w) = pinned_manager((50, 100));

    manager.unpin_pinned_window();

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "明示解除で topmost を戻す"
    );
    assert_eq!(
        find_set(&snapshot(&mut manager), "TestSet").pinned,
        None,
        "ミラーもクリアされる"
    );
}

// =====================================================================
// 3. 落下 clamp（既に下端以深を含む）
// =====================================================================

#[test]
fn manager_clamp_sets_anchor_to_window_bottom_for_holder() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((30, w));
        st.set_frame(30, w);
    }
    let mut manager = make_manager(env, clamp_table(), Box::new(ScriptedFactory));
    // anchor.x は窓の水平範囲内・anchor.y は既に下端以深。
    manager.add(mascot_of_set("TestSet", (250, 320)));
    manager.tick(Instant::now());
    manager.set_pin_dropped_window_allowed(true);
    manager
        .mouse_released_at(0, (200, 250))
        .expect("release returns Ok");
    manager.set_behavior_at(0, "Idle");

    manager.tick(Instant::now());

    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(m.pinned, Some(30));
    assert_eq!(
        m.anchor,
        (250, 300),
        "anchor を (anchor.x, W.bottom) に補正する"
    );
    assert!(
        is_bottom_behavior(m.behavior.as_deref()),
        "下端掴み 3 種のいずれかへ強制遷移する（実際: {:?}）",
        m.behavior
    );
}

#[test]
fn manager_clamp_skips_when_anchor_x_outside_window() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((30, w));
        st.set_frame(30, w);
    }
    let mut manager = make_manager(env, clamp_table(), Box::new(ScriptedFactory));
    // anchor.x = 50 < W.left = 100（水平範囲外）。
    manager.add(mascot_of_set("TestSet", (50, 320)));
    manager.tick(Instant::now());
    manager.set_pin_dropped_window_allowed(true);
    manager
        .mouse_released_at(0, (200, 250))
        .expect("release returns Ok");
    manager.set_behavior_at(0, "Idle");

    manager.tick(Instant::now());

    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(m.anchor, (50, 320), "anchor.x が水平範囲外なら補正しない");
    assert_eq!(
        m.behavior.as_deref(),
        Some("Idle"),
        "水平範囲外では下端掴みへ遷移しない"
    );
}

#[test]
fn manager_clamp_does_not_retransition_when_already_bottom_behavior() {
    let built = Rc::new(RefCell::new(Vec::new()));
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((30, w));
        st.set_frame(30, w);
    }
    let mut manager = make_manager(
        env,
        clamp_table(),
        Box::new(RecordingFactory {
            built: built.clone(),
        }),
    );
    // 既に下端（W.bottom）にいて、既に下端掴みの 1 つを実行中。
    manager.add(mascot_of_set("TestSet", (250, 300)));
    manager.tick(Instant::now());
    pin_index_zero(&mut manager, &state, w);
    manager.set_behavior_at(0, "ClimbIEBottom");
    built.borrow_mut().clear();

    manager.tick(Instant::now());

    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(
        m.behavior.as_deref(),
        Some("ClimbIEBottom"),
        "実行中の下端掴み行為を維持する"
    );
    assert_eq!(m.anchor, (250, 300), "anchor は変わらない");
    assert!(
        !built
            .borrow()
            .iter()
            .any(|n| is_bottom_behavior(Some(n.as_str()))),
        "既に下端掴み 3 種のいずれかにある場合は再遷移しない（再構築しない）"
    );
}

#[test]
fn manager_clamp_does_not_apply_to_non_holder() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((30, w));
        st.set_frame(30, w);
    }
    let mut manager = make_manager(env, clamp_table(), Box::new(ScriptedFactory));
    // holder は水平範囲外、非保持者は範囲内かつ下端以深。
    manager.add(mascot_of_set("HolderSet", (50, 100)));
    manager.add(mascot_of_set("OtherSet", (250, 320)));
    manager.tick(Instant::now());
    pin_index_zero(&mut manager, &state, w);
    manager.set_behavior_at(0, "Idle");
    manager.set_behavior_at(1, "Idle");

    manager.tick(Instant::now());

    let snap = snapshot(&mut manager);
    let holder = find_set(&snap, "HolderSet");
    let other = find_set(&snap, "OtherSet");
    assert_eq!(holder.pinned, Some(30), "index 0 が holder");
    assert_eq!(other.anchor, (250, 320), "非保持者には clamp を適用しない");
    assert_eq!(
        other.behavior.as_deref(),
        Some("Idle"),
        "非保持者は下端掴みへ遷移しない"
    );
}

// =====================================================================
// 4. holder 隔離（グローバル active window を pin で置換しない）
// =====================================================================

#[test]
fn manager_holder_scope_isolated_from_global_active_window() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let real = rect(500, 500, 700, 700);
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((30, w));
        st.set_frame(30, w);
        // 実アクティブ窓は pin 窓とは別 id。
        st.active_window = Some((999, real));
    }
    let log: Rc<RefCell<Vec<(String, i64)>>> = Rc::new(RefCell::new(Vec::new()));
    let mut manager = make_manager(
        env,
        table(vec![row("ProbeHolder", 100), row("ProbeOther", 100)]),
        Box::new(ProbeFactory { log: log.clone() }),
    );
    manager.add(mascot_of_set("HolderSet", (50, 100)));
    manager.add(mascot_of_set("OtherSet", (50, 100)));
    manager.tick(Instant::now());
    pin_index_zero(&mut manager, &state, w);
    manager.set_behavior_at(0, "ProbeHolder");
    manager.set_behavior_at(1, "ProbeOther");
    log.borrow_mut().clear();

    manager.tick(Instant::now());

    {
        let records = log.borrow();
        let holder = records
            .iter()
            .find(|(n, _)| n == "ProbeHolder")
            .expect("holder の probe が動く");
        assert_eq!(holder.1, 30, "holder の tick 中は pin 窓が activeIE");
        let other = records
            .iter()
            .find(|(n, _)| n == "ProbeOther")
            .expect("非保持者の probe が動く");
        assert_eq!(other.1, 999, "非保持者の tick 中は実アクティブ窓のまま");
    }
    assert_eq!(
        manager.environment_view().active_window_id(),
        999,
        "tick 外のグローバルな active window は pin で置換されない"
    );
}
