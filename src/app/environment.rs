//! Environment — [`EnvironmentView`](crate::mascot::EnvironmentView) の OS 供給実装
//! （design.md §1.7 / §1.5・タスク #8）。
//!
//! Java 正本（.tmp/java-ref/）を仕様として逐語移植する:
//! - `AbstractEnvironment.tick` L167-183: screen / complexScreen / complexWorkArea の
//!   更新（`Area.set` L430-442 逐語のデルタ記録）+ カーソル更新
//!   （`Location.set` L159-160 の平均化式・Java int 除算 0 向け切り捨て）
//! - `WindowsEnvironment.tick` L66-85: アクティブ窓（同一 id → 差分 / id 変化 →
//!   `resetDeltas` / rect 無し → `setRect(-1, -1, 0, 0)`・`visible` =
//!   screen と `intersects`）
//! - `AbstractEnvironment.getWorkAreaAt` L186-193: contains（境界含み・Area.java
//!   L477-484）する最初の monitor の work area・該当無しは invisibleScreen（L93-98）
//! - `AbstractEnvironment.updateScreenRect` L104-150: 全モニタ矩形の union
//!   （`Rectangle.union` 式）- WindowsEnvironment.getWorkAreaRect(false) 相当の
//!   work area 直値供給（#10）
//!
//! スロット契約: [`AreaSlot::WorkArea(i)`] / [`AreaSlot::Screen(i)`] の `i` は
//! monitor index 順で 1:1（env.rs モジュール doc 参照）。
//!
//! 意図的差異（Java 一致検証時に差し引くこと・design §1.7(e) 延長）:
//! 1. Java AbstractEnvironment は 5 秒タイマの screenRect 更新スレッドを持つが
//!    （L42-66）、tao 単一スレッド契約（AGENTS §3）のため tick 毎に source から
//!    取得する（常時新鮮・むしろ健全）
//! 2. Java は device id 文字列キーの Map（screenRects / workAreaRects）で slots を
//!    識別するが、本実装は monitor index 順の Vec スロット（design §1.7 の
//!    monitor index 順 1:1 契約）。モニタ数減少時は末尾スロットを除去し、
//!    再増設時は fresh slot（delta 0）で再構築する
//! 3. per-mascot `currentWorkArea` キャッシュは design §1.7(e) により存在しない。
//!    base-4 メソッド（[`EnvironmentView::work_area`] /
//!    [`EnvironmentView::screen`]）は仮想座標 (0,0) を含む monitor（= プライマリ）
//!    に解決する（該当なしは先頭スロット）
//! 4. WindowsEnvironment の isInteractive / DWM cloaked / タイトル whitelist /
//!    blacklist 選別（L87-176）は OS 依存の窓発見処理のため [`OsSource`] 側（#10）に
//!    委譲する。active_window() は「選別済みの単一窓」を受け取る
//! 5. WindowsEnvironment.moveActiveWindow の DPI 補正（L279-283）・
//!    SetWindowPos 呼び出し（L285-288）は [`OsSource::move_window`] へ委譲
//! 6. WindowsEnvironment.restoreWindows / refreshCache / interactiveCache
//!    （L291-346）は #9/#10 の管轄（tray 経路）
//! 7. activeWindowTitle（L84）は #8 の観測経路が無いため保持しない
//!    （Phase 1 未使用・asset-report.md §2）
//! 8. tick の可変状態は `RefCell` 内包で管理し `&self` 更新にする
//!    （tests/app_test.rs 契約・tao 単一スレッド前提のため Mutex は使わない）

use std::cell::RefCell;

use crate::config::script::EvalContext;
use crate::mascot::env::{AreaSlot, AreaState, CursorState};
use crate::mascot::{EnvironmentView, Rect};

/// spawn キューの 1 件（Breed 出生要求・Java Breed.java L73-101 相当）。
/// `behavior_name` は BornBehaviour 属性の評価結果（省略時 ""・#8 で 4 引数化）。
pub struct SpawnRequest {
    pub image_set_name: String,
    pub anchor: (i32, i32),
    pub look_right: bool,
    pub behavior_name: String,
}

