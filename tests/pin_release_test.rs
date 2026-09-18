//! 機能 #30 / タスク 30-8b（案A: しがみつき後の落下で pin 解除）の契約テスト（TDD RED）。
//!
//! 設計正本: `.tmp/design.md` §1.10(z) 追補「30-8b 案A」・`.tmp/plan.md` 30-8。
//! pin 生成（30-3/30-4）・解除フック（30-5）・追従平滑化（30-8a）は GREEN 済み。
//! 本ファイルは「保持者が下端掴み行為を一度でも取った後に Fall / Thrown へ遷移したら
//! pin を解除する」挙動のみを、既存の公開インターフェース（Manager / Mascot / FakeSource）
//! で観測する。**新規公開 API は不要**（30-8b 未実装のため cargo test は assertion 失敗 = RED が正常）。
//!
//! == 固定する契約（実装詳細・モックには依存しない）==
//! 1. 下端掴み 3 種（`ClimbIEBottom` / `GrabIEBottomLeftWall` / `GrabIEBottomRightWall`）を
//!    一度でも取った後に `Fall` または `Thrown` へ遷移 → pin 解除
//!    （観測: `set_window_topmost(W.id, false)` が記録され、`Manager::pinned_holder()` が None、
//!    保持者 Mascot の `pinned_window()` ミラーが None）。
//! 2. 解除時、`was_topmost == false` の窓のみ最前面解除。元から TOPMOST の窓は触らない。
//! 3. R13 保護: ドロップ直後の `Thrown`（下端掴み未経験）では解除しない。
//! 4. 下端掴み未経験の `Fall` でも解除しない。
//! 5. 下端掴み行為の実行中（しがみつき中）は pin を保持する。
//! 6. 解除後は同 tick / 以降の tick でアンカーが窓下辺へ引き戻されない（clamp が走らない）。
//! 7. 解除後に再ドロップして pin し直すと、前回の「しがみつき済み」を持ち越さない
//!    （新規 pin 直後の `Thrown` は解除しない）。
//!
//! 実 Win32（SetWindowPos / EnumWindows）は自動テスト不能のため、FakeSource で
//! 記録・再現できる範囲のみを検証する（実機確認は design §1.10(z) 末尾に委ねる）。
//! ハーネスは `tests/pin_drop_test.rs` を踏襲する（独立コンパイル単位のため重複を許容）。

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
    /// `window_at_point` に渡された点の記録。
    at_point_queries: Vec<(i32, i32)>,
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

fn unit_rng() -> Box<BoxedRng> {
    Box::new(BoxedRng {
        values: vec![0.5; 128],
        consumed: 0,
    })
}

/// 常に has_next=true・next no-op の action（anchor・behavior を動かさない）。
/// 30-8b の観測は「behavior 名の遷移」だけで駆動するため物理は走らせない。
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

/// 下端掴み 3 種 + Fall / Thrown + Idle を含む表（clamp と 30-8b の両方が構築可能）。
fn release_table() -> BehaviorTable {
    table(vec![
        row("Idle", 100),
        row("Fall", 100),
        row("Thrown", 100),
        row("ClimbIEBottom", 100),
        row("GrabIEBottomLeftWall", 100),
        row("GrabIEBottomRightWall", 100),
    ])
}

fn make_manager(env: Environment) -> Manager {
    let mut manager = Manager::new(env, release_table(), Box::new(ScriptedFactory), unit_rng());
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
fn pinned_manager(anchor: (i32, i32)) -> (Manager, Rc<RefCell<FakeState>>, Rect) {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("TestSet", anchor));
    manager.tick(Instant::now());
    pin_index_zero(&mut manager, &state, w);
    (manager, state, w)
}

/// 保持者の behavior を設定して 1 tick 進める（30-8b は tick 内の holder ブランチで判定される）。
fn set_behavior_and_tick(manager: &mut Manager, name: &str) {
    manager.set_behavior_at(0, name);
    manager.tick(Instant::now());
}

/// 保持者ミラー（index 0 の `pinned_window()`）。
fn holder_mirror(manager: &mut Manager) -> Option<i64> {
    find_set(&snapshot(manager), "TestSet").pinned
}

/// 「下端掴み→指定 behavior へ遷移」で解除されることを検証する共通本体。
///
/// anchor は窓下辺上（`y == W.bottom`）。解除が正しく clamp より先に評価されなければ、
/// clamp が behavior を下端掴みへ上書きして pin が残る（= FAIL）。
fn assert_released_after(bottom_behavior: &str, release_behavior: &str) {
    let (mut manager, state, _w) = pinned_manager((250, 300));

    // 前提: 下端掴みを一度取る（しがみつき中は pin 保持）。
    set_behavior_and_tick(&mut manager, bottom_behavior);
    assert_eq!(
        manager.pinned_holder(),
        Some(0),
        "前提: {bottom_behavior} 実行中は pin を保持"
    );

    // Fall / Thrown へ遷移 → 解除。
    set_behavior_and_tick(&mut manager, release_behavior);

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(PIN_ID, true), (PIN_ID, false)],
        "{bottom_behavior} 後の {release_behavior} で TOPMOST を解除する"
    );
    assert_eq!(
        holder_mirror(&mut manager),
        None,
        "保持者ミラーが None になる"
    );
    assert_eq!(
        manager.pinned_holder(),
        None,
        "Environment の pin も消える（pinned_holder が None）"
    );
}

// =====================================================================
// 1. 下端掴み後に Fall / Thrown へ遷移 → 解除
// =====================================================================

#[test]
fn release_after_climb_bottom_on_fall() {
    assert_released_after("ClimbIEBottom", "Fall");
}

#[test]
fn release_after_climb_bottom_on_thrown() {
    assert_released_after("ClimbIEBottom", "Thrown");
}

