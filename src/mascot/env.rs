//! Environment 式（pure・OS 非依存）— design.md §1.7（タスク #7a）。
//!
//! Java 版の環境依存は「`MascotEnvironment` の判定式 + `Border`（`Area`）の
//! isOn / move + `Area` 幾何」。これらを OS 非依存の純関数として抽出し、
//! [`EnvironmentView`](super::EnvironmentView)（app 側 #8 が実装）が供給する
//! スナップショットから評価する。mascot は app を import しない一方向依存。
//!
//! Java 正本（.tmp/java-ref/environment/・2026-09-06 dea8952 取得分）との対応:
//! - 幾何: `Area` → [`AreaState`]（`set` によるデルタ記録を含む）
//! - 境界: `Wall` / `FloorCeiling` / `NotOnBorder` → [`Edge`] + [`BorderRef`]
//!   トークン方式（design §1.7(b)）。Java の Border は Area への live 参照だが
//!   Rust は可変借用を保持できないため、init 時に [`resolve_border`] で安定トークン
//!   （AreaSlot + Edge）を取得し、tick 毎に [`border_is_on`] / [`border_move`] で
//!   fresh snapshot を評価する。Java の Area は長寿命スロット（ComplexArea が
//!   `set()` で値更新・Border フィールドは生成時固定）なのでトークンは安定する
//! - パス: [`resolve_env_path`]（値 18 パス）+ [`resolve_env_is_on`]（isOn ターゲット
//!   11 パス）= スクリプト 29 パス（asset-report.md §1）。[`MascotContext`](super::MascotContext)
//!   が自前解決に使う（design §1.7(f)）
//!
//! 意図的差異（design §1.7(e)・Java 一致検証時に差し引くこと）:
//! - currentWorkArea キャッシュなし: [`resolve_work_area`] は anchor 引数付きの
//!   毎回 fresh 解決（Java は per-mascot キャッシュ・Dragged 中のみ更新）
//! - multiscreen 境界上アンカーはスロット順（monitor index 順）の決定的優先
//!   （Java は現在モニタ優先）
//!
//! スロット契約: [`AreaSlot::WorkArea(i)`] / [`AreaSlot::Screen(i)`] の `i` は
//! 同一モニタ順序（monitor index 順）を共有する。Java も `AbstractEnvironment` が
//! 同一キー集合（デバイス ID）で complexScreen / complexWorkArea を持つため 1:1。
//! 整数演算: Java int 除算は 0 向け切り捨て = Rust `i32` 除算と同値。

use crate::config::script::to_java_int;

use super::EnvironmentView;

/// `mascot.environment.` プレフィックス（env パス判定用）。
pub const ENV_PREFIX: &str = "mascot.environment.";

/// env パス（`MascotContext` の自前解決対象）か。
/// 非 env パスは従来どおり `EnvironmentView::eval_context` へ委譲される。
pub fn is_env_path(path: &str) -> bool {
    path.starts_with(ENV_PREFIX)
}

// =====================================================================
// 型（design §1.7(a)）
// =====================================================================

/// 領域 1 つのスナップショット（Java `Area` 逐語・calcDeltas=true 既定相当）。
/// [`crate::mascot::Rect`] とは別物（こちらはデルタと visible を持つ long-lived
/// スロットの値）。right / bottom は含まない座標だが `contains` は閉区間。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AreaState {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    /// 前フレームからの left 変化量（Java `Area.dleft`）。
    pub dleft: i32,
    /// 前フレームからの top 変化量（Java `Area.dtop`）。
    pub dtop: i32,
    /// 前フレームからの right 変化量（Java `Area.dright`）。
    pub dright: i32,
    /// 前フレームからの bottom 変化量（Java `Area.dbottom`）。
    pub dbottom: i32,
    /// 可視性（Java `Area.visible`・invisibleScreen は false）。
    pub visible: bool,
}

impl AreaState {
    /// Java `Area.getWidth()` L383-385 逐語。
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    /// Java `Area.getHeight()` L392-394 逐語。
    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// Java `Area.set(int, int, int, int)` L430-442 逐語。
    /// 「新値 - 旧値」をデルタに記録してから座標を更新する。
    pub fn set(&mut self, left: i32, top: i32, right: i32, bottom: i32) {
        self.dleft = left - self.left;
        self.dtop = top - self.top;
        self.dright = right - self.right;
        self.dbottom = bottom - self.bottom;

        self.left = left;
        self.top = top;
        self.right = right;
        self.bottom = bottom;
    }