/// OS 供給の抽象（実 Win32 供給は #10）。
/// WindowsEnvironment.java の該当部分（EnumWindows / MonitorFromPoint /
/// MouseInfo / SetWindowPos）をこの trait の背後に隠す（テストは fake source で差し替え）。
pub trait OsSource {
    /// 全モニタの (モニタ矩形, WorkArea)（`updateScreenRect` L114-140 相当・
    /// monitor index 順）。Java は 5 秒タイマだが tick 毎に取得する（doc 差異 1）。
    fn monitors(&self) -> Vec<(Rect, Rect)>;

    /// カーソル位置。None = 取得失敗（`MouseInfo.getPointerInfo()` null 相当・
    /// AbstractEnvironment L177-178）。
    fn cursor_position(&self) -> Option<(i32, i32)>;

    /// アクティブ窓（選別済み・WindowsEnvironment L178-194 findActiveWindow 相当）。
    /// (id, 矩形)・None = アクティブ窓無し（`getWindowRect` null 相当・L72）。
    fn active_window(&self) -> Option<(i64, Rect)>;

    /// 窓移動（`moveActiveWindow` L274-289 相当・SetWindowPos SWP_NOSIZE）。
    fn move_window(&self, id: i64, x: i32, y: i32);
}

/// Environment から参照する eval context（`mascot.environment.*` は MascotContext が
/// 自前解決し、`mascot.custom.*` 等のカスタム変数は Environment が保持しないため
/// 常時 None / false を返す）。Phase 2 の agent 用フック。
struct NullEnvCtx;

impl EvalContext for NullEnvCtx {
    fn number(&self, _path: &str) -> Option<f64> {
        None
    }

    fn boolean(&self, _path: &str) -> Option<bool> {
        None
    }

    fn is_on(&self, _target: &str, _x: f64, _y: f64) -> bool {
        false
    }
}

/// 恒常 invisibleScreen（AbstractEnvironment L93-98: 全 0・visible=false）。
fn invisible_state() -> AreaState {
    AreaState {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
        dleft: 0,
        dtop: 0,
        dright: 0,
        dbottom: 0,
        visible: false,
    }
}

/// 長寿命 `Area` スロット 1 つの初期値（fresh slot 作成時は delta 0・
/// visible=true = Java Area 既定。以後の tick は [`AreaState::set`] が delta を記録する）。
fn fresh_area_state(rect: Rect) -> AreaState {
    AreaState {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
        dleft: 0,
        dtop: 0,
        dright: 0,
        dbottom: 0,
        visible: true,
    }
}

/// tick 毎に更新する可変状態（Java `Area` スロット群 + `Location` 相当）。
/// [`Environment`] から `RefCell` 経由で内部更新される（doc 差異 8）。
struct EnvCore {
    /// `complexScreen` 相当（monitor index 順の長寿命スロット）。
    screens: Vec<AreaState>,
    /// `complexWorkArea` 相当（monitor index 順の長寿命スロット）。
    work_areas: Vec<AreaState>,
    /// `screen` 相当（全モニタ union・`new Area(false)`）。
    screen_union: AreaState,
    /// `WindowsEnvironment.activeWindow` 相当（calcDeltas true）。
    active_window: AreaState,
    /// `activeWindowHandle` 相当（最後 tick で観測した id・None = 窓無し）。
    active_window_id: Option<i64>,
    /// `Location` 相当（平均化 delta 込み）。
    cursor: CursorState,
}

