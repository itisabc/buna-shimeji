//! 機能 #30 バグ修正「最大化窓にドロップすると最大化窓が TOPMOST 固定され、
//! 落下中 holder 以外のマスコットが窓の後ろに隠れる」の契約テスト（TDD RED）。
//!
//! == 固定する契約（公開インターフェースのみ）==
//! `OsSource` に追加される `fn is_window_maximized(&self, id: i64) -> bool`
//! （既定実装は false）を観測点として、`Environment` の以下の振る舞いを固定する:
//!
//! 1. ドロップ抑止（案A）: 最大化中の窓を `pin_window(id, holder)` すると false を返し、
//!    `set_window_topmost` を呼ばない・pin 状態も空のまま。
//! 2. 既存 pin の温存: 既に別窓の pin がある状態で最大化窓へドロップすると、新規 pin は
//!    せず既存 pin が維持される（ガードは `unpin_window()` より前 = 既存 pin を剥がさない）。
//! 3. 案B（自動解除）: pin 中に次 tick で最大化されたら自動 unpin。
//!    副作用は既存の自動 unpin（window_frame=None 経路）と同一:
//!    `was_topmost==false` は `set_window_topmost(id,false)` + `activate_window(id)`、
//!    `was_topmost==true` は activate のみ。
//! 4. 回帰（非最大化）: 通常窓は従来どおり pin され、tick を重ねても解除されない。
//! 5. R15 維持: 他ウィンドウが最大化されても、固定対象（非最大化）の pin は解除されない
//!    （自動解除は固定対象窓 W 自身の最大化のみを条件とする）。
//!
//! 契約5（`is_window_maximized` をオーバーライドしない FakeSource でも既存 pin フローが
//! 成立 = trait の既定実装が存在）は既存の pin 系テスト群が同型のダブルで担保しており、
//! 本ファイルでは専用テストを置かない（`cargo test` 全体の回帰で確認する）。
//! 本ファイルの FakeSource が `is_window_maximized` を impl する事実自体が、メソッドが
//! trait に存在する（既定実装が無くてもよい）コンパイル契約を固定する。
//!
//! 実 Win32 依存（`IsWindow` / `IsZoomed` / `SetWindowPos`）は自動テスト不能のため、
//! FakeSource で観測できる振る舞いのみを検証する（実機確認は手動に委ねる）。
//! ハーネスは `tests/pin_unpin_activate_test.rs` を踏襲する（独立コンパイル単位のため重複許容）。

use std::cell::RefCell;
use std::rc::Rc;

use shimeji::app::environment::{Environment, OsSource};
use shimeji::mascot::Rect;

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

// =====================================================================
// FakeSource（maximized / topmost / activate の呼び出し記録つき）
// =====================================================================

#[derive(Default)]
struct FakeState {
    monitors: Vec<(Rect, Rect)>,
    /// `window_frame` の応答（存在しない id は None）。
    frames: Vec<(i64, Rect)>,
    /// `is_window_topmost` が返す現在値（既定 false）。
    topmost: Vec<(i64, bool)>,
    /// `set_window_topmost` 成功可否（UIPI 保護窓相当 = false）。
    set_topmost_ok: bool,
    /// `set_window_topmost` の呼び出し列。
    set_topmost_calls: Vec<(i64, bool)>,
    /// `activate_window` の呼び出し列。
    activate_calls: Vec<i64>,
    /// `is_window_maximized` が返す現在値（既定 false・本バグの制御点）。
    maximized: Vec<(i64, bool)>,
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

    fn is_maximized(&self, id: i64) -> bool {
        self.maximized
            .iter()
            .find(|(mid, _)| *mid == id)
            .map(|(_, v)| *v)
            .unwrap_or(false)
    }

    fn set_maximized(&mut self, id: i64, value: bool) {
        match self.maximized.iter_mut().find(|(mid, _)| *mid == id) {
            Some((_, slot)) => *slot = value,
            None => self.maximized.push((id, value)),
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
        Vec::new()
    }

    fn raise_window(&self, _id: i64) {}

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

    fn activate_window(&self, id: i64) -> bool {
        self.state.borrow_mut().activate_calls.push(id);
        true
    }

    // ---- 本バグの追加メソッド（オーバーライドして最大化状態を制御する）----

    fn is_window_maximized(&self, id: i64) -> bool {
        self.state.borrow().is_maximized(id)
    }
}

/// 単一モニタ (0,0,1920,1080) / work area (0,0,1920,1040)。
/// `set_window_topmost` は既定で成功。
fn env_default() -> (Environment, Rc<RefCell<FakeState>>) {
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
// 契約1: 案A（最大化窓へのドロップ抑止）
// =====================================================================

#[test]
fn pin_window_is_rejected_for_maximized_window() {
    let (env, state) = env_default();
    let w = rect(0, 0, 1920, 1080);
    {
        let mut st = state.borrow_mut();
        st.set_frame(30, w);
        st.set_maximized(30, true);
    }

    assert!(
        !env.pin_window(30, 0),
        "最大化中の窓への pin_window は false を返す（案A: ドロップ抑止）"
    );
    assert!(
        env.pinned_window().is_none(),
        "最大化窓では pin 状態が立たない"
    );
    assert!(
        state.borrow().set_topmost_calls.is_empty(),
        "最大化窓を SetWindowPos(HWND_TOPMOST) しない（topmost 化しない）"
    );
    assert!(
        state.borrow().activate_calls.is_empty(),
        "pin しないため activate_window も呼ばれない"
    );
}

// =====================================================================
// 契約2: 既存 pin の温存（ガードは unpin より前）
// =====================================================================

#[test]
fn pin_attempt_on_maximized_window_keeps_existing_pin() {
    let (env, state) = env_default();
    let pinned_window = rect(100, 100, 400, 300);
    let maximized_window = rect(0, 0, 1920, 1080);
    {
        let mut st = state.borrow_mut();
        st.set_frame(1, pinned_window);
        st.set_frame(2, maximized_window);
        st.set_maximized(2, true);
    }

    assert!(
        env.pin_window(1, 0),
        "非最大化の窓 1 は通常どおり pin される"
    );
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(1, true)],
        "前提: 既存 pin で (1, true) が記録される"
    );