    /// Java `Area.setRect(int, int, int, int)` L416-418 逐語。
    pub fn set_rect(&mut self, x: i32, y: i32, width: i32, height: i32) {
        self.set(x, y, x + width, y + height);
    }

    /// Java `Area.resetDeltas()` L447-454 逐語。
    pub fn reset_deltas(&mut self) {
        self.dleft = 0;
        self.dtop = 0;
        self.dright = 0;
        self.dbottom = 0;
    }

    /// Java `Area.contains(int, int)` L477-484 逐語。
    /// 4 辺とも境界含み（`java.awt.Rectangle` と異なる）・負の寸法は常に false。
    pub fn contains(&self, x: i32, y: i32) -> bool {
        if ((self.right - self.left) | (self.bottom - self.top)) < 0 {
            // At least one of the dimensions is negative
            return false;
        }
        self.left <= x && x <= self.right && self.top <= y && y <= self.bottom
    }

    /// Java `Area.intersects(Area)` L583-604 逐語（`Rectangle.intersects` 移植式）。
    /// 空でない交差のみ true。
    pub fn intersects(&self, a: &AreaState) -> bool {
        let tw = self.right - self.left;
        let th = self.bottom - self.top;
        let aw = a.right - a.left;
        let ah = a.bottom - a.top;
        if aw <= 0 || ah <= 0 || tw <= 0 || th <= 0 {
            return false;
        }
        let tx = self.left;
        let ty = self.top;
        let ax = a.left;
        let ay = a.top;
        let tw = self.right;
        let th = self.bottom;
        let aw = a.right;
        let ah = a.bottom;
        //      overflow || intersect
        (aw < ax || aw > tx) && (ah < ay || ah > ty) && (tw < tx || tw > ax) && (th < ty || th > ay)
    }
}

/// 境界種別（getFloor / getCeiling / getWall の呼び分け）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderKind {
    Floor,
    Ceiling,
    Wall,
}

/// 領域の 4 辺（Java `Area.getLeftBorder/getTopBorder/getRightBorder/getBottomBorder`
/// 相当の区別）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

/// 領域スロット（Java の長寿命 `Area` オブジェクトの所在）。トークンの参照先。
/// `WorkArea(i)` / `Screen(i)` の `i` は monitor index 順で 1:1 対応。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AreaSlot {
    WorkArea(usize),
    Screen(usize),
    ActiveWindow,
    /// どのモニタにも属さない点用の代替（Java `AbstractEnvironment.invisibleScreen`。
    /// 全 0・visible=false）。
    Invisible,
}

/// 境界トークン（design §1.7(b)）。Java `Border`（Area への live 参照）の代わりに
/// 「どのスロットのどの辺」を安定参照する。`None` = `NotOnBorder.INSTANCE`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorderRef {
    pub area: AreaSlot,
    pub edge: Edge,
}

/// カーソルのスナップショット（Java `Location` 相当）。
/// dx / dy は前フレームからの移動量（平均化は #8 の app 側が行う）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorState {
    pub x: i32,
    pub y: i32,
    pub dx: i32,
    pub dy: i32,
}

/// `mascot.environment.*` スクリプト値パスの解決結果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EnvValue {
    Number(f64),
    Boolean(bool),
}

// =====================================================================
// 辺判定コア（Wall.isOn / FloorCeiling.isOn の共通式）
// =====================================================================

/// 領域 1 つの辺上に点があるか。
/// - Wall（Wall.java L138-141）: `isVisible() && getX() == x && top <= y <= bottom`
/// - FloorCeiling（FloorCeiling.java L147-150）: `isVisible() && getY() == y && left <= x <= right`
///   y / x も境界含み。
fn area_border_is_on(area: &AreaState, edge: Edge, point: (i32, i32)) -> bool {
    let (x, y) = point;
    if !area.visible {
        return false;
    }
    match edge {
        Edge::Left => area.left == x && area.top <= y && y <= area.bottom,
        Edge::Right => area.right == x && area.top <= y && y <= area.bottom,
        Edge::Top => area.top == y && area.left <= x && x <= area.right,
        Edge::Bottom => area.bottom == y && area.left <= x && x <= area.right,
    }
}

