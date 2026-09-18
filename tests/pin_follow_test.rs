//! 機能 #30 / タスク 30-8a（窓追従の平滑化）の Manager 側契約テスト（TDD RED）。
//!
//! 設計正本: `.tmp/design.md` §1.10(z) 追補「30-8a 追従平滑化」・`.tmp/plan.md` 30-8。
//! 検証する契約（公開 API のみ・FakeSource で再現可能な範囲）:
//! - 保持マスコットのアンカーが前 tick の窓下辺上にあるとき、窓がゆっくり純並進
//!   （最大変位 ≤80px/tick・サイズ不変）なら、その tick 後のアンカーが新矩形の下辺に
//!   厳密追従する（`Manager::tick` の振る舞い変更。新規公開 API はない）。
//! - 純並進で最大変位 >80px/tick は追従しない（ユーザー承認 案Y・80px しきい値）。
//! - サイズ変更（`dleft != dright` 等）は 80px しきい値の対象外で常に追従する。
//! - アンカーが前 tick の窓下辺上に無い（y が下辺でない / x が水平範囲外）ときは追従しない。
//!
//! 実 Win32（SetWindowPos 等）には依存しない。既存 `tests/pin_drop_test.rs` の
//! Manager fixture（FakeSource / ScriptedAction / ScriptedFactory / clamp_table /
//! snapshot）パターンを踏襲する。Environment 側の delta 公開 / clear は
//! `tests/pin_window_test.rs` が担う。

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

/// pin 対象窓 id。FakeSource の window_at_point / window_frame が返す。
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
// FakeSource（pin に必要な最小限: window_at_point / window_frame / topmost）
// =====================================================================

#[derive(Default)]
struct FakeState {
    active_window: Option<(i64, Rect)>,
    /// `window_frame` の応答（存在しない id は None）。
    frames: Vec<(i64, Rect)>,
    /// `window_at_point` の応答（None = 該当窓なし）。
    at_point: Option<(i64, Rect)>,
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
}

struct FakeSource {
    state: Rc<RefCell<FakeState>>,
}

impl OsSource for FakeSource {
    fn monitors(&self) -> Vec<(Rect, Rect)> {
        vec![(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1040))]
    }

    fn cursor_position(&self) -> Option<(i32, i32)> {
        None
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

    fn set_window_topmost(&self, _id: i64, _topmost: bool) -> bool {
        true
    }

    fn is_window_topmost(&self, _id: i64) -> bool {
        false
    }
}

