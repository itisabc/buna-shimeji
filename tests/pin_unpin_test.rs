//! 機能 #30 / タスク 30-5: ピン解除フックの配線 契約テスト。
//!
//! 設計正本: `.tmp/design.md` §1.10(z)（item 3・4・5）。計画: `.tmp/plan.md` 30-5。
//! pin の生成側（30-3/30-4）は `tests/pin_drop_test.rs` で GREEN 済み。
//! 本ファイルは「pin 済み Manager に解除操作を適用したときの解除フック」だけを
//! FakeSource で観測する（実 Win32 は自動テスト不能・実機確認は design 末尾に委ねる）。
//!
//! == 固定する契約 ==
//! トグル ON で窓 30 を pin 済みの Manager に対し、以下のいずれかを適用すると
//! **保持窓が TOPMOST から戻され（FakeSource に `set_window_topmost(30, false)` が
//! 記録され）、全 Mascot のミラー（`pinned_window()`）が即座にクリアされる**:
//!
//! 1. `apply_tray_command(... TrayCommand::SetAllowed(AllowedKind::PinDroppedWindow, false) ...)`
//! 2. `apply_tray_command(... TrayCommand::RestoreWindows ...)`
//! 3. `apply_tray_command(... TrayCommand::DismissAll ...)`
//! 4. `Manager::reload(materials)`
//!
//! 追加の境界契約:
//! - pin 前から TOPMOST だった窓（`was_topmost == true`）は解除時に
//!   `set_window_topmost(id, false)` を**呼ばない**（我々が付けたものではない）。
//! - `TrayCommand::SetAllowed(AllowedKind::PinDroppedWindow, true)` は
//!   それ自体では pin を立てないし、既存 pin を解除しない（純粋なトグル設定のみ）。
//!
//! 30-5 配線済みのため本ファイルは全 GREEN。テストは既存の公開 API のみを使う。
//! 実 Win32 の TOPMOST 実効・追従・CPU・クラッシュ残存は実機確認に委ねる。

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
// 合成ヘルパ（pin_drop_test.rs 踏襲・ヘルパ重複は許容）
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

/// マスコット集合の pin ミラー一覧（apply_all 経由の公開 API のみ）。
fn pinned_snapshot(manager: &mut Manager) -> Vec<Option<i64>> {
    let mut out = Vec::new();
    manager.apply_all(|m| out.push(m.pinned_window()));
    out
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
/// 前提として set_topmost_calls = [(30, true)]・ミラー = Some(30) を確認する。
fn pinned_manager() -> (Manager, Rc<RefCell<FakeState>>, Rect) {
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("TestSet", (50, 100)));
    manager.tick(Instant::now());
    pin_index_zero(&mut manager, &state, w);

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "前提: pin で (30, true) が 1 回"
    );
    assert_eq!(
        pinned_snapshot(&mut manager),
        vec![Some(30)],
        "前提: ミラーが Some(30)"
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
        let root =
            std::env::temp_dir().join(format!("simeji_pinunpin_{}_{tag}", std::process::id()));
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
// 1. トレイ SetAllowed(PinDroppedWindow, false) で解除
// =====================================================================

#[test]
fn tray_pin_toggle_off_unpins_and_clears_mirror() {
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

    assert!(
        !settings.allowed.pin_dropped_window,
        "settings 側のトグルも false になる"
    );
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "トグル OFF で保持窓を TOPMOST から戻す"
    );
    assert_eq!(
        pinned_snapshot(&mut manager),
        vec![None],
        "全 Mascot のミラーが即座にクリアされる"
    );
}

// =====================================================================
// 2. トレイ RestoreWindows で解除
// =====================================================================

#[test]
fn tray_restore_windows_unpins_and_clears_mirror() {
    let home = TempDir::new("restore");
    let (mut manager, state, _w) = pinned_manager();
    let mut settings = Settings::default();

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::RestoreWindows,
        &context(&home),
    );

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "RestoreWindows は pin を解除する（窓は画面外のみ移動対象・この窓は移動しない）"
    );
    assert!(
        state.borrow().moved.is_empty(),
        "窓 30 は画面内のため restore_windows 自体は移動しない"
    );
    assert_eq!(
        pinned_snapshot(&mut manager),
        vec![None],
        "全 Mascot のミラーが即座にクリアされる"
    );
}