// =====================================================================
// work area / activeIE 解決（MascotEnvironment 逐語）
// =====================================================================

/// work area スロットの現在値を取得するヘルパ。
fn work_area_of(env: &dyn EnvironmentView, anchor: (i32, i32)) -> AreaState {
    env.work_area_state(resolve_work_area(env, anchor))
}

/// gating 適用後の activeIE（Java `getActiveIE()` 相当・anchor ベース）。
fn active_ie_of(env: &dyn EnvironmentView, anchor: (i32, i32)) -> AreaState {
    active_ie_effective(env, &work_area_of(env, anchor))
}

/// Java `MascotEnvironment.getActiveIE()` L281-289 逐語（currentWorkArea を
/// ステートレス解決に置き換え）。
///
/// gating 条件: `currentWorkArea != null && !multiscreen && 交差しない` →
/// `new Area()`（全 0・visible=true・Area.java L28 既定）を返す。
/// currentWorkArea は呼び出し側が [`resolve_work_area`] 等で解決して渡す
/// （null 相当は存在しない）。
pub fn active_ie_effective(env: &dyn EnvironmentView, work_area: &AreaState) -> AreaState {
    let active_ie = env.active_window();
    if !env.multiscreen() && !work_area.intersects(&active_ie) {
        // Java `new Area()`: 全座標 0・デルタ 0・visible=true
        return AreaState {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
            dleft: 0,
            dtop: 0,
            dright: 0,
            dbottom: 0,
            visible: true,
        };
    }
    active_ie
}

/// Java `MascotEnvironment.getWorkArea(boolean)` L66-114 のステートレス版
/// （currentWorkArea キャッシュと multiscreen 分岐を除外し、anchor から毎回 fresh 解決）。
///
/// 決定木（Java 逐語）:
/// 1. `impl.getWorkAreaAt(anchor)` を取得し、anchor を含むならそれを返す（L97-100）。
///    invisibleScreen が返った場合も Java 同様 contains 判定する（全 0 領域は
///    anchor == (0,0) で true になる quirk を保持）
/// 2. 見つからなければ screens の中で anchor を含む画面矩形を返す（L104-109。
///    タスクバー上では workArea.* が画面矩形になる quirk の源泉）
/// 3. それも無ければ `impl.getWorkAreaAt` の結果（invisibleScreen）を返す（L112-113）
pub fn resolve_work_area(env: &dyn EnvironmentView, anchor: (i32, i32)) -> AreaSlot {
    let slot = env.work_area_at(anchor.0, anchor.1);
    if env.work_area_state(slot).contains(anchor.0, anchor.1) {
        return slot;
    }
    for (i, screen) in env.screens().iter().enumerate() {
        if screen.contains(anchor.0, anchor.1) {
            return AreaSlot::Screen(i);
        }
    }
    slot
}

// =====================================================================
// 境界解決（getFloor / getCeiling / getWall 逐語）
// =====================================================================