fn env() -> (Environment, Rc<RefCell<FakeState>>) {
    let state = Rc::new(RefCell::new(FakeState::default()));
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
/// 追従そのものを観測したいので、behavior の物理は一切走らせない。
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

fn clamp_table() -> BehaviorTable {
    BehaviorTable::new(&BehaviorsConfig {
        entries: vec![
            row("Idle", 100),
            row("ClimbIEBottom", 100),
            row("GrabIEBottomLeftWall", 100),
            row("GrabIEBottomRightWall", 100),
        ],
        ..Default::default()
    })
}

fn make_manager(env: Environment) -> Manager {
    let mut manager = Manager::new(env, clamp_table(), Box::new(ScriptedFactory), unit_rng());
    manager.set_exit_on_last_removed(false);
    manager
}

/// マスコット集合の観測スナップショット（apply_all 経由の公開 API のみ）。
#[derive(Debug, Clone)]
struct MascotView {
    image_set: String,
    anchor: (i32, i32),
    pinned: Option<i64>,
}

fn snapshot(manager: &mut Manager) -> Vec<MascotView> {
    let mut out: Vec<MascotView> = Vec::new();
    manager.apply_all(|m| {
        out.push(MascotView {
            image_set: m.image_set_name().to_string(),
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

/// 窓 `w` を holder = index 0 に pin した Manager を作る。
///
/// `anchor` は保持マスコットの初期アンカー。`behavior` には下端掴み行為
/// （`ClimbIEBottom` 等）を渡す。これにより tick 後の落下 clamp が早期 return し、
/// **追従の有無だけを観測できる**（clamp による上書きで追従欠落が隠れない）。
fn holder_manager(
    anchor: (i32, i32),
    w: Rect,
    behavior: &str,
) -> (Manager, Rc<RefCell<FakeState>>) {
    let (env, state) = env();
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("TestSet", anchor));
    manager.tick(Instant::now());

    {
        let mut st = state.borrow_mut();
        st.at_point = Some((PIN_ID, w));
        st.set_frame(PIN_ID, w);
    }
    manager.set_pin_dropped_window_allowed(true);
    manager
        .mouse_released_at(0, (w.left + 10, w.top + 10))
        .expect("mouse_released_at ok");
    manager.set_behavior_at(0, behavior);
    (manager, state)
}

// =====================================================================
// 1. ゆっくり純並進 → 窓下辺へ厳密追従
// =====================================================================

#[test]
fn manager_follows_slow_pure_translation_on_window_bottom() {
    let w = rect(100, 100, 400, 300);
    let (mut manager, state) = holder_manager((250, 300), w, "ClimbIEBottom");

    // 横 +50 / 縦 +20 の純並進（サイズ不変・最大変位 50 ≤ 80）。
    state
        .borrow_mut()
        .set_frame(PIN_ID, rect(150, 120, 450, 320));
    manager.tick(Instant::now());

    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(m.pinned, Some(PIN_ID), "pin は維持される");
    assert_eq!(
        m.anchor,
        (300, 320),
        "新矩形の下辺に厳密追従する（x は窓の横移動に比例・y は新 bottom）"
    );
}

// =====================================================================
// 2. 速い純並進（>80px/tick）→ 追従しない（案Y）
// =====================================================================

#[test]
fn manager_does_not_follow_fast_pure_translation_over_80px() {
    let w = rect(100, 100, 400, 300);
    let (mut manager, state) = holder_manager((250, 300), w, "ClimbIEBottom");

    // 横 +100 の純並進（最大変位 100 > 80）。追従せず既存ガードへ委ねる。
    state
        .borrow_mut()
        .set_frame(PIN_ID, rect(200, 100, 500, 300));
    manager.tick(Instant::now());

    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(
        m.anchor,
        (250, 300),
        "80px 超の純並進では追従しない（anchor 不変）"
    );
}

// =====================================================================
// 3. サイズ変更は 80px しきい値の対象外 → 常に追従
// =====================================================================

#[test]
fn manager_follows_size_change_regardless_of_80px_threshold() {
    let w = rect(100, 100, 400, 300);
    let (mut manager, state) = holder_manager((250, 300), w, "ClimbIEBottom");

    // 最大化相当（dleft=0, dright=600, dtop=0, dbottom=500）。dbottom > 80 だが
    // サイズ変化（dleft != dright）なので常に追従。
    state
        .borrow_mut()
        .set_frame(PIN_ID, rect(100, 100, 1000, 800));
    manager.tick(Instant::now());

    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    // 旧幅 [100,400]（幅 300）での x=250 は旧 left から 50%。新幅 [100,1000]（幅 900）へ
    // 比例写像すると x = 100 + 0.5*900 = 550。y は新 bottom = 800。
    assert_eq!(
        m.anchor,
        (550, 800),
        "サイズ変更時は常に厳密追従し x は比例式・y は新 bottom"
    );
}

// =====================================================================
// 4. アンカーが前 tick の窓下辺上に無い → 追従しない
// =====================================================================

#[test]
fn manager_does_not_follow_when_anchor_above_window_bottom() {
    let w = rect(100, 100, 400, 300);
    // anchor.y = 250 < 前 tick の下辺 300。
    let (mut manager, state) = holder_manager((250, 250), w, "ClimbIEBottom");

    state
        .borrow_mut()
        .set_frame(PIN_ID, rect(150, 120, 450, 320));
    manager.tick(Instant::now());

    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(
        m.anchor,
        (250, 250),
        "前 tick の下辺上（y == old_bottom）に無いアンカーは追従しない"
    );
}

#[test]
fn manager_does_not_follow_when_anchor_outside_horizontal_range() {
    let w = rect(100, 100, 400, 300);
    // anchor.x = 50 < 窓 left = 100（下辺の y は一致・x が範囲外）。
    let (mut manager, state) = holder_manager((50, 300), w, "ClimbIEBottom");

    state
        .borrow_mut()
        .set_frame(PIN_ID, rect(150, 100, 450, 300));
    manager.tick(Instant::now());

    let snap = snapshot(&mut manager);
    let m = find_set(&snap, "TestSet");
    assert_eq!(
        m.anchor,
        (50, 300),
        "窓の水平範囲外のアンカーは追従しない（下端 clamp も働かない）"
    );
}
