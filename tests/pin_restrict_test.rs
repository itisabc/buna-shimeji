//! 機能 #30 / タスク 30-9(C): ピン保持中の飛び降り行動の安全行動への差し替え
//! （案C: 縁伝いは維持しつつ、最前面固定を解除させない）の契約テスト（TDD RED）。
//!
//! 背景: ピン保持者は窓 W を activeIE として既存の IE 行動で縁を伝う。上面から
//! 飛び降りる 4 行動を選ぶと窓外へ落下し、30-8b により pin（最前面）が自動解除
//! されてしまう。案C は「ピン保持中だけ」この 4 行動を同系統の安全行動へ
//! 差し替える（差し替え先も IE の縁に留まる）。
//!
//! 差し替え写像（本テストが固定する契約）:
//! - `JumpFromLeftEdgeOfIE`  -> `SitOnTheLeftEdgeOfIE`
//! - `JumpFromRightEdgeOfIE` -> `SitOnTheRightEdgeOfIE`
//! - `WalkLeftAlongIEAndJump`  -> `WalkLeftAlongIEAndSit`
//! - `WalkRightAlongIEAndJump` -> `WalkRightAlongIEAndSit`
//!
//! == 固定する公開契約（実装詳細・モックには依存しない）==
//! 1. ピン保持者の behavior が飛び降り 4 行動のいずれかにある状態で tick すると、
//!    対応する安全行動へ差し替わる（tick 後の `behavior_name()` が安全側）。
//! 2. 差し替えで pin を解除しない（`Manager::pinned_holder()` が `Some` のまま・
//!    `set_window_topmost(id, false)` が記録されない）。
//! 3. 非保持者が同じ飛び降り行動を持っていても差し替えない。
//! 4. 飛び降り系以外（下端掴み・側面・上面の通常歩行・安全行動自身）は
//!    差し替えない（安全行動は冪等に維持）。
//! 5. pin 非保持時（解除後）は同じ飛び降り行動でも差し替えない。
//!
//! 30-8b（下端掴み後の Fall/Thrown で解除）の回帰は既存 `tests/pin_release_test.rs`
//! が担保する。本ファイルは重複を避け、差し替え契約のみを検証する。
//!
//! 注意（clamp との相互作用）: `clamp_holder_to_pinned_window` は
//! `anchor.y < W.bottom` のとき no-op。上面想定のケースではアンカーを窓下辺より
//! 上に置く（`follow_pinned_window_bottom` も同条件で no-op）。
//! ハーネスは `tests/pin_release_test.rs` / `tests/pin_drop_test.rs` を踏襲する
//! （独立コンパイル単位のため重複を許容）。

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

