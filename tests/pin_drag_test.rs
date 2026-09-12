//! 機能 #30 / タスク 30-7: 窓下辺の holder を掴んで引きはがしたときの pin 解除と、
//! そのドラッグ解放で再ピンしないことの契約テスト（RED）。
//!
//! 不具合: holder をクリックすると (1) 次 tick の clamp がドラッグを上書きし、
//! (2) 解放時に `pin_dropped_window_at` が走って再ピンする。要件 R16
//! 「掴んで引きはがす → 固定解除」の自然な実装として、掴んだ時点で pin を解除し、
//! その解放では再ピンしない、という公開挙動を固定する。
//!
//! 変更対象は `src/app/manager.rs` のみ（`pin_pull_off` は private のため本テストは
//! 触れない）。公開 API（`Manager::mouse_pressed_at` /
//! `mouse_released_at` / `set_pin_dropped_window_allowed`、`Mascot::pinned_window`、
//! `Environment`(FakeSource) の `set_window_topmost` 記録）だけを検証する。
//!
//! 実 Win32 依存（SetWindowPos 等）は自動テスト不能のため FakeSource で記録・再現
//! できる範囲のみを検証する（実機確認は design §1.10(z) に委ねる）。
//! FakeSource / ヘルパは既存 `tests/pin_drop_test.rs` の流儀を踏襲している。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use simeji::app::environment::{Environment, OsSource};
use simeji::app::manager::Manager;
use simeji::config::{BehaviorDef, BehaviorEntry, BehaviorsConfig, SequenceChild, VarMap};
use simeji::mascot::behavior::{
    Action, ActionError, BehaviorError, BehaviorFactory, BehaviorTable,
};
use simeji::mascot::{EnvironmentView, Mascot, Rect, Rng};
use simeji::render::imageset::ImageSet;

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

#[derive(Default)]
struct FakeState {
    monitors: Vec<(Rect, Rect)>,
    windows: Vec<(i64, Rect)>,
    /// `window_at_point` の応答（None = 該当窓なし）。
    at_point: Option<(i64, Rect)>,
    /// `window_frame` の応答（存在しない id は None）。
    frames: Vec<(i64, Rect)>,
    /// `is_window_topmost` が返す現在値（既定 false）。
    topmost: Vec<(i64, bool)>,
    /// `set_window_topmost` 成功可否。
    set_topmost_ok: bool,
    /// `set_window_topmost` の呼び出し列（本テストの主要観測点）。
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
        None
    }

    fn active_window(&self) -> Option<(i64, Rect)> {
        None
    }

    fn move_window(&self, _id: i64, _x: i32, _y: i32) {}

    fn windows(&self) -> Vec<(i64, Rect)> {
        self.state.borrow().windows.clone()
    }

    fn raise_window(&self, _id: i64) {}

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
// Manager fixture
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
        values: vec![0.5; 64],
        consumed: 0,
    })
}

/// 常に has_next=true・next no-op の action（マスコットを動かさない）。
/// `is_draggable` は既定 true のため `mouse_pressed` は `Dragged` 行を構築する
/// （テスト用 table に `Dragged` を用意する理由）。
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

fn row_entry(name: &str, frequency: i32) -> BehaviorEntry {
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
    BehaviorTable::new(&BehaviorsConfig { entries })
}

/// `Idle` + `Dragged`（mouse_pressed が Dragged を構築できる最小 table）。
fn press_table() -> BehaviorTable {
    table(vec![row_entry("Idle", 100), row_entry("Dragged", 100)])
}

fn make_manager(env: Environment, table: BehaviorTable) -> Manager {
    let mut manager = Manager::new(env, table, Box::new(ScriptedFactory), unit_rng());
    manager.set_exit_on_last_removed(false);
    manager
}

/// 全マスコットの pin ミラー（`Mascot::pinned_window`）を index 順に観測する。
fn pinned_mirrors(manager: &mut Manager) -> Vec<Option<i64>> {
    let mut out: Vec<Option<i64>> = Vec::new();
    manager.apply_all(|m| out.push(m.pinned_window()));
    out
}

/// index 0 を holder として窓 30 を pin させる（トグル ON 経由）。
fn pin_index_zero(manager: &mut Manager, state: &Rc<RefCell<FakeState>>, w: Rect) {
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((30, w));
        st.set_frame(30, w);
    }
    manager.set_pin_dropped_window_allowed(true);
    manager
        .mouse_released_at(0, (w.left + 10, w.top + 10))
        .expect("pin release ok");
}

