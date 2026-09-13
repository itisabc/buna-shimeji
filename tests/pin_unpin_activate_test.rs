//! 機能 #30 追補「pin 解除時に対象窓をアクティブ化する（unpin-activate）」の
//! 契約テスト。
//!
//! 背景: `set_window_topmost(W, false)`（HWND_NOTOPMOST）は W を非トップモスト帯の
//! 最前面へ置くため、非フォアグラウンドの W を pin 解除すると手前に残って見える。
//! 解除後に対象窓 W をアクティブ化（Win32 SetForegroundWindow）すれば、背後の窓の
//! クリックが本物のアクティブ切替になり W が背後へ回る。
//!
//! == 固定する契約（公開インターフェースのみ）==
//! 1. `OsSource` に `activate_window(&self, id: i64) -> bool` を追加し、既定実装は
//!    false（既存のテストダブルが壊れない = 既定実装が存在すること）。
//! 2. `Environment::unpin_window()` は pin が存在した場合に
//!    `self.source.activate_window(pin.id)` を呼ぶ:
//!    - pin 無しの解除 → activate しない
//!    - `was_topmost == false` の解除 → `set_window_topmost(id, false)` と activate の両方
//!    - `was_topmost == true` の解除 → `set_window_topmost` は呼ばず activate は呼ぶ
//! 3. 解除の全経路（保持しめじを掴んで引きはがす / トレイのトグル OFF / DismissAll /
//!    Reload / 自動解除（窓消失））で activate が起きる。
//!
//! 実 Win32（SetForegroundWindow / SetWindowPos / EnumWindows）は自動テスト不能のため、
//! FakeSource で観測できる範囲のみを検証する（実機確認は design §1.10(z-4) に委ねる）。
//! ハーネスは `tests/pin_unpin_test.rs` を踏襲する（独立コンパイル単位のため重複を許容）。
//!
//! `activate_window` とその呼び出しは実装済み（本ファイルは Green）。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use simeji::app::environment::{Environment, OsSource};
use simeji::app::manager::Manager;
use simeji::app::reload::ReloadMaterial;
use simeji::config::{BehaviorDef, BehaviorEntry, BehaviorsConfig, SequenceChild, VarMap};
use simeji::mascot::behavior::{
    Action, ActionError, BehaviorError, BehaviorFactory, BehaviorTable,
};
use simeji::mascot::{EnvironmentView, Mascot, Rect, Rng};
use simeji::render::imageset::ImageSet;
use simeji::tray::{apply_tray_command, AllowedKind, Settings, TrayCommand, TrayContext};

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
// FakeSource（topmost / activate の呼び出し記録つき）
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
    /// `set_window_topmost` の呼び出し列。
    set_topmost_calls: Vec<(i64, bool)>,
    /// `activate_window` の呼び出し列（本追補の観測点）。
    activate_calls: Vec<i64>,
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

    fn activate_window(&self, id: i64) -> bool {
        self.state.borrow_mut().activate_calls.push(id);
        true
    }
}

/// `activate_window` を上書きしない source。trait の既定実装（false）が存在する
/// コンパイル契約を固定する（既定実装が無ければ本ファイルはコンパイルできない）。
struct DefaultActivateSource {
    state: Rc<RefCell<FakeState>>,
}

impl OsSource for DefaultActivateSource {
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

/// 常に has_next=true・next no-op の action（anchor を動かさない）。
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

/// 窓 30 を pin 済みの Manager（holder = index 0）。
/// 前提として set_topmost_calls = [(30, true)]・activate 未呼び出し・ミラー = Some(30)。
fn pinned_manager() -> (Manager, Rc<RefCell<FakeState>>, Rect) {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("TestSet", (50, 100)));
    manager.tick(Instant::now());
    pin_index_zero(&mut manager, &state, w);

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(PIN_ID, true)],
        "前提: pin で (30, true) が 1 回"
    );
    assert!(
        state.borrow().activate_calls.is_empty(),
        "前提: pin 段階では activate しない"
    );
    (manager, state, w)
}

// =====================================================================
// apply_tray_command 用の temp dir / TrayContext
// =====================================================================

/// settings.toml 保存先を与える使い捨てディレクトリ（Drop で再帰削除）。
struct TempDir {
    root: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "simeji_pinunpin_activate_{}_{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("temp dir を作れる");
        TempDir { root }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn context(home: &TempDir) -> TrayContext {
    TrayContext {
        conf_dir: home.root.clone(),
        img_dir: home.root.clone(),
        image_sets: Vec::new(),
    }
}

// =====================================================================
// 1. unpin_window: pin 無しでは activate しない
// =====================================================================

#[test]
fn unpin_without_pin_does_not_activate() {
    let (env, state) = env();

    env.unpin_window();
    env.unpin_window();

    assert!(env.pinned_window().is_none());
    assert!(
        state.borrow().activate_calls.is_empty(),
        "pin が無い解除では activate_window を呼ばない"
    );
}

// =====================================================================
// 2. unpin_window: was_topmost=false は剥がして activate
// =====================================================================

#[test]
fn unpin_activates_and_restores_non_topmost_window() {
    let (env, state) = env();
    state.borrow_mut().set_frame(PIN_ID, rect(10, 10, 200, 200));
    assert!(env.pin_window(PIN_ID, 0));
    assert!(
        state.borrow().activate_calls.is_empty(),
        "pin 段階では activate しない"
    );

    env.unpin_window();

    assert!(env.pinned_window().is_none(), "unpin で pin が解除される");
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(PIN_ID, true), (PIN_ID, false)],
        "was_topmost=false の窓は剥がすときに set_window_topmost(id,false) を呼ぶ"
    );
    assert_eq!(
        state.borrow().activate_calls,
        vec![PIN_ID],
        "解除後に対象窓をアクティブ化する"
    );
}