/// pin 対象窓 id。
const PIN_ID: i64 = 30;

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
    /// `window_frame` の応答（存在しない id は None）。
    frames: Vec<(i64, Rect)>,
    /// `is_window_topmost` が返す現在値（既定 false）。
    topmost: Vec<(i64, bool)>,
    /// `set_window_topmost` 成功可否（UIPI 保護窓相当 = false）。
    set_topmost_ok: bool,
    /// `set_window_topmost` の呼び出し列（解除フックの観測点）。
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

    fn window_at_point(&self, _x: i32, _y: i32) -> Option<(i64, Rect)> {
        self.state.borrow().at_point
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

fn unit_rng() -> Box<BoxedRng> {
    Box::new(BoxedRng {
        values: vec![0.5; 128],
        consumed: 0,
    })
}

/// 常に has_next=true・next no-op の action（anchor・behavior を動かさない）。
/// 差し替えは「behavior 名の遷移」だけで観測できるため物理は走らせない。
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

fn row(name: &str, frequency: i32) -> BehaviorEntry {
    BehaviorEntry::Single(BehaviorDef {
        name: name.to_string(),
        frequency,
        hidden: false,
        toggleable: false,
        action: SequenceChild::Ref {
            name: name.to_string(),
            attrs: VarMap::new(),
        },
        next: None,
    })
}

fn table(entries: Vec<BehaviorEntry>) -> BehaviorTable {
    BehaviorTable::new(&BehaviorsConfig {
        entries,
        ..Default::default()
    })
}

/// 差し替え判定・差し替え先構築・clamp のいずれもが名前で引ける表。
fn restrict_table() -> BehaviorTable {
    table(vec![
        row("Idle", 100),
        // 下端掴み 3 種（clamp の強制遷移先・30-8b のしがみつき判定）
        row("ClimbIEBottom", 100),
        row("GrabIEBottomLeftWall", 100),
        row("GrabIEBottomRightWall", 100),
        // 上面の通常歩行（飛び降り系ではない）
        row("WalkAlongIECeiling", 100),
        row("RunAlongIECeiling", 100),
        // 側面（飛び降り系ではない）
        row("HoldOntoIEWall", 100),
        row("ClimbIEWall", 100),
        // 安全行動（差し替え先・冪等に維持されるべき）
        row("SitOnTheLeftEdgeOfIE", 100),
        row("SitOnTheRightEdgeOfIE", 100),
        row("WalkLeftAlongIEAndSit", 100),
        row("WalkRightAlongIEAndSit", 100),
        // 飛び降り 4 行動（差し替え対象）
        row("JumpFromLeftEdgeOfIE", 100),
        row("JumpFromRightEdgeOfIE", 100),
        row("WalkLeftAlongIEAndJump", 100),
        row("WalkRightAlongIEAndJump", 100),
        // 30-8b の落下系（回帰の土台・本テストでは差し替え対象外）
        row("Fall", 100),
        row("Thrown", 100),
    ])
}

fn make_manager(env: Environment) -> Manager {
    let mut manager = Manager::new(env, restrict_table(), Box::new(ScriptedFactory), unit_rng());
    manager.set_exit_on_last_removed(false);
    manager
}

/// マスコット集合の観測スナップショット（apply_all 経由の公開 API のみ）。
#[derive(Debug, Clone)]
struct MascotView {
    image_set: String,
    behavior: Option<String>,
    pinned: Option<i64>,
}

fn snapshot(manager: &mut Manager) -> Vec<MascotView> {
    let mut out: Vec<MascotView> = Vec::new();
    manager.apply_all(|m| {
        out.push(MascotView {
            image_set: m.image_set_name().to_string(),
            behavior: m.behavior_name().map(|n| n.to_string()),
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

/// index 0 のマスコットを holder として窓 30 を pin させる（トグル ON 経由）。
fn pin_index_zero(manager: &mut Manager, state: &Rc<RefCell<FakeState>>, w: Rect) {
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((PIN_ID, w));
        st.set_frame(PIN_ID, w);
    }
    manager.set_pin_dropped_window_allowed(true);
    manager
        .mouse_released_at(0, (w.left + 10, w.top + 10))
        .expect("mouse_released_at ok");
}

/// 窓 30 を pin 済みの Manager（holder = index 0・anchor は呼び出し側指定）。
/// 既定 anchor は窓下辺（W.bottom）より上で clamp / follow が発火しない位置。
fn pinned_manager(anchor: (i32, i32)) -> (Manager, Rc<RefCell<FakeState>>, Rect) {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("TestSet", anchor));
    manager.tick(Instant::now());
    pin_index_zero(&mut manager, &state, w);
    (manager, state, w)
}

/// index の behavior を設定して 1 tick 進める
/// （差し替えは tick 内の保持者ブランチで評価される）。
fn set_behavior_and_tick(manager: &mut Manager, index: usize, name: &str) {
    manager.set_behavior_at(index, name);
    manager.tick(Instant::now());
}

// =====================================================================
// 1. 保持者の飛び降り 4 行動 → 対応する安全行動へ差し替え
// =====================================================================

/// 「飛び降り行動 `jump` → tick → 安全行動 `safe`」の共通本体。
/// 差し替えで pin を解除しないことも併せて固定する。
fn assert_holder_jump_replaced(jump: &str, safe: &str) {
    // anchor は窓下辺より上（clamp も追従も no-op）。
    let (mut manager, state, _w) = pinned_manager((250, 250));

    set_behavior_and_tick(&mut manager, 0, jump);

    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(
        m.behavior.as_deref(),
        Some(safe),
        "ピン保持者の `{jump}` は tick 後に `{safe}` へ差し替わる"
    );
    assert_eq!(m.pinned, Some(PIN_ID), "差し替えでミラーを失わない");
    assert_eq!(
        manager.pinned_holder(),
        Some(0),
        "差し替えで pin を解除しない（pinned_holder が Some のまま）"
    );
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(PIN_ID, true)],
        "差し替えで set_window_topmost(id, false) を呼ばない"
    );
}

#[test]
fn jump_from_left_edge_is_replaced_by_sit_left() {
    assert_holder_jump_replaced("JumpFromLeftEdgeOfIE", "SitOnTheLeftEdgeOfIE");
}

#[test]
fn jump_from_right_edge_is_replaced_by_sit_right() {
    assert_holder_jump_replaced("JumpFromRightEdgeOfIE", "SitOnTheRightEdgeOfIE");
}

#[test]
fn walk_left_along_ie_and_jump_is_replaced_by_walk_left_and_sit() {
    assert_holder_jump_replaced("WalkLeftAlongIEAndJump", "WalkLeftAlongIEAndSit");
}

#[test]
fn walk_right_along_ie_and_jump_is_replaced_by_walk_right_and_sit() {
    assert_holder_jump_replaced("WalkRightAlongIEAndJump", "WalkRightAlongIEAndSit");
}

// =====================================================================
// 3. 非保持者には差し替えを適用しない
// =====================================================================

#[test]
fn non_holder_jump_is_not_replaced() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("HolderSet", (250, 250)));
    manager.add(mascot_of_set("OtherSet", (250, 250)));
    manager.tick(Instant::now());
    pin_index_zero(&mut manager, &state, w);
    manager.set_behavior_at(0, "Idle");
    manager.set_behavior_at(1, "JumpFromLeftEdgeOfIE");

    manager.tick(Instant::now());

    let snap = snapshot(&mut manager);
    let holder = find_set(&snap, "HolderSet");
    let other = find_set(&snap, "OtherSet");
    assert_eq!(holder.pinned, Some(PIN_ID), "前提: index 0 が holder");
    assert_eq!(manager.pinned_holder(), Some(0), "前提: pin は保持中");
    assert_eq!(
        other.behavior.as_deref(),
        Some("JumpFromLeftEdgeOfIE"),
        "非保持者の飛び降り行動は差し替えない"
    );
}

