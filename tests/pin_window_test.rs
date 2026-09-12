//! 機能 #30 / タスク 30-3: Environment の pin 状態（最前面固定窓）と per-mascot
//! holder 隔離の公開契約テスト（TDD RED）。
//!
//! 設計正本: `.tmp/design.md` §1.10(z)（item 2 / item 3）。
//! 検証対象の公開 API（未実装のため cargo test は compile error = RED が正常）:
//! - `Environment::pin_window(&self, id: i64, holder: usize) -> bool`
//! - `Environment::unpin_window(&self)`
//! - `Environment::set_holder_scope(&self, holder: Option<usize>)`
//! - `Environment::pinned_window(&self) -> Option<PinnedWindow>`
//! - `pub struct PinnedWindow { pub id: i64, pub rect: Rect, pub holder: usize,
//!   pub was_topmost: bool }`
//! - `OsSource::window_frame` / `set_window_topmost` / `is_window_topmost`
//!   （`window_at_point` は 30-3 の Environment からは呼ばれず 30-4 Manager が使う
//!   ため、本テストでは FakeSource が値を持てることだけ担保しテスト対象外）
//!
//! 実 Win32 依存（EnumWindows 選別・GetWindowRect・SetWindowPos）は自動テスト不能の
//! ため扱わない（設計 §1.10(z) 手動確認に委ねる）。本テストは FakeSource による
//! 振る舞い契約のみを検証する。

use std::cell::RefCell;
use std::rc::Rc;

use simeji::app::environment::{Environment, OsSource, PinnedWindow};
use simeji::mascot::{EnvironmentView, Rect};

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

fn area_rect(state: &simeji::mascot::env::AreaState) -> Rect {
    rect(state.left, state.top, state.right, state.bottom)
}

/// pin の記録内容を一括検証する（public フィールドのみ）。
fn assert_pinned(pinned: &PinnedWindow, id: i64, holder: usize, was_topmost: bool, expected: Rect) {
    assert_eq!(pinned.id, id, "pinned.id");
    assert_eq!(pinned.holder, holder, "pinned.holder");
    assert_eq!(pinned.was_topmost, was_topmost, "pinned.was_topmost");
    assert_eq!(pinned.rect, expected, "pinned.rect");
}

// =====================================================================
// FakeSource（OsSource 実装・呼び出し記録）
// =====================================================================

/// FakeSource の可変状態（テストから Rc<RefCell<..>> 経由で差し替える）。
#[derive(Default)]
struct FakeState {
    monitors: Vec<(Rect, Rect)>,
    cursor: Option<(i32, i32)>,
    active_window: Option<(i64, Rect)>,
    windows: Vec<(i64, Rect)>,
    /// `move_window` の呼び出し列（(id, x, y)）。
    moved: Vec<(i64, i32, i32)>,
    /// `raise_window` の呼び出し列。
    raised: Vec<i64>,
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

    // ---- 30-3 で OsSource に追加されるメソッド（既定実装の上書き）----

    fn window_at_point(&self, _x: i32, _y: i32) -> Option<(i64, Rect)> {
        // 30-3 の Environment からは呼ばれない（30-4 Manager が使用）。
        None
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

/// 単一モニタ (0,0,1920,1080) / work area (0,0,1920,1040) の Environment。
/// `set_window_topmost` は既定で成功する。
fn env_default() -> (Environment, Rc<RefCell<FakeState>>) {
    let state = Rc::new(RefCell::new(FakeState {
        monitors: vec![(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1040))],
        set_topmost_ok: true,
        ..Default::default()
    }));
    let env = Environment::new(FakeSource {
        state: state.clone(),
    });
    (env, state)
}

// =====================================================================
// 1. pin / unpin と was_topmost
// =====================================================================

#[test]
fn pin_window_sets_topmost_and_records_holder() {
    let (env, state) = env_default();
    let target = rect(100, 50, 400, 300);
    state.borrow_mut().set_frame(30, target);

    assert!(
        env.pin_window(30, 7),
        "set_window_topmost 成功時は pin_window が true を返す"
    );

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "TOPMOST 化は 1 回だけ呼ばれる"
    );

    let pinned = env.pinned_window().expect("pin が記録される");
    assert_pinned(&pinned, 30, 7, false, target);
}