#[test]
fn release_after_grab_bottom_left_on_fall() {
    assert_released_after("GrabIEBottomLeftWall", "Fall");
}

#[test]
fn release_after_grab_bottom_right_on_thrown() {
    assert_released_after("GrabIEBottomRightWall", "Thrown");
}

// =====================================================================
// 2. was_topmost 規則（元から TOPMOST の窓は剥がさない）
// =====================================================================

#[test]
fn release_keeps_topmost_when_window_was_already_topmost() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("TestSet", (250, 300)));
    manager.tick(Instant::now());

    // pin 前から窓 30 は TOPMOST。
    state.borrow_mut().set_topmost_state(PIN_ID, true);
    pin_index_zero(&mut manager, &state, w);
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(PIN_ID, true)],
        "前提: was_topmost=true でも pin は (30, true) を記録する"
    );

    set_behavior_and_tick(&mut manager, "ClimbIEBottom");
    set_behavior_and_tick(&mut manager, "Fall");

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(PIN_ID, true)],
        "was_topmost=true の窓は解除時に set_window_topmost(id, false) を呼ばない"
    );
    assert_eq!(holder_mirror(&mut manager), None, "pin 自体は解除される");
    assert_eq!(manager.pinned_holder(), None);
}

// =====================================================================
// 3-4. R13 保護（下端掴み未経験では Fall / Thrown でも解除しない）
// =====================================================================

#[test]
fn no_release_for_thrown_without_cling() {
    // anchor は窓下辺より上（clamp も追従も不発）。
    let (mut manager, state, _w) = pinned_manager((250, 250));

    set_behavior_and_tick(&mut manager, "Thrown");

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(PIN_ID, true)],
        "ドロップ直後の Thrown では解除しない（R13）"
    );
    assert_eq!(
        holder_mirror(&mut manager),
        Some(PIN_ID),
        "ミラーは Some(W.id) のまま"
    );
    assert_eq!(manager.pinned_holder(), Some(0), "pin を保持する");
}

#[test]
fn no_release_for_fall_without_cling() {
    let (mut manager, state, _w) = pinned_manager((250, 250));

    set_behavior_and_tick(&mut manager, "Fall");

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(PIN_ID, true)],
        "下端掴み未経験の Fall では解除しない"
    );
    assert_eq!(holder_mirror(&mut manager), Some(PIN_ID));
    assert_eq!(manager.pinned_holder(), Some(0));
}

// =====================================================================
// 5. しがみつき中は解除しない
// =====================================================================

#[test]
fn pin_kept_while_clinging() {
    let (mut manager, state, _w) = pinned_manager((250, 300));

    set_behavior_and_tick(&mut manager, "ClimbIEBottom");
    // しがみつきを複数 tick 継続しても保持。
    manager.tick(Instant::now());
    manager.tick(Instant::now());

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(PIN_ID, true)],
        "下端掴み行為の実行中は解除しない"
    );
    assert_eq!(holder_mirror(&mut manager), Some(PIN_ID));
    assert_eq!(manager.pinned_holder(), Some(0));
}

// =====================================================================
// 6. 解除後は clamp が走らない（同 tick / 以降）
// =====================================================================

#[test]
fn anchor_not_clamped_after_release() {
    // anchor.x を窓中点 (=250) からずらす。もし clamp が走れば
    // 下端掴み行為（GrabIEBottomLeftWall）へ上書きされて検出できる。
    let (mut manager, _state, _w) = pinned_manager((200, 300));

    set_behavior_and_tick(&mut manager, "ClimbIEBottom");
    set_behavior_and_tick(&mut manager, "Fall");

    // 同 tick: 解除済み・behavior は Fall のまま（clamp に上書きされない）。
    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(m.pinned, None, "解除済み");
    assert_eq!(
        m.behavior.as_deref(),
        Some("Fall"),
        "解除後に clamp が走り下端掴み行為へ上書きされない"
    );
    assert_eq!(m.anchor, (200, 300), "anchor は窓下辺上に留まる");

    // 以降の tick でも窓下辺へ引き戻されない。
    manager.tick(Instant::now());
    manager.tick(Instant::now());

    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(m.pinned, None, "以降の tick でも pin は無い");
    assert_eq!(
        m.behavior.as_deref(),
        Some("Fall"),
        "以降の tick でも clamp に上書きされない"
    );
    assert_eq!(m.anchor, (200, 300), "以降の tick でも引き戻されない");
}

// =====================================================================
// 7. 再 pin 時に「しがみつき済み」を持ち越さない
// =====================================================================

#[test]
fn cling_flag_reset_on_new_pin() {
    let (mut manager, state, w) = pinned_manager((250, 300));

    // 1 回目の pin でしがみつき → Fall で解除。
    set_behavior_and_tick(&mut manager, "ClimbIEBottom");
    set_behavior_and_tick(&mut manager, "Fall");
    assert_eq!(manager.pinned_holder(), None, "前提: 1 回目は解除済み");

    // 再ドロップで pin し直す。
    manager
        .mouse_released_at(0, (w.left + 10, w.top + 10))
        .expect("re-pin returns Ok");
    assert_eq!(
        holder_mirror(&mut manager),
        Some(PIN_ID),
        "前提: 再 pin 成功"
    );

    // 直後の Thrown は解除されない（前回のしがみつき済みを持ち越さない）。
    set_behavior_and_tick(&mut manager, "Thrown");

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(PIN_ID, true), (PIN_ID, false), (PIN_ID, true)],
        "再 pin 直後の Thrown では解除フックが発火しない"
    );
    assert_eq!(
        holder_mirror(&mut manager),
        Some(PIN_ID),
        "再 pin が保持される"
    );
    assert_eq!(manager.pinned_holder(), Some(0));
}
