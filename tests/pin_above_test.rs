//! 機能 #30 / タスク 30-6: 保持マスコット窓をピン対象窓の前面へ再アサートするための
//! `Manager::pinned_holder` の契約テスト（RED）。
//!
//! 設計正本: `.tmp/design.md` §1.10(z)。計画: `.tmp/plan.md` 30-6。
//!
//! 本テストが固定する公開 API（未実装のため cargo test は E0599 = RED が正常）:
//!
//! ```text
//! // ---- src/app/manager.rs ----
//! impl Manager {
//!     pub fn pinned_holder(&self) -> Option<usize>;
//! }
//! ```
//!
//! `pinned_holder` は pin の真実（`Environment.pinned`）と各 `Mascot` のミラー
//! （`Mascot::pinned_window`）から**現在の**保持者 index を探索して返す。
//! `PinState.holder` に保存された index をそのまま返すのではなく、ミラーから
//! 現在位置を求めることで、削除による index ずれにも追随できる（30-6 の要）。
//!
//! 実 Win32 依存（`ensure_window_above` / `SetWindowPos`）と `src/main.rs` の配線は
//! 自動テスト不能のため対象外。`src/` は一切変更しない。

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
// 合成ヘルパ（tests/pin_drop_test.rs の流儀を踏襲）
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
// FakeSource（pin フローに必要な最小限の OS 供給）
// =====================================================================

#[derive(Default)]
struct FakeState {
    monitors: Vec<(Rect, Rect)>,
    cursor: Option<(i32, i32)>,
    active_window: Option<(i64, Rect)>,
    /// `window_at_point` の応答（None = 該当窓なし）。
    at_point: Option<(i64, Rect)>,
    /// `window_frame` の応答（存在しない id は None）。
    frames: Vec<(i64, Rect)>,
    /// `is_window_topmost` が返す現在値（既定 false）。
    topmost: Vec<(i64, bool)>,
    /// `set_window_topmost` 成功可否（UIPI 保護窓相当 = false）。
    set_topmost_ok: bool,
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

    fn move_window(&self, _id: i64, _x: i32, _y: i32) {}

    fn windows(&self) -> Vec<(i64, Rect)> {
        Vec::new()
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
        values: vec![0.5; 64],
        consumed: 0,
    })
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
    BehaviorTable::new(&BehaviorsConfig { entries })
}

fn make_manager(env: Environment) -> Manager {
    let mut manager = Manager::new(
        env,
        table(vec![row("Idle", 100)]),
        Box::new(ScriptedFactory),
        unit_rng(),
    );
    manager.set_exit_on_last_removed(false);
    manager
}

/// `index` のマスコットを holder として窓 30 を pin させる（トグル ON 経由）。
fn pin_index(manager: &mut Manager, state: &Rc<RefCell<FakeState>>, index: usize, w: Rect) {
    {
        let mut st = state.borrow_mut();
        st.at_point = Some((30, w));
        st.set_frame(30, w);
    }
    manager.set_pin_dropped_window_allowed(true);
    manager
        .mouse_released_at(index, (w.left + 10, w.top + 10))
        .expect("mouse_released_at ok");
}

/// 窓 30 をピン済みの Manager（holder = index 0）。
fn pinned_manager(anchor: (i32, i32)) -> (Manager, Rc<RefCell<FakeState>>, Rect) {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("TestSet", anchor));
    manager.tick(Instant::now());
    pin_index(&mut manager, &state, 0, w);
    (manager, state, w)
}

// =====================================================================
// 契約: pinned_holder
// =====================================================================

/// ピンが無い Manager では `None`。
#[test]
fn pinned_holder_is_none_without_pin() {
    let (env, _state) = env();
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("TestSet", (50, 100)));
    manager.tick(Instant::now());

    assert_eq!(manager.pinned_holder(), None, "ピンが無ければ None");
}

/// トグル ON で窓をピンした後、保持マスコットの現在 index を返す。
#[test]
fn pinned_holder_returns_current_index_after_pin() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("TestSet", (50, 100)));
    manager.tick(Instant::now());

    pin_index(&mut manager, &state, 0, w);

    assert_eq!(
        manager.pinned_holder(),
        Some(0),
        "ピン成立後は保持マスコットの index を返す"
    );
}

/// ピン解除（明示 `unpin_pinned_window`）後は `None` に戻る。
#[test]
fn pinned_holder_is_none_after_explicit_unpin() {
    let (mut manager, _state, _w) = pinned_manager((50, 100));

    manager.unpin_pinned_window();

    assert_eq!(manager.pinned_holder(), None, "明示解除後は None");
}

/// ピン解除（トグル OFF・即 unpin）後は `None` に戻る。
#[test]
fn pinned_holder_is_none_after_toggle_off() {
    let (mut manager, _state, _w) = pinned_manager((50, 100));

    manager.set_pin_dropped_window_allowed(false);

    assert_eq!(manager.pinned_holder(), None, "トグル OFF 後は None");
}

/// 複数マスコットのうち、保持していない個体の index は返さない。
/// 保持者が index 1 のときは `Some(1)`（`Some(0)` = 非保持者ではない）。
#[test]
fn pinned_holder_returns_holder_index_not_non_holder() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("FirstSet", (50, 100)));
    manager.add(mascot_of_set("SecondSet", (50, 100)));
    manager.tick(Instant::now());

    pin_index(&mut manager, &state, 1, w);

    assert_eq!(
        manager.pinned_holder(),
        Some(1),
        "保持者 index 1 のみを返し、非保持者 0 は返さない"
    );
}

/// 先行 index の非保持者が削除されて index がずれても、ミラーから現在位置を返す。
/// （`PinState.holder` の保存値をそのまま返す実装では `Some(1)` のままになり失敗する。）
#[test]
fn pinned_holder_follows_mirror_after_earlier_index_removed() {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("FirstSet", (50, 100)));
    manager.add(mascot_of_set("SecondSet", (50, 100)));
    manager.tick(Instant::now());
    pin_index(&mut manager, &state, 1, w);
    assert_eq!(manager.pinned_holder(), Some(1));

    // 先行 index 0 の非保持者を削除 → 保持者は index 0 に繰り上がる。
    manager.dismiss_at(0);
    manager.tick(Instant::now());

    assert_eq!(
        manager.pinned_holder(),
        Some(0),
        "削除で index がずれてもミラーから現在位置を探索して返す"
    );
}