#[test]
fn pin_window_records_was_topmost_when_window_already_topmost() {
    let (env, state) = env_default();
    let target = rect(10, 10, 200, 200);
    state.borrow_mut().set_frame(30, target);
    state.borrow_mut().set_topmost_state(30, true);

    assert!(env.pin_window(30, 2));

    let pinned = env.pinned_window().expect("pin が記録される");
    assert!(
        pinned.was_topmost,
        "元から TOPMOST の窓は was_topmost=true を記録する"
    );
}

#[test]
fn pin_window_fails_and_does_not_pin_when_set_topmost_fails() {
    let (env, state) = env_default();
    state.borrow_mut().set_frame(30, rect(10, 10, 200, 200));
    state.borrow_mut().set_topmost_ok = false;

    assert!(
        !env.pin_window(30, 1),
        "UIPI 保護窓相当（set_window_topmost=false）では pin_window は false"
    );
    assert!(env.pinned_window().is_none(), "失敗時は pin を立てない");
    assert_eq!(state.borrow().set_topmost_calls, vec![(30, true)]);
}

#[test]
fn unpin_restores_non_topmost_window() {
    let (env, state) = env_default();
    state.borrow_mut().set_frame(30, rect(10, 10, 200, 200));
    assert!(env.pin_window(30, 0));

    env.unpin_window();

    assert!(env.pinned_window().is_none(), "unpin で pin が解除される");
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "was_topmost=false の窓は剥がすときに set_window_topmost(id,false) を呼ぶ"
    );
}

#[test]
fn unpin_does_not_strip_pre_existing_topmost() {
    let (env, state) = env_default();
    state.borrow_mut().set_frame(30, rect(10, 10, 200, 200));
    state.borrow_mut().set_topmost_state(30, true);
    assert!(env.pin_window(30, 0));

    env.unpin_window();

    assert!(env.pinned_window().is_none());
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "was_topmost=true の窓は我々が剥がさない（false を呼ばない）"
    );
}

#[test]
fn unpin_is_idempotent() {
    let (env, state) = env_default();
    // pin が無い状態の unpin は何もしない（panic しない）。
    env.unpin_window();
    env.unpin_window();
    assert!(state.borrow().set_topmost_calls.is_empty());

    state.borrow_mut().set_frame(30, rect(10, 10, 200, 200));
    assert!(env.pin_window(30, 0));
    env.unpin_window();
    // 二重解除ガード: 2 回目の unpin は false を再送しない。
    env.unpin_window();

    assert!(env.pinned_window().is_none());
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)]
    );
}

#[test]
fn pinning_another_window_releases_previous_first() {
    let (env, state) = env_default();
    state.borrow_mut().set_frame(1, rect(0, 0, 100, 100));
    state.borrow_mut().set_frame(2, rect(300, 300, 500, 500));

    assert!(env.pin_window(1, 0));
    assert!(env.pin_window(2, 0));

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(1, true), (1, false), (2, true)],
        "旧 pin を was_topmost 規則で解除してから新 pin を張る"
    );
    assert_pinned(
        &env.pinned_window().expect("新 pin"),
        2,
        0,
        false,
        rect(300, 300, 500, 500),
    );
}

#[test]
fn pinning_another_window_keeps_previous_pre_existing_topmost() {
    let (env, state) = env_default();
    state.borrow_mut().set_frame(1, rect(0, 0, 100, 100));
    state.borrow_mut().set_frame(2, rect(300, 300, 500, 500));
    state.borrow_mut().set_topmost_state(1, true);

    assert!(env.pin_window(1, 0));
    assert!(env.pin_window(2, 0));

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(1, true), (2, true)],
        "元から TOPMOST の旧 pin は解除時に剥がさない"
    );
    assert_eq!(env.pinned_window().expect("新 pin").id, 2);
}

// =====================================================================
// 2. tick による pin 追跡と自動解除
// =====================================================================