// =====================================================================
// 3. トレイ DismissAll で解除
// =====================================================================

#[test]
fn tray_dismiss_all_unpins_and_clears_mirror() {
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
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "DismissAll は pin を解除する（mascot 削除は次 tick でも解除は即座）"
    );
    assert_eq!(
        pinned_snapshot(&mut manager),
        vec![None],
        "全 Mascot のミラーが即座にクリアされる"
    );
}

// =====================================================================
// 4. Manager::reload で解除
// =====================================================================

#[test]
fn reload_unpins_and_clears_mirror() {
    let (mut manager, state, _w) = pinned_manager();

    let materials = vec![ReloadMaterial {
        name: "TestSet".to_string(),
        image_set: empty_image_set("TestSet"),
        table: table(vec![row("Idle", 100)]),
        disabled_animations: Vec::new(),
    }];
    manager.reload(materials);

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true), (30, false)],
        "Reload は pin を解除する"
    );
    assert_eq!(
        pinned_snapshot(&mut manager),
        vec![None],
        "全 Mascot のミラーが即座にクリアされる"
    );
}

// =====================================================================
// 5. was_topmost 規則（元から TOPMOST の窓は剥がさない）
// =====================================================================

#[test]
fn unpin_skips_topmost_restore_when_window_was_already_topmost() {
    let home = TempDir::new("was_topmost");
    let (env, state) = env();
    let w = rect(100, 100, 400, 300);
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("TestSet", (50, 100)));
    manager.tick(Instant::now());

    // pin 前から窓 30 は TOPMOST。
    state.borrow_mut().set_topmost_state(30, true);
    pin_index_zero(&mut manager, &state, w);
    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "前提: 元 TOPMOST でも pin は (30, true) を記録する"
    );

    let mut settings = Settings::default();
    settings.allowed.pin_dropped_window = true;
    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::SetAllowed(AllowedKind::PinDroppedWindow, false),
        &context(&home),
    );

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "was_topmost=true の窓は解除時に set_window_topmost(id, false) を呼ばない"
    );
    assert_eq!(
        pinned_snapshot(&mut manager),
        vec![None],
        "ミラーはクリアされる（pin 自体は解除）"
    );
}

// =====================================================================
// 6. SetAllowed(ON) は既存 pin を解除しない
// =====================================================================

#[test]
fn tray_set_allowed_on_does_not_unpin_existing_pin() {
    let home = TempDir::new("on_keeps_pin");
    let (mut manager, state, _w) = pinned_manager();
    // 既に pin 済み（pinned_manager 内でトグル ON 済み）。
    let mut settings = Settings::default();
    settings.allowed.pin_dropped_window = true;

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::SetAllowed(AllowedKind::PinDroppedWindow, true),
        &context(&home),
    );

    assert_eq!(
        state.borrow().set_topmost_calls,
        vec![(30, true)],
        "ON 操作は解除フックを発火しない（(30, false) が記録されない）"
    );
    assert_eq!(
        pinned_snapshot(&mut manager),
        vec![Some(30)],
        "既存 pin のミラーは維持される"
    );
    assert!(settings.allowed.pin_dropped_window);
}

// =====================================================================
// 7. SetAllowed(ON) はそれ自体では pin を立てない
// =====================================================================

#[test]
fn tray_set_allowed_on_does_not_pin_by_itself() {
    let home = TempDir::new("on_no_pin");
    let (env, state) = env();
    let mut manager = make_manager(env);
    manager.add(mascot_of_set("TestSet", (50, 100)));
    manager.tick(Instant::now());
    let mut settings = Settings::default();

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::SetAllowed(AllowedKind::PinDroppedWindow, true),
        &context(&home),
    );

    assert!(
        state.borrow().set_topmost_calls.is_empty(),
        "ON 操作は解放点も窓も経由しないため pin を立てない"
    );
    assert_eq!(
        pinned_snapshot(&mut manager),
        vec![None],
        "ミラーは None のまま"
    );
    assert!(settings.allowed.pin_dropped_window);
}