    assert!(!env.pin_window(2, 1), "最大化窓 2 への pin_window は false");

    let pinned = env.pinned_window().expect("既存 pin が温存される");
    assert_eq!(pinned.id, 1, "既存 pin（窓 1）が維持される");
    assert_eq!(pinned.holder, 0, "既存 pin の holder も変わらない");
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(1, true)],
        "既存 pin を剥がさず（(1,false) 無し）、最大化窓も topmost 化しない（(2,true) 無し）"
    );
    assert!(
        state.borrow().activate_calls.is_empty(),
        "既存 pin を解除しないため activate_window は呼ばれない"
    );
}

// =====================================================================
// 契約3: 案B（pin 中に最大化されたら次 tick で自動解除）
// =====================================================================

#[test]
fn tick_auto_unpins_when_pinned_window_becomes_maximized() {
    let (env, state) = env_default();
    let w = rect(100, 100, 400, 300);
    {
        let mut st = state.borrow_mut();
        st.set_frame(30, w);
        st.set_maximized(30, false);
    }
    assert!(env.pin_window(30, 0), "前提: 非最大化の間は pin できる");
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "前提: pin で (30, true) が記録される"
    );

    // 次 tick までに窓が最大化された。
    state.borrow_mut().set_maximized(30, true);
    env.tick();

    assert!(
        env.pinned_window().is_none(),
        "pin 中に最大化されたら自動 unpin される（案B）"
    );
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "自動 unpin は window_frame=None 経路と同じく was_topmost 規則で topmost を戻す"
    );
    assert_eq!(
        state.borrow().activate_calls,
        vec![30],
        "自動 unpin でも対象窓をアクティブ化する（既存自動解除と同一副作用）"
    );
}

#[test]
fn tick_auto_unpin_maximized_keeps_pre_existing_topmost_and_activates() {
    let (env, state) = env_default();
    let w = rect(100, 100, 400, 300);
    {
        let mut st = state.borrow_mut();
        st.set_frame(30, w);
        st.set_maximized(30, false);
        // pin 前から元々 TOPMOST。
        st.set_topmost_state(30, true);
    }
    assert!(env.pin_window(30, 0));
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "前提: 元 TOPMOST でも pin は (30, true) を記録する"
    );

    state.borrow_mut().set_maximized(30, true);
    env.tick();

    assert!(env.pinned_window().is_none(), "最大化で自動 unpin される");
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "was_topmost=true の窓は我々が剥がさない（(30,false) を呼ばない）"
    );
    assert_eq!(
        state.borrow().activate_calls,
        vec![30],
        "元から TOPMOST でも自動解除時にアクティブ化する"
    );
}

// =====================================================================
// 契約4: 回帰（非最大化窓の pin は tick を重ねても維持される）
// =====================================================================

#[test]
fn normal_window_pin_persists_across_ticks() {
    let (env, state) = env_default();
    let w = rect(100, 100, 400, 300);
    {
        let mut st = state.borrow_mut();
        st.set_frame(30, w);
        // 既定 false（非最大化）を明示。
        st.set_maximized(30, false);
    }

    assert!(
        env.pin_window(30, 0),
        "非最大化の通常窓はドロップで pin される"
    );
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "通常窓は topmost 化される"
    );

    // 最大化されないまま tick を重ねても解除されない。
    env.tick();
    env.tick();

    let pinned = env
        .pinned_window()
        .expect("非最大化の pin は tick 後も維持される（意図せず解除しない）");
    assert_eq!(pinned.id, 30, "pin 対象 id は維持される");
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "tick を重ねても set_window_topmost は追加で呼ばれない（剥がさない）"
    );
    assert!(
        state.borrow().activate_calls.is_empty(),
        "維持中は activate_window を呼ばない"
    );
}

// =====================================================================
// 契約5（R15）: 他ウィンドウの最大化では固定対象（非最大化）の pin を解除しない
// =====================================================================

#[test]
fn other_window_maximized_does_not_unpin_non_maximized_target() {
    let (env, state) = env_default();
    let w = rect(100, 100, 400, 300);
    {
        let mut st = state.borrow_mut();
        st.set_frame(30, w);
        st.set_maximized(30, false);
    }
    assert!(env.pin_window(30, 0), "前提: 非最大化の窓 30 を pin");

    // 別ウィンドウ 99 だけが最大化された（固定対象 30 は非最大化のまま）。
    state.borrow_mut().set_maximized(99, true);
    env.tick();

    let pinned = env
        .pinned_window()
        .expect("R15: 他ウィンドウの最大化では固定対象（非最大化）の pin を解除しない");
    assert_eq!(pinned.id, 30, "固定対象 id は維持される");
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "他ウィンドウの最大化で topmost を剥がさない（(30,false) を呼ばない）"
    );
    assert!(
        state.borrow().activate_calls.is_empty(),
        "解除しないため activate_window を呼ばない"
    );
}