#[test]
fn tick_tracks_moved_pinned_window_rect_for_holder() {
    let (env, state) = env_default();
    let first = rect(10, 20, 110, 120);
    state.borrow_mut().set_frame(30, first);
    assert!(env.pin_window(30, 0));

    env.tick();
    env.set_holder_scope(Some(0));
    assert_eq!(area_rect(&env.active_window()), first);
    assert_eq!(env.active_window_id(), 30);

    // 窓が動いたら毎 tick の window_frame で追従する。
    let second = rect(60, 20, 160, 120);
    state.borrow_mut().set_frame(30, second);
    env.tick();

    assert_eq!(
        area_rect(&env.active_window()),
        second,
        "holder から見た active_window は最新 frame に一致する"
    );
}

#[test]
fn tick_records_pinned_rect_delta_for_follow() {
    let (env, state) = env_default();
    let first = rect(10, 20, 110, 120);
    state.borrow_mut().set_frame(30, first);
    assert!(env.pin_window(30, 0));
    env.tick();

    let second = rect(60, 20, 160, 120);
    state.borrow_mut().set_frame(30, second);
    env.tick();

    env.set_holder_scope(Some(0));
    let active = env.active_window();
    // 既存 AreaState の delta 追従（border_move）に載る。
    assert_eq!(active.dleft, second.left - first.left);
    assert_eq!(active.dright, second.right - first.right);
    assert_eq!(active.dtop, 0);
    assert_eq!(active.dbottom, 0);
}

#[test]
fn tick_auto_unpins_when_window_frame_disappears() {
    let (env, state) = env_default();
    state.borrow_mut().set_frame(30, rect(10, 10, 200, 200));
    assert!(env.pin_window(30, 0));
    env.tick();

    // 窓クローズ / 不可視 / 最小化 / クローク相当。
    state.borrow_mut().clear_frame(30);
    env.tick();

    assert!(
        env.pinned_window().is_none(),
        "window_frame=None で自動 unpin される"
    );
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "自動 unpin でも was_topmost 規則で topmost を戻す"
    );
}

#[test]
fn tick_auto_unpin_keeps_pre_existing_topmost() {
    let (env, state) = env_default();
    state.borrow_mut().set_frame(30, rect(10, 10, 200, 200));
    state.borrow_mut().set_topmost_state(30, true);
    assert!(env.pin_window(30, 0));
    env.tick();

    state.borrow_mut().clear_frame(30);
    env.tick();

    assert!(env.pinned_window().is_none());
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "元から TOPMOST の窓は自動 unpin でも剥がさない"
    );
}

// =====================================================================
// 3. per-mascot holder 隔離（EnvironmentView の gating）
// =====================================================================

#[test]
fn holder_scope_reports_pinned_window_as_active() {
    let (env, state) = env_default();
    let other = rect(500, 500, 700, 700);
    let pin_rect = rect(10, 20, 110, 120);
    {
        let mut st = state.borrow_mut();
        st.active_window = Some((999, other));
        st.set_frame(30, pin_rect);
    }
    assert!(env.pin_window(30, 3));
    env.tick();

    env.set_holder_scope(Some(3));
    assert_eq!(env.active_window_id(), 30, "holder には pin 窓 id を報告");
    assert_eq!(area_rect(&env.active_window()), pin_rect);
    env.move_active_window(1, 2);
    assert_eq!(
        state.borrow().moved,
        vec![(30, 1, 2)],
        "holder の move_active_window は pin 窓を動かす"
    );

    // 非保持（None）では従来どおり OsSource の実アクティブ窓。
    env.set_holder_scope(None);
    assert_eq!(env.active_window_id(), 999);
    assert_eq!(area_rect(&env.active_window()), other);
    env.move_active_window(3, 4);
    assert_eq!(state.borrow().moved, vec![(30, 1, 2), (999, 3, 4)]);
}

#[test]
fn non_holder_scope_never_sees_or_moves_pinned_window() {
    let (env, state) = env_default();
    let other = rect(500, 500, 700, 700);
    {
        let mut st = state.borrow_mut();
        st.active_window = Some((999, other));
        st.set_frame(30, rect(10, 20, 110, 120));
    }
    assert!(env.pin_window(30, 3));
    env.tick();

    // 他マスコット（holder=4）からは pin 窓を activeIE と見なさない。
    env.set_holder_scope(Some(4));
    assert_eq!(env.active_window_id(), 999);
    assert_eq!(area_rect(&env.active_window()), other);
    env.move_active_window(5, 6);
    assert_eq!(
        state.borrow().moved,
        vec![(999, 5, 6)],
        "非保持者は pin 窓を移動できない"
    );

    // tick はグローバルな active window を pin で置換しない。
    env.tick();
    env.set_holder_scope(None);
    assert_eq!(env.active_window_id(), 999);
    assert_eq!(area_rect(&env.active_window()), other);
}