/// デスクトップ環境（Java `AbstractEnvironment` + `WindowsEnvironment` 相当）。
/// Manager が所有し、[`EnvironmentView`] 経由で Action / mascot 純関数に参照を渡す。
pub struct Environment {
    source: Box<dyn OsSource>,
    core: RefCell<EnvCore>,
    /// Breed 出生要求（Java は manager.add() 即時。Rust は次 tick 一括 = 意図的差異・
    /// design §1.8(f)）。`&self` から push できるよう RefCell（tao 単一スレッド前提・
    /// Mutex は増やさない）。
    spawns: RefCell<Vec<SpawnRequest>>,
    null_ctx: NullEnvCtx,
    /// Settings.java L32-37 / L45 既定値（settings.properties 無しのため既定適用）。
    breeding: bool,
    transients: bool,
    transformation: bool,
    throwing: bool,
    multiscreen: bool,
    scaling: f64,
}

impl Environment {
    /// [`OsSource`] から Environment を構築する（Java コンストラクタ 相当・
    /// thread 停止と autoUpdateScreenRect は構造差異のため持たない）。
    pub fn new(source: impl OsSource + 'static) -> Environment {
        Environment {
            source: Box::new(source),
            core: RefCell::new(EnvCore {
                screens: Vec::new(),
                work_areas: Vec::new(),
                // `new Area(false)` 相当: Area.java L28 既定 visible=true・delta は常時 0
                //（tick で set → reset_deltas = calcDeltas=false 相当）
                screen_union: fresh_area_state(Rect {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                }),
                // `new Area()`（calcDeltas=true・visible=true 初期）相当。
                // visible は tick 毎に intersects で上書きされる
                active_window: fresh_area_state(Rect {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                }),
                active_window_id: None,
                cursor: CursorState {
                    x: 0,
                    y: 0,
                    dx: 0,
                    dy: 0,
                },
            }),
            spawns: RefCell::new(Vec::new()),
            null_ctx: NullEnvCtx,
            breeding: true,
            transients: true,
            transformation: true,
            throwing: true,
            multiscreen: true,
            scaling: 1.0,
        }
    }

    /// AbstractEnvironment.tick L167-183 + WindowsEnvironment.tick L66-85 逐語:
    /// ① screen / complexScreen / complexWorkArea 更新 ② カーソル更新 ③ アクティブ窓。
    pub fn tick(&self) {
        // Java L104-150（updateScreenRect 相当の直取得・doc 差異 1）
        let monitors = self.source.monitors();
        let mut core = self.core.borrow_mut();

        // ① screen.set(screenRect)（L170）: 全モニタ矩形の union（Rectangle.union 逐語）。
        if let Some((union_left, union_top, union_right, union_bottom)) =
            monitors
                .iter()
                .fold(None, |acc: Option<(i32, i32, i32, i32)>, (monitor, _)| {
                    let (l, t, r, b) = acc.unwrap_or((i32::MAX, i32::MAX, i32::MIN, i32::MIN));
                    Some((
                        l.min(monitor.left),
                        t.min(monitor.top),
                        r.max(monitor.right),
                        b.max(monitor.bottom),
                    ))
                })
        {
            // `new Area(false)` 相当: 座標は更新するが delta は常時 0（reset で殺す）
            core.screen_union
                .set(union_left, union_top, union_right, union_bottom);
            core.screen_union.reset_deltas();
        }

        // L171-172: complexScreen / complexWorkArea slot 更新（doc 差異 2）
        for (index, (monitor, _)) in monitors.iter().enumerate() {
            match core.screens.get_mut(index) {
                Some(area) => area.set(monitor.left, monitor.top, monitor.right, monitor.bottom),
                None => core.screens.push(fresh_area_state(*monitor)),
            }
        }
        core.screens.truncate(monitors.len());

        for (index, (_, work_area)) in monitors.iter().enumerate() {
            match core.work_areas.get_mut(index) {
                Some(state) => state.set(
                    work_area.left,
                    work_area.top,
                    work_area.right,
                    work_area.bottom,
                ),
                None => core.work_areas.push(fresh_area_state(*work_area)),
            }
        }
        core.work_areas.truncate(monitors.len());

        // AbstractEnvironment L177-182 逐語: カーソル（平均化式は Location.set L159-160）。
        match self.source.cursor_position() {
            Some((x, y)) => {
                // Location.set L159-160 逐語: 平均化（Java int 除算 0 向け切り捨て）
                core.cursor.dx = (core.cursor.dx + x - core.cursor.x) / 2;
                core.cursor.dy = (core.cursor.dy + y - core.cursor.y) / 2;
                core.cursor.x = x;
                core.cursor.y = y;
            }
            None => {
                // L180-181 逐語: cursor.set(0, 0)・delta も set(x, y) 相当で更新
                core.cursor.dx = (core.cursor.dx - core.cursor.x) / 2;
                core.cursor.dy = (core.cursor.dy - core.cursor.y) / 2;
                core.cursor.x = 0;
                core.cursor.y = 0;
            }
        }

        // WindowsEnvironment L66-85 逐語: activeWindow 更新。
        let prev_id = core.active_window_id;
        match self.source.active_window() {
            Some((id, rect)) => {
                core.active_window_id = Some(id);
                core.active_window
                    .set(rect.left, rect.top, rect.right, rect.bottom);
            }
            None => {
                core.active_window_id = None;
                // L72-73: setRect(-1, -1, 0, 0)
                core.active_window.set_rect(-1, -1, 0, 0);
            }
        }
        // L77: setVisible(activeWindow.intersects(getScreen()))
        core.active_window.visible = core.active_window.intersects(&core.screen_union);
        if prev_id != core.active_window_id {
            // L79-82: id 変化 → resetDeltas
            core.active_window.reset_deltas();
        }
    }