// =====================================================================
// 4. 飛び降り系以外は差し替えない（安全行動は冪等に維持）
// =====================================================================

/// 「`name` の behavior は差し替えられない」共通本体（保持者・anchor は窓下辺上）。
fn assert_holder_behavior_unchanged(name: &str) {
    let (mut manager, _state, _w) = pinned_manager((250, 250));

    set_behavior_and_tick(&mut manager, 0, name);

    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(
        m.behavior.as_deref(),
        Some(name),
        "飛び降り系以外（`{name}`）は差し替えない"
    );
    assert_eq!(m.pinned, Some(PIN_ID), "前提: pin は保持中");
}

#[test]
fn bottom_holder_behaviors_are_not_replaced() {
    for name in [
        "ClimbIEBottom",
        "GrabIEBottomLeftWall",
        "GrabIEBottomRightWall",
    ] {
        assert_holder_behavior_unchanged(name);
    }
}

#[test]
fn side_holder_behaviors_are_not_replaced() {
    for name in ["HoldOntoIEWall", "ClimbIEWall"] {
        assert_holder_behavior_unchanged(name);
    }
}

#[test]
fn top_walk_holder_behaviors_are_not_replaced() {
    for name in ["WalkAlongIECeiling", "RunAlongIECeiling"] {
        assert_holder_behavior_unchanged(name);
    }
}

#[test]
fn safe_holder_behaviors_are_not_replaced() {
    // 差し替え先そのもの。再差し替え・揺り戻しが起きないこと（冪等）。
    for name in [
        "SitOnTheLeftEdgeOfIE",
        "SitOnTheRightEdgeOfIE",
        "WalkLeftAlongIEAndSit",
        "WalkRightAlongIEAndSit",
    ] {
        assert_holder_behavior_unchanged(name);
    }
}

// =====================================================================
// 5. pin 解除後は差し替えを適用しない（ピン保持中だけ）
// =====================================================================

#[test]
fn jump_not_replaced_after_pin_released() {
    let (mut manager, _state, _w) = pinned_manager((250, 250));
    // トグル OFF で即 unpin（保持マスコットのミラーもクリア）。
    manager.set_pin_dropped_window_allowed(false);

    set_behavior_and_tick(&mut manager, 0, "JumpFromLeftEdgeOfIE");

    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(m.pinned, None, "前提: pin は解除済み");
    assert_eq!(
        m.behavior.as_deref(),
        Some("JumpFromLeftEdgeOfIE"),
        "pin 非保持時は飛び降り行動を差し替えない"
    );
}