/// 2 体（index 0 = holder / index 1 = 非 holder）を持ち、窓 30 を index 0 に
/// pin 済みの Manager。`mouse_pressed` 用に `Dragged` 行を含む table を使う。
fn pinned_two_holder(
    anchor0: (i32, i32),
    anchor1: (i32, i32),
) -> (Manager, Rc<RefCell<FakeState>>, Rect) {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(env, press_table());
    manager.add(mascot_of_set("HolderSet", anchor0));
    manager.add(mascot_of_set("OtherSet", anchor1));
    manager.tick(Instant::now());
    pin_index_zero(&mut manager, &state, w);
    (manager, state, w)
}

// =====================================================================
// 1. holder を掴んだ時点で pin 解除 + 全ミラークリア
// =====================================================================

#[test]
fn holder_press_unpins_and_clears_mirrors() {
    let (mut manager, state, w) = pinned_two_holder((50, 100), (250, 100));
    // 掴む点の直下にも窓が残っている（引きはがし操作の前提）。
    state.borrow_mut().at_point = Some((30, w));

    manager
        .mouse_pressed_at(0, (w.left + 10, w.bottom))
        .expect("press ok");

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "holder を掴んだ時点で最前面固定を解除する（set_window_topmost(id, false)）"
    );
    assert_eq!(
        pinned_mirrors(&mut manager),
        vec![None, None],
        "解除時に全マスコットの pin ミラーをクリアする"
    );
}

// =====================================================================
// 2. holder の引きはがし解放では再ピンしない
// =====================================================================

#[test]
fn holder_press_then_release_over_window_does_not_repin() {
    let (mut manager, state, w) = pinned_two_holder((50, 100), (250, 100));
    // カーソル（解放点）は窓の上にある = 通常なら再ピン条件を満たす。
    state.borrow_mut().at_point = Some((30, w));

    manager
        .mouse_pressed_at(0, (w.left + 10, w.bottom))
        .expect("press ok");
    // 押下で解除済み。解放時の呼び出しを孤立して観測する。
    state.borrow_mut().set_topmost_calls.clear();

    manager
        .mouse_released_at(0, (w.left + 10, w.bottom))
        .expect("release ok");

    assert!(
        state.borrow().set_topmost_calls.is_empty(),
        "引きはがしの解放では再ピンしない（set_window_topmost(id, true) を新たに呼ばない）"
    );
    assert_eq!(
        pinned_mirrors(&mut manager),
        vec![None, None],
        "引きはがし後は pin が無いまま"
    );
}

// =====================================================================
// 3. 対照: 非 holder のドロップは従来どおりピンする
//    (a) pin が無い状態からの初回ドロップ
//    (b) pin 済みの状態で他個体へ持ち替えるドロップ
// =====================================================================

#[test]
fn unpinned_press_then_release_over_window_still_pins() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((30, w));
        st.set_frame(30, w);
    }
    let mut manager = make_manager(env, press_table());
    manager.add(mascot_of_set("TestSet", (250, 100)));
    manager.tick(Instant::now());
    manager.set_pin_dropped_window_allowed(true);

    // pin が無い状態で掴み、窓の上で離す（通常のドロップ操作）。
    manager.mouse_pressed_at(0, (250, 150)).expect("press ok");
    assert_eq!(
        pinned_mirrors(&mut manager),
        vec![None],
        "掴んだだけではピンしない"
    );

    manager
        .mouse_released_at(0, (200, 250))
        .expect("release ok");

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "pin が無い状態からのドロップは従来どおり固定する"
    );
    assert_eq!(
        pinned_mirrors(&mut manager),
        vec![Some(30)],
        "ドロップした個体が holder になる"
    );
}

#[test]
fn non_holder_press_does_not_unpin_and_release_rehomes_pin() {
    let (mut manager, state, w) = pinned_two_holder((50, 100), (250, 100));
    state.borrow_mut().at_point = Some((30, w));

    // 非 holder(index 1) を押しても holder(index 0) の pin は解除されない。
    manager.mouse_pressed_at(1, (250, 150)).expect("press ok");
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "非 holder の押下では set_window_topmost を一切呼ばない"
    );
    assert_eq!(
        pinned_mirrors(&mut manager),
        vec![Some(30), None],
        "pin は holder(index 0) のまま"
    );

    // その解放は窓の上 → 従来どおり新しい holder(index 1) へピンし直す。
    manager
        .mouse_released_at(1, (w.left + 10, w.bottom))
        .expect("release ok");
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false), (30, true)],
        "非 holder のドロップは旧 pin を剥がして新 holder に固定する"
    );
    assert_eq!(
        pinned_mirrors(&mut manager),
        vec![None, Some(30)],
        "ミラーが新 holder(index 1) へ移る"
    );
}