#[test]
fn holder_scope_without_pin_uses_active_window() {
    let (env, state) = env_default();
    let other = rect(500, 500, 700, 700);
    state.borrow_mut().active_window = Some((999, other));
    env.tick();

    env.set_holder_scope(Some(3));
    assert_eq!(env.active_window_id(), 999);
    assert_eq!(area_rect(&env.active_window()), other);
    env.move_active_window(7, 8);
    assert_eq!(state.borrow().moved, vec![(999, 7, 8)]);
}

// =====================================================================
// 4. 30-8a: PinnedWindow の矩形差分公開と clear
//    （Manager が前 tick 差分からアンカーを厳密追従するための契約）
// =====================================================================

#[test]
fn pinned_window_exposes_horizontal_delta_after_tick() {
    let (env, state) = env_default();
    state.borrow_mut().set_frame(30, rect(10, 20, 110, 120));
    assert!(env.pin_window(30, 0));
    env.tick(); // 初回は差分 0 のベースライン

    // 水平 50px の純並進。
    state.borrow_mut().set_frame(30, rect(60, 20, 160, 120));
    env.tick();

    let pinned = env.pinned_window().expect("pin が維持される");
    assert_eq!(pinned.rect, rect(60, 20, 160, 120), "rect は最新 frame");
    assert_eq!(pinned.dleft, 50, "dleft = 新 left - 旧 left");
    assert_eq!(pinned.dright, 50, "dright = 新 right - 旧 right");
    assert_eq!(pinned.dtop, 0, "水平移動では dtop = 0");
    assert_eq!(pinned.dbottom, 0, "水平移動では dbottom = 0");
}

#[test]
fn pinned_window_exposes_vertical_delta_after_tick() {
    let (env, state) = env_default();
    state.borrow_mut().set_frame(30, rect(10, 20, 110, 120));
    assert!(env.pin_window(30, 0));
    env.tick();

    // 垂直 40px の純並進。
    state.borrow_mut().set_frame(30, rect(10, 60, 110, 160));
    env.tick();

    let pinned = env.pinned_window().expect("pin が維持される");
    assert_eq!(pinned.dleft, 0, "垂直移動では dleft = 0");
    assert_eq!(pinned.dright, 0, "垂直移動では dright = 0");
    assert_eq!(pinned.dtop, 40, "dtop = 新 top - 旧 top");
    assert_eq!(pinned.dbottom, 40, "dbottom = 新 bottom - 旧 bottom");
}

#[test]
fn clear_pinned_delta_zeroes_deltas_without_moving_rect() {
    let (env, state) = env_default();
    state.borrow_mut().set_frame(30, rect(10, 20, 110, 120));
    assert!(env.pin_window(30, 0));
    env.tick();

    state.borrow_mut().set_frame(30, rect(60, 20, 160, 120));
    env.tick();
    assert_eq!(
        env.pinned_window().expect("pin").dleft,
        50,
        "前提: tick で差分が載っている"
    );

    env.clear_pinned_delta();

    let pinned = env.pinned_window().expect("pin が維持される");
    assert_eq!(pinned.dleft, 0, "dleft がクリアされる");
    assert_eq!(pinned.dtop, 0, "dtop がクリアされる");
    assert_eq!(pinned.dright, 0, "dright がクリアされる");
    assert_eq!(pinned.dbottom, 0, "dbottom がクリアされる");
    assert_eq!(
        pinned.rect,
        rect(60, 20, 160, 120),
        "rect 自体は変化しない（delta のみクリア）"
    );
}

#[test]
fn clear_pinned_delta_without_pin_does_not_panic() {
    let (env, _state) = env_default();
    // pin が無い状態でも panic しない（no-op）。
    env.clear_pinned_delta();
    assert!(env.pinned_window().is_none());
}