/// Java `MascotEnvironment.getFloor(boolean)` / `getCeiling(boolean)` /
/// `getWall(boolean)`（L212-230 / L169-187 / L253-272）逐語。
///
/// - `point` は Java では常に `mascot.getAnchor()`（BorderedAction / is_screen_* 呼び出し）。
///   スクリプト isOn 経路（[`resolve_env_is_on`]）は isOn に渡された点を解決に使う
///   （解決と isOn 判定の両方に同じ点。資産 21 式は `isOn(mascot.anchor)` 形式のため
///   実運用では点 == anchor で Java と同一結果）
/// - 優先順位: activeIE の辺 → work area の辺（separator 論理）→ `None`
///   （Java `NotOnBorder.INSTANCE`）
/// - work area 辺は `!ignoreSeparator || isScreen*` を満たすときのみ返す
///   （getFloor L224 / getCeiling L181 / getWall L266）。スクリプト isOn 経路は
///   Java 無引数オーバーロード相当で `ignore_separator = false`（L156 / L198 / L242）、
///   Fall は `true` で呼ぶ前提（#7b）
/// - Wall の方向: activeIE は `lookRight ? 左辺 : 右辺`（L258）・work area は
///   `lookRight ? 右辺 : 左辺`（L264）と逆向き
pub fn resolve_border(
    env: &dyn EnvironmentView,
    kind: BorderKind,
    point: (i32, i32),
    look_right: bool,
    ignore_separator: bool,
) -> Option<BorderRef> {
    let work_area_slot = resolve_work_area(env, point);
    let work_area = env.work_area_state(work_area_slot);
    let active_ie = active_ie_effective(env, &work_area);

    // L173 / L216 / L258: activeIE の辺（Floor = 上・Ceiling = 下・Wall = lookRight ? 左 : 右）
    let active_ie_edge = match kind {
        BorderKind::Floor => Edge::Top,
        BorderKind::Ceiling => Edge::Bottom,
        BorderKind::Wall => {
            if look_right {
                Edge::Left
            } else {
                Edge::Right
            }
        }
    };
    if area_border_is_on(&active_ie, active_ie_edge, point) {
        return Some(BorderRef {
            area: AreaSlot::ActiveWindow,
            edge: active_ie_edge,
        });
    }

    // L179-184 / L221-227 / L263-269: work area の辺（Floor = 下・Ceiling = 上・
    // Wall = lookRight ? 右 : 左）
    let work_area_edge = match kind {
        BorderKind::Floor => Edge::Bottom,
        BorderKind::Ceiling => Edge::Top,
        BorderKind::Wall => {
            if look_right {
                Edge::Right
            } else {
                Edge::Left
            }
        }
    };
    if area_border_is_on(&work_area, work_area_edge, point) {
        // separator 論理（L181 / L224 / L266）: スクリーン間の境界は
        // ignoreSeparator か、その辺が単一スクリーンの辺であるときだけ有効
        let on_screen_border = match kind {
            BorderKind::Wall => is_screen_left_right(env, point),
            BorderKind::Floor | BorderKind::Ceiling => is_screen_top_bottom(env, point),
        };
        if !ignore_separator || on_screen_border {
            return Some(BorderRef {
                area: work_area_slot,
                edge: work_area_edge,
            });
        }
    }

    // NotOnBorder.INSTANCE
    None
}

/// トークンで指された辺上に点があるか（design §1.7(b) の fresh 評価）。
/// Java `Border.isOn(Point)` のトークン版:
/// - `None`（NotOnBorder）: 常に false（NotOnBorder.java L25-27）
/// - 辺判定は評価時点のスロット値で行う（init 時トークンは安定・値は fresh）
pub fn border_is_on(
    env: &dyn EnvironmentView,
    border: Option<BorderRef>,
    point: (i32, i32),
) -> bool {
    let Some(r) = border else {
        return false;
    };
    area_border_is_on(&env.work_area_state(r.area), r.edge, point)
}

/// トークンで指された辺の動きに点を追従させる。
/// Java `Border.move(Point)` のトークン版（fresh 評価・戻り値は移動後の点）。
pub fn border_move(
    env: &dyn EnvironmentView,
    border: Option<BorderRef>,
    location: (i32, i32),
) -> (i32, i32) {
    let Some(r) = border else {
        return location; // NotOnBorder.move L30-32
    };
    let area = env.work_area_state(r.area);
    match r.edge {
        Edge::Left => wall_move(&area, false, location), // Wall.java L144-169
        Edge::Right => wall_move(&area, true, location),
        Edge::Top => floor_ceiling_move(&area, false, location), // FloorCeiling.java L153-179
        Edge::Bottom => floor_ceiling_move(&area, true, location),
    }
}

/// Java `Wall.move(Point)` L144-169 逐語。
///
/// デルタ比例再配置: `newY = (y - prevTop) * height / prevHeight + top`。
/// ガード: 前フレーム高さ 0（0 除算回避）・`|Δx| >= 80 || |Δy| >= 80`（跳躍は追随しない）。
fn wall_move(area: &AreaState, right: bool, location: (i32, i32)) -> (i32, i32) {
    let (x, y) = location;
    if !area.visible {
        return location; // L145-147
    }

    let dx = if right { area.dright } else { area.dleft }; // getDX L99-101
                                                           // Return the location as is if the border hasn't moved（L150-152）
    if area.dtop == 0 && area.dbottom == 0 && dx == 0 {
        return location;
    }

    let prev_top = area.top - area.dtop; // L154
    let prev_height = area.bottom - area.dbottom - prev_top; // L155
                                                             // Return the location as is if the height of the border was previously 0,
                                                             // to avoid division by 0（L157-159）
    if prev_height == 0 {
        return location;
    }

    let new_x = x + dx; // L161
    let new_y = (y - prev_top) * area.height() / prev_height + area.top; // L162

    if (new_x - x).abs() >= 80 || (new_y - y).abs() >= 80 {
        return location; // L164-166
    }

    (new_x, new_y)
}