// =====================================================================
// 3. unpin_window: was_topmost=true でも activate する（剥がさない）
// =====================================================================

#[test]
fn unpin_activates_pre_existing_topmost_without_stripping() {
    let (env, state) = env();
    state.borrow_mut().set_frame(PIN_ID, rect(10, 10, 200, 200));
    state.borrow_mut().set_topmost_state(PIN_ID, true);
    assert!(env.pin_window(PIN_ID, 0));

    env.unpin_window();

    assert!(env.pinned_window().is_none());
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(PIN_ID, true)],
        "was_topmost=true の窓は我々が剥がさない（false を呼ばない）"
    );
    assert_eq!(
        state.borrow().activate_calls,
        vec![PIN_ID],
        "元から TOPMOST の窓でも解除時にアクティブ化する"
    );
}

// =====================================================================
// 4. OsSource::activate_window の既定実装（上書きしない source でも通る）
// =====================================================================

#[test]
fn os_source_without_activate_override_defaults_and_unpins() {
    let state = Rc::new(RefCell::new(FakeState {
        monitors: vec![(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1040))],
        set_topmost_ok: true,
        ..Default::default()
    }));
    let env = Environment::new(DefaultActivateSource {
        state: state.clone(),
    });
    state.borrow_mut().set_frame(PIN_ID, rect(10, 10, 200, 200));

    assert!(
        env.pin_window(PIN_ID, 0),
        "activate_window を上書きしなくても pin/unpin は通る（既定実装が存在）"
    );
    env.unpin_window();

    assert!(env.pinned_window().is_none());
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(PIN_ID, true), (PIN_ID, false)]
    );
}

// =====================================================================
// 5. 解除経路: 保持しめじを掴んで引きはがす
// =====================================================================

#[test]
fn pull_off_activates_window() {
    let (mut manager, state, w) = pinned_manager();

    // 保持者（index 0）を掴む = 引きはがし → unpin。
    manager
        .mouse_pressed_at(0, (w.left + 10, w.top + 10))
        .expect("mouse_pressed_at ok");

    assert!(
        manager.pinned_holder().is_none(),
        "引きはがしで pin が解除される"
    );
    assert_eq!(
        state.borrow().activate_calls,
        vec![PIN_ID],
        "引きはがし解除で対象窓をアクティブ化する"
    );
}

// =====================================================================
// 6. 解除経路: トレイのトグル OFF
// =====================================================================

#[test]
fn tray_toggle_off_activates_window() {
    let home = TempDir::new("toggle_off");
    let (mut manager, state, _w) = pinned_manager();
    let mut settings = Settings::default();
    settings.allowed.pin_dropped_window = true;

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::SetAllowed(AllowedKind::PinDroppedWindow, false),
        &context(&home),
    );

    assert!(!settings.allowed.pin_dropped_window);
    assert_eq!(
        state.borrow().activate_calls,
        vec![PIN_ID],
        "トレイの pin トグル OFF で対象窓をアクティブ化する"
    );
}

// =====================================================================
// 7. 解除経路: トレイの DismissAll
// =====================================================================

#[test]
fn tray_dismiss_all_activates_window() {
    let home = TempDir::new("dismiss");
    let (mut manager, state, _w) = pinned_manager();
    let mut settings = Settings::default();

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::DismissAll,
        &context(&home),
    );

    assert_eq!(
        state.borrow().activate_calls,
        vec![PIN_ID],
        "DismissAll の即時解除で対象窓をアクティブ化する"
    );
}

// =====================================================================
// 8. 解除経路: Reload
// =====================================================================

#[test]
fn reload_activates_window() {
    let (mut manager, state, _w) = pinned_manager();

    let materials = vec![ReloadMaterial {
        name: "TestSet".to_string(),
        image_set: empty_image_set("TestSet"),
        table: table(vec![row("Idle", 100)]),
        disabled_animations: Vec::new(),
    }];
    manager.reload(materials);

    assert_eq!(
        state.borrow().activate_calls,
        vec![PIN_ID],
        "Reload の解除で対象窓をアクティブ化する"
    );
}

// =====================================================================
// 9. 解除経路: 自動解除（窓消失を tick が検出）
// =====================================================================

#[test]
fn auto_unpin_activates_window() {
    let (mut manager, state, _w) = pinned_manager();

    // pin 窓が消える（クローズ / 不可視化相当）→ 次 tick の env.tick が自動 unpin。
    state.borrow_mut().clear_frame(PIN_ID);
    manager.tick(Instant::now());

    assert!(
        manager.pinned_holder().is_none(),
        "窓消失で pin が自動解除される"
    );
    assert_eq!(
        state.borrow().activate_calls,
        vec![PIN_ID],
        "自動解除でも対象窓をアクティブ化する"
    );
}