    /// Breed 出生要求の spawn キューを FIFO 全取り出し + クリアする
    /// （Java は manager.add() 即時だが AGENTS §5-6 追加/削除キューイング踏襲・
    /// 反映は [`Manager::tick`]・意図的差異 design §1.8(f)）。
    pub fn drain_spawns(&mut self) -> Vec<SpawnRequest> {
        std::mem::take(&mut self.spawns.borrow_mut())
    }

    /// settings トグル setter 群（Settings.java L32-37 / L45 相当）。
    pub fn set_breeding_allowed(&mut self, allowed: bool) {
        self.breeding = allowed;
    }

    pub fn set_transients_enabled(&mut self, enabled: bool) {
        self.transients = enabled;
    }

    pub fn set_transformation_allowed(&mut self, allowed: bool) {
        self.transformation = allowed;
    }

    pub fn set_throwing_allowed(&mut self, allowed: bool) {
        self.throwing = allowed;
    }

    pub fn set_multiscreen(&mut self, multiscreen: bool) {
        self.multiscreen = multiscreen;
    }

    pub fn set_scaling(&mut self, scaling: f64) {
        self.scaling = scaling;
    }
}

impl EnvironmentView for Environment {
    // ---- 既存 4 メソッド（#6 確定分・シグネチャ不変）----

    /// base-4 work_area: 仮想座標 (0,0) を含む monitor（= プライマリ）の work area。
    /// 該当なしは先頭スロット。doc 差異 3 の意図的差異（design §1.7(e) 延長）。
    fn work_area(&self) -> Rect {
        let core = self.core.borrow();
        let primary = core.screens.iter().position(|screen| screen.contains(0, 0));
        let rect_of = |state: &AreaState| Rect {
            left: state.left,
            top: state.top,
            right: state.right,
            bottom: state.bottom,
        };
        primary
            .and_then(|index| core.work_areas.get(index))
            .or_else(|| core.work_areas.first())
            .map(rect_of)
            .unwrap_or(Rect {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            })
    }

    /// base-4 screen: 全モニタ union（Java `getScreen()` 相当・
    /// tick 毎に screen_union slot へ set 済みの値 = doc 差異 doc 3）。
    fn screen(&self) -> Rect {
        let core = self.core.borrow();
        Rect {
            left: core.screen_union.left,
            top: core.screen_union.top,
            right: core.screen_union.right,
            bottom: core.screen_union.bottom,
        }
    }

    /// Settings.java L37 既定 true（Settings 供給経路・#9 トレイが setter 使用）。
    fn multiscreen(&self) -> bool {
        self.multiscreen
    }