/// Java `FloorCeiling.move(Point)` L153-179 逐語。
///
/// デルタ比例再配置: `newX = (x - prevLeft) * width / prevWidth + left`。
/// ガードは非対称: `|Δx| >= 80 || Δy > 20 || Δy < -80`（下方向は 20px・上方向は
/// 80px まで追随）。
fn floor_ceiling_move(area: &AreaState, bottom: bool, location: (i32, i32)) -> (i32, i32) {
    let (x, y) = location;
    if !area.visible {
        return location; // L155-157
    }

    let dy = if bottom { area.dbottom } else { area.dtop }; // getDY L108-110
                                                            // Return the location as is if the border hasn't moved（L159-161）
    if area.dleft == 0 && area.dright == 0 && dy == 0 {
        return location;
    }

    let prev_left = area.left - area.dleft; // L163
    let prev_width = area.right - area.dright - prev_left; // L164
                                                           // Return the location as is if the width of the border was previously 0,
                                                           // to avoid division by 0（L166-168）
    if prev_width == 0 {
        return location;
    }

    let new_x = (x - prev_left) * area.width() / prev_width + area.left; // L170
    let new_y = y + dy; // L171

    if (new_x - x).abs() >= 80 || new_y - y > 20 || new_y - y < -80 {
        return location; // L173-176
    }

    (new_x, new_y)
}

// =====================================================================
// スクリーン境界判定（isScreenTopBottom / isScreenLeftRight 逐語）
// =====================================================================

/// Java `MascotEnvironment.isScreenTopBottom(Point)` L375-399 逐語。
/// 点が「ちょうど 1 画面の上 / 下辺」上なら true。複数辺上（= スクリーン間
/// separator）なら false。
pub fn is_screen_top_bottom(env: &dyn EnvironmentView, point: (i32, i32)) -> bool {
    count_borders_on(env, point, [Edge::Top, Edge::Bottom]) == 1
}

/// Java `MascotEnvironment.isScreenLeftRight(Point)` L418-442 逐語。
/// 点が「ちょうど 1 画面の左 / 右辺」上なら true。
pub fn is_screen_left_right(env: &dyn EnvironmentView, point: (i32, i32)) -> bool {
    count_borders_on(env, point, [Edge::Left, Edge::Right]) == 1
}

/// isScreen* の共通本体（L376-398 / L419-441 逐語）:
/// screens の該当 2 辺でカウントし、0 なら complexWorkArea へフォールバックして
/// 再カウントする。`count == 1` を返す。
/// complexWorkArea の列挙は work area スロット `0..screens.len()`（monitor index 順
/// 1:1 契約・モジュール doc 参照）で行う。
fn count_borders_on(env: &dyn EnvironmentView, point: (i32, i32), edges: [Edge; 2]) -> usize {
    let mut count = 0usize;
    let screens = env.screens();

    for screen in &screens {
        for edge in edges {
            if area_border_is_on(screen, edge, point) {
                count += 1;
            }
        }
    }

    if count == 0 {
        for i in 0..screens.len() {
            let work_area = env.work_area_state(AreaSlot::WorkArea(i));
            for edge in edges {
                if area_border_is_on(&work_area, edge, point) {
                    count += 1;
                }
            }
        }
    }

    count
}

// =====================================================================
// スクリプトパス解決（MascotEnvironment の式・29 パス）
// =====================================================================

/// `mascot.environment.*` の値パス 18 件を解決する
/// （asset-report.md §1: workArea 6 / cursor 4 / screen.height 1 / activeIE 7）。
///
/// 対応 Java 式（`MascotEnvironment`）:
/// - `workArea.*` = `getWorkArea()`（anchor ベース・L54-114）。タスクバー等で
///   anchor が work area 外のとき screens フォールバックで画面矩形になる quirk を含む
/// - `screen.height` = `getScreen().getHeight()`（全モニタ union・L132-134）
/// - `cursor.*` = `getCursor()`（`Location`・AbstractEnvironment.tick L177-182）
/// - `activeIE.*` = `getActiveIE()`（gating 後・L281-289）
///
/// 未知パスは `None`（評価器が Err に変換する）。
pub fn resolve_env_path(
    env: &dyn EnvironmentView,
    anchor: (i32, i32),
    path: &str,
) -> Option<EnvValue> {
    let rest = path.strip_prefix(ENV_PREFIX)?;
    fn num(v: i32) -> Option<EnvValue> {
        Some(EnvValue::Number(f64::from(v)))
    }
    match rest {
        // getWorkArea()（anchor ベース fresh 解決）
        "workArea.left" => num(work_area_of(env, anchor).left),
        "workArea.top" => num(work_area_of(env, anchor).top),
        "workArea.right" => num(work_area_of(env, anchor).right),
        "workArea.bottom" => num(work_area_of(env, anchor).bottom),
        "workArea.width" => num(work_area_of(env, anchor).width()),
        "workArea.height" => num(work_area_of(env, anchor).height()),
        // getScreen().getHeight()（全モニタ union）
        "screen.height" => num(env.screen_area().height()),
        // getCursor()（Location の 4 値）
        "cursor.x" => num(env.cursor().x),
        "cursor.y" => num(env.cursor().y),
        "cursor.dx" => num(env.cursor().dx),
        "cursor.dy" => num(env.cursor().dy),
        // getActiveIE()（gating 後）
        "activeIE.left" => num(active_ie_of(env, anchor).left),
        "activeIE.top" => num(active_ie_of(env, anchor).top),
        "activeIE.right" => num(active_ie_of(env, anchor).right),
        "activeIE.bottom" => num(active_ie_of(env, anchor).bottom),
        "activeIE.width" => num(active_ie_of(env, anchor).width()),
        "activeIE.height" => num(active_ie_of(env, anchor).height()),
        "activeIE.visible" => Some(EnvValue::Boolean(active_ie_of(env, anchor).visible)),
        _ => None,
    }
}

/// `mascot.environment.*` の isOn ターゲット 11 件を解決する
/// （floor・ceiling・wall 3 + workArea 辺 4 + activeIE 辺 4）。
///
/// - floor / ceiling / wall は Java 無引数オーバーロード相当
///   （`ignoreSeparator = false`・getFloor L198 / getCeiling L156 / getWall L242）で、
///   渡された点で境界を解決してから [`border_is_on`] で判定する
/// - workArea 辺は `getWorkArea()` のスロット辺に直接判定する
///   （Java `getWorkArea().getLeftBorder().isOn(point)` 等と同じ式）
/// - activeIE 辺は gating 後の `getActiveIE()` の辺に直接判定する
/// - f64 → i32 変換は Java 準拠（`to_java_int`・NaN→0 / 切り詰め / 飽和）
/// - 未知ターゲットは `NotOnBorder` 扱いで false
pub fn resolve_env_is_on(
    env: &dyn EnvironmentView,
    look_right: bool,
    target: &str,
    x: f64,
    y: f64,
) -> bool {
    let Some(rest) = target.strip_prefix(ENV_PREFIX) else {
        return false;
    };
    let point = (to_java_int(x), to_java_int(y));
    match rest {
        "floor" => {
            let border = resolve_border(env, BorderKind::Floor, point, look_right, false);
            border_is_on(env, border, point)
        }
        "ceiling" => {
            let border = resolve_border(env, BorderKind::Ceiling, point, look_right, false);
            border_is_on(env, border, point)
        }
        "wall" => {
            let border = resolve_border(env, BorderKind::Wall, point, look_right, false);
            border_is_on(env, border, point)
        }
        "workArea.leftBorder" => area_border_is_on(&work_area_of(env, point), Edge::Left, point),
        "workArea.rightBorder" => area_border_is_on(&work_area_of(env, point), Edge::Right, point),
        "workArea.topBorder" => area_border_is_on(&work_area_of(env, point), Edge::Top, point),
        "workArea.bottomBorder" => {
            area_border_is_on(&work_area_of(env, point), Edge::Bottom, point)
        }
        "activeIE.leftBorder" => area_border_is_on(&active_ie_of(env, point), Edge::Left, point),
        "activeIE.rightBorder" => area_border_is_on(&active_ie_of(env, point), Edge::Right, point),
        "activeIE.topBorder" => area_border_is_on(&active_ie_of(env, point), Edge::Top, point),
        "activeIE.bottomBorder" => {
            area_border_is_on(&active_ie_of(env, point), Edge::Bottom, point)
        }
        _ => false,
    }
}