    /// eval context は platform primitives のみ返す（mod.rs 契約）。
    /// カスタム変数は保持しないため常に None / false。
    fn eval_context(&self) -> &dyn EvalContext {
        &self.null_ctx
    }

    // ---- #7a 追加 10 メソッド ----

    /// `Environment.getScreen()` / AbstractEnvironment.getScreen 相当。
    fn screen_area(&self) -> AreaState {
        self.core.borrow().screen_union
    }

    /// `getScreens()` / AbstractEnvironment L206-208 相当・monitor index 順。
    fn screens(&self) -> Vec<AreaState> {
        self.core.borrow().screens.clone()
    }

    /// getWorkAreaAt L186-193 逐語: contains（境界含み）する最初の monitor・
    /// 該当なしなら invisibleScreen。
    fn work_area_at(&self, x: i32, y: i32) -> AreaSlot {
        let core = self.core.borrow();
        for (index, work_area) in core.work_areas.iter().enumerate() {
            if work_area.contains(x, y) {
                return AreaSlot::WorkArea(index);
            }
        }
        // L192: invisibleScreen
        AreaSlot::Invisible
    }

    /// スロットの現在値（スナップショット・Area への live 参照値相当）。
    /// index は `screens()` と monitor index 順の同一順序を共有する。
    fn work_area_state(&self, slot: AreaSlot) -> AreaState {
        let core = self.core.borrow();
        match slot {
            AreaSlot::WorkArea(index) => core
                .work_areas
                .get(index)
                .cloned()
                .unwrap_or_else(invisible_state),
            AreaSlot::Screen(index) => core
                .screens
                .get(index)
                .cloned()
                .unwrap_or_else(invisible_state),
            AreaSlot::ActiveWindow => core.active_window,
            AreaSlot::Invisible => invisible_state(),
        }
    }

    /// WindowsEnvironment L259-261 getActiveWindow 相当（gating は env.rs）。
    fn active_window(&self) -> AreaState {
        self.core.borrow().active_window
    }

    /// WindowsEnvironment L269-271: getActiveWindowId 逐語（handle null → 0）。
    fn active_window_id(&self) -> i64 {
        self.core.borrow().active_window_id.unwrap_or(0)
    }

    /// WindowsEnvironment L274-289 moveActiveWindow 逐語（窓無しは呼ばない）。
    /// DPI 補正は [`OsSource`] 側（doc 差異 5）。
    fn move_active_window(&self, x: i32, y: i32) {
        if let Some(id) = self.core.borrow().active_window_id {
            self.source.move_window(id, x, y);
        }
    }

    /// `getCursor()` / Location 相当（平均化式・tick で更新済みの値）。
    fn cursor(&self) -> CursorState {
        self.core.borrow().cursor
    }

    /// Settings.java L45 scaling 供給経路（Scaling getter）。
    fn scaling(&self) -> f64 {
        self.scaling
    }

    /// Settings.java L35 throwing 供給経路。
    fn throwing_allowed(&self) -> bool {
        self.throwing
    }

    /// Settings.java L32 breeding 供給経路（design §1.8(f)）。
    fn breeding_allowed(&self) -> bool {
        self.breeding
    }

    /// Settings.java L33 transients 供給経路。
    fn transients_enabled(&self) -> bool {
        self.transients
    }

    /// Settings.java L34 transformation 供給経路。
    fn transformation_allowed(&self) -> bool {
        self.transformation
    }

    /// Breed 出生を spawn キューへ積む（&self から push 可 = RefCell・
    /// Mutex は増やさない。反映は [`Manager::tick`]・意図的差異 design §1.8(f)）。
    fn queue_spawn(
        &self,
        image_set_name: &str,
        anchor: (i32, i32),
        look_right: bool,
        behavior_name: &str,
    ) {
        self.spawns.borrow_mut().push(SpawnRequest {
            image_set_name: image_set_name.to_string(),
            anchor,
            look_right,
            behavior_name: behavior_name.to_string(),
        });
    }
}
