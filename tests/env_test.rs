//! タスク #7a: src/mascot/env.rs（Environment 式・pure）+ MascotContext 自前解決の契約テスト。
//!
//! Java 正本（.tmp/java-ref/environment/・2026-09-06 dea8952 取得分）を仕様として、
//! design.md §1.7 の純関数群の公開契約（振る舞い）のみを検証する:
//! - AreaState 幾何 = Java Area 逐語: getWidth=right-left（Area.java L383-394）・
//!   contains は境界含み+負次元ガード（L477-484）・intersects は Rectangle 移植式
//!   （L548-604）・set によるデルタ更新+resetDeltas（L416-454）
//! - resolve_border = MascotEnvironment getFloor/getWall/getCeiling 逐語
//!   （L212-230 / L253-272 / L169-187）: activeIE 優先 → workArea → separator 論理
//!   （!ignoreSeparator || isScreen*）→ None（NotOnBorder）。lookRight による Wall の
//!   方向決定（L258/L264）。スクリプト isOn 経路は ignore_separator=false
//!   （Java 無引数オーバーロード L156/L198/L242）、Fall は true で呼ぶ前提（#7b）
//! - border_is_on / border_move = Wall（L138-169）/ FloorCeiling（L147-179）/
//!   NotOnBorder（L25-32）逐語: デルタ比例再配置・|Δx|>=80・FloorCeiling は
//!   Δy>20 / Δy<-80（非対称ガード）・prevHeight/prevWidth==0 の 0 除算回避・
//!   invisible は無変換。トークン（BorderRef）は安定・評価は毎回 fresh（§1.7(b)）
//! - resolve_work_area = getWorkArea のステートレス版（L66-114 からキャッシュ除外）:
//!   work_area_at → work area contains → screens fallback（L104-109）→
//!   work_area_at の結果（invisibleScreen 含む・AbstractEnvironment L186-193）
//! - active_ie_effective = getActiveIE gating 逐語（L281-289）: multiscreen=false かつ
//!   work area と非交差のとき Java `new Area()`（全 0・visible=true、Area.java L28）
//! - is_screen_top_bottom / is_screen_left_right（L375-399 / L418-442）:
//!   screens の境界 isOn カウント → 0 なら complexWorkArea へフォールバック → count==1
//! - resolve_env_path 相当 = スクリプト 29 パス（asset-report.md §1）を MascotContext
//!   （EvalContext 実装）経由で検証: 値パス 18（workArea 6 / cursor 4 / screen.height 1 /
//!   activeIE 7 含む visible ブール）+ isOn ターゲット 11（workArea 4 / activeIE 4 /
//!   floor・wall・ceiling 3）= 29
//! - MascotContext 自前解決（design §1.7(f)）: mascot.environment.* を
//!   snapshot（anchor/look_right/total_count）+ env primitives の合成で解決する。
//!   env.eval_context() が mascot.environment.* を一切知らないモックでも解決成功 =
//!   自前解決の証明。非 env パスの eval_context 委譲は現行どおり維持（差分契約）
//! - EnvironmentView 拡張 10 メソッド（design §1.7(d)）: test-double で全メソッドを
//!   &dyn 経由で呼び出し可能（シグネチャ契約）+ 戻り値形を合成モニタ状態で検証。
//!   既存 4 メソッドのシグネチャ不変は double が trait 実装できること自体で担保
//!   （OS 依存の実挙動は #8 の検証対象）
//!
//! 意図的差異（design §1.7(e) / plan.md (V)-(X)。Java 一致検証時に差し引く）:
//! - (W) マルチモニタ境界上アンカー: Java=current モニタ優先（per-mascot キャッシュ）/
//!   Rust=スロット順（monitor index 順）の決定的優先 → resolve_work_area 系テスト
//! - (X) multiscreen=false の work area: Java=初回解決を維持（Dragged 中のみ更新）/
//!   Rust=常に fresh → resolve_work_area 系テスト
//! - (V) Dragged.tick の refreshWorkArea() no-op 化: 呼び出し点は #7b（Dragged 実装）側
//!   のため本テストの対象外（コメント保持は #7b の Done 条件）
//!
//! 実装 (src/mascot/env.rs) は未存在のため cargo test は compile error = RED が正常。
//! すべて合成モニタ状態で動作し、実資産ファイル・tests/common には依存しない。

use std::cell::RefCell;

use shimeji::config::script::EvalContext;
use shimeji::mascot::env::{
    active_ie_effective, border_is_on, border_move, is_screen_left_right, is_screen_top_bottom,
    resolve_border, resolve_work_area, AreaSlot, AreaState, BorderKind, BorderRef, CursorState,
    Edge,
};
use shimeji::mascot::{EnvironmentView, EvalSnapshot, MascotContext, Rect};

// =====================================================================
// 合成モニタ状態の test-double（design §1.7(g): test-double EnvironmentView）
// =====================================================================

/// Java Area 相当の AreaState を構築する（deltas 0・visible=true。Area.java 既定）。
fn area(left: i32, top: i32, right: i32, bottom: i32) -> AreaState {
    AreaState {
        left,
        top,
        right,
        bottom,
        dleft: 0,
        dtop: 0,
        dright: 0,
        dbottom: 0,
        visible: true,
    }
}

/// env.eval_context() が返す評価コンテキスト。
/// 既定はすべてのパスで None / false を返す（= mascot.environment.* がここに委譲
/// されていたら解決失敗になる検出器。自前解決契約の証明に使う）。
/// probe を設定したパスのみ値を返す（非 env パスの委譲維持の証明に使う）。
#[derive(Default)]
struct ProbeCtx {
    number_probe: Option<(&'static str, f64)>,
    is_on_probe: Option<(&'static str, bool)>,
}

impl EvalContext for ProbeCtx {
    fn number(&self, path: &str) -> Option<f64> {
        match self.number_probe {
            Some((p, v)) if p == path => Some(v),
            _ => None,
        }
    }

    fn boolean(&self, path: &str) -> Option<bool> {
        let _ = path;
        None
    }

    fn is_on(&self, target: &str, _x: f64, _y: f64) -> bool {
        matches!(self.is_on_probe, Some((t, true)) if t == target)
    }
}

/// 合成デスクトップ環境。EnvironmentView の全メソッド（既存 4 + 拡張 10）を実装し、
/// AbstractEnvironment / WindowsEnvironment のプラットフォーム契約を合成状態で模倣する。
///
/// - `work_area_at` = AbstractEnvironment.getWorkAreaAt（L186-193）: 点を含む work area
///   をスロット順に探し、無ければ Invisible
/// - `work_area_state(Invisible)` = invisibleScreen（L93-98）: 全 0・visible=false
/// - screens / work_areas は monitor index 順のスロット（design §1.7(e)-1）
struct SynthEnv {
    multiscreen: bool,
    screens: Vec<AreaState>,
    work_areas: Vec<AreaState>,
    active_window: AreaState,
    cursor: CursorState,
    scaling_value: f64,
    throwing: bool,
    window_id: i64,
    moved_to: RefCell<Vec<(i32, i32)>>,
    ctx: ProbeCtx,
}

fn union_area(areas: &[AreaState]) -> AreaState {
    let mut u = areas[0];
    for a in &areas[1..] {
        u.left = u.left.min(a.left);
        u.top = u.top.min(a.top);
        u.right = u.right.max(a.right);
        u.bottom = u.bottom.max(a.bottom);
    }
    u
}

impl SynthEnv {
    /// 単一モニタ: screen=(0,0,1920,1080) / work area=(0,0,1920,1040)（タスクバー下 40px）/
    /// active window=(300,200,900,800)（work area と交差 → gating 無効）。
    fn single() -> SynthEnv {
        SynthEnv {
            multiscreen: false,
            screens: vec![area(0, 0, 1920, 1080)],
            work_areas: vec![area(0, 0, 1920, 1040)],
            active_window: area(300, 200, 900, 800),
            cursor: CursorState {
                x: 300,
                y: 200,
                dx: 5,
                dy: -2,
            },
            scaling_value: 1.0,
            throwing: true,
            window_id: 0,
            moved_to: RefCell::new(Vec::new()),
            ctx: ProbeCtx::default(),
        }
    }

    /// 横並び 2 モニタ（左右で x=1920 を共有）。
    fn dual_side_by_side() -> SynthEnv {
        let mut env = SynthEnv::single();
        env.screens = vec![area(0, 0, 1920, 1080), area(1920, 0, 3840, 1080)];
        env.work_areas = vec![area(0, 0, 1920, 1040), area(1920, 0, 3840, 1040)];
        env
    }

    /// 縦積み 2 モニタ（y=1080 を共有）。insets 0（Java は work area == screen、
    /// AbstractEnvironment L129-131 が合法）→ モニタ境界がそのまま separator になる。
    fn dual_stacked() -> SynthEnv {
        let mut env = SynthEnv::single();
        env.screens = vec![area(0, 0, 1920, 1080), area(0, 1080, 1920, 2160)];
        env.work_areas = vec![area(0, 0, 1920, 1080), area(0, 1080, 1920, 2160)];
        env
    }

    /// 左右にタスクバーがある単一モニタ（work area が左右に内側）。
    fn side_taskbar() -> SynthEnv {
        let mut env = SynthEnv::single();
        env.work_areas = vec![area(100, 0, 1820, 1080)];
        env
    }
}

impl EnvironmentView for SynthEnv {
    // ---- 既存 4 メソッド（#6・シグネチャ不変契約） ----
    fn work_area(&self) -> Rect {
        let a = &self.work_areas[0];
        Rect {
            left: a.left,
            top: a.top,
            right: a.right,
            bottom: a.bottom,
        }
    }

    fn screen(&self) -> Rect {
        let u = union_area(&self.screens);
        Rect {
            left: u.left,
            top: u.top,
            right: u.right,
            bottom: u.bottom,
        }
    }

    fn multiscreen(&self) -> bool {
        self.multiscreen
    }

    fn eval_context(&self) -> &dyn EvalContext {
        &self.ctx
    }

    // ---- #7a 追加 10 メソッド（design §1.7(d)） ----
    fn screen_area(&self) -> AreaState {
        union_area(&self.screens)
    }

    fn screens(&self) -> Vec<AreaState> {
        self.screens.clone()
    }

    fn work_area_at(&self, x: i32, y: i32) -> AreaSlot {
        // AbstractEnvironment.getWorkAreaAt L186-193 相当:
        // 点を含む work area（スロット順）→ 無ければ invisibleScreen
        match self.work_areas.iter().position(|a| a.contains(x, y)) {
            Some(i) => AreaSlot::WorkArea(i),
            None => AreaSlot::Invisible,
        }
    }

    fn work_area_state(&self, slot: AreaSlot) -> AreaState {
        match slot {
            AreaSlot::WorkArea(i) => self.work_areas[i],
            AreaSlot::Screen(i) => self.screens[i],
            AreaSlot::ActiveWindow => self.active_window,
            AreaSlot::Invisible => AreaState {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
                dleft: 0,
                dtop: 0,
                dright: 0,
                dbottom: 0,
                visible: false,
            },
        }
    }

    fn active_window(&self) -> AreaState {
        self.active_window
    }

    fn active_window_id(&self) -> i64 {
        self.window_id
    }

    fn move_active_window(&self, x: i32, y: i32) {
        self.moved_to.borrow_mut().push((x, y));
    }

    fn cursor(&self) -> CursorState {
        self.cursor
    }

    fn scaling(&self) -> f64 {
        self.scaling_value
    }

    fn throwing_allowed(&self) -> bool {
        self.throwing
    }
}

fn snapshot(anchor: (i32, i32), look_right: bool) -> EvalSnapshot {
    EvalSnapshot {
        anchor,
        look_right,
        total_count: 1,
    }
}

// =====================================================================
// AreaState 幾何（Java Area.java 逐語）
// =====================================================================

#[test]
fn area_state_geometry_matches_java_area() {
    // Area.java L383-394: getWidth=right-left / getHeight=bottom-top
    let a = area(100, 50, 350, 200);
    assert_eq!(a.width(), 250);
    assert_eq!(a.height(), 150);

    // Area.java L477-484: contains は 4 辺とも境界含み（java.awt.Rectangle と異なる）
    assert!(a.contains(100, 50)); // 左上角
    assert!(a.contains(350, 200)); // 右下角
    assert!(a.contains(100, 125)); // 左辺
    assert!(a.contains(350, 125)); // 右辺
    assert!(a.contains(225, 50)); // 上辺
    assert!(a.contains(225, 200)); // 下辺
    assert!(a.contains(225, 125)); // 内部
    assert!(!a.contains(99, 125));
    assert!(!a.contains(351, 125));
    assert!(!a.contains(225, 49));
    assert!(!a.contains(225, 201));

    // Area.java L478-480: 負の寸法は常に false
    let inverted = area(350, 200, 100, 50);
    assert!(!inverted.contains(125, 125));

    // 幅 0（left==right）でも contains は成立しうる（L483 の閉区間判定の帰結）
    let zero_w = area(100, 50, 100, 200);
    assert!(zero_w.contains(100, 125));
}

#[test]
fn area_state_intersects_matches_java_rectangle_port() {
    // Area.java L583-604（Rectangle.intersects 移植式）: 空でない交差のみ true
    let a = area(0, 0, 10, 10);
    assert!(a.intersects(&area(5, 5, 15, 15))); // 部分重なり
    assert!(a.intersects(&area(2, 2, 5, 5))); // 包含
    assert!(!a.intersects(&area(10, 0, 20, 10))); // 接触のみ（空の交差）
    assert!(!a.intersects(&area(20, 20, 30, 30))); // 離れている
    assert!(!a.intersects(&area(5, 5, 5, 5))); // 幅 0（rw<=0）
    assert!(!area(10, 0, 0, 10).intersects(&area(0, 0, 5, 5))); // 自身の幅 0（tw<=0）
}

#[test]
fn area_state_set_updates_deltas_and_reset_deltas() {
    // Area.java L430-442: set は「新値 - 旧値」をデルタに記録してから座標を更新する
    let mut a = area(0, 0, 100, 50);
    a.set(10, -5, 90, 60);
    assert_eq!(a.dleft, 10);
    assert_eq!(a.dtop, -5);
    assert_eq!(a.dright, -10);
    assert_eq!(a.dbottom, 10);
    assert_eq!((a.left, a.top, a.right, a.bottom), (10, -5, 90, 60));

    // Area.java L416-418: setRect(x,y,w,h) == set(x, y, x+w, y+h)
    a.set_rect(0, 0, 100, 50);
    assert_eq!((a.left, a.top, a.right, a.bottom), (0, 0, 100, 50));
    assert_eq!((a.dleft, a.dtop, a.dright, a.dbottom), (-10, 5, 10, -10));

    // 同一座標の set はデルタ 0（L431-435 の差分計算の帰結）
    a.set(0, 0, 100, 50);
    assert_eq!((a.dleft, a.dtop, a.dright, a.dbottom), (0, 0, 0, 0));

    // Area.java L447-454: resetDeltas はデルタのみ 0 にする
    a.set(5, 5, 95, 55);
    a.reset_deltas();
    assert_eq!((a.dleft, a.dtop, a.dright, a.dbottom), (0, 0, 0, 0));
    assert_eq!((a.left, a.top, a.right, a.bottom), (5, 5, 95, 55));
}

// =====================================================================
// resolve_border（MascotEnvironment getFloor/getWall/getCeiling 逐語）
// =====================================================================

#[test]
fn resolve_floor_prefers_active_ie_top_then_work_area_bottom() {
    // MascotEnvironment.getFloor L212-230 逐語
    let env = SynthEnv::single();

    // active IE の上辺（y=200, x∈[300,900]）にアンカー → ActiveWindow スロット
    assert_eq!(
        resolve_border(&env, BorderKind::Floor, (600, 200), false, false),
        Some(BorderRef {
            area: AreaSlot::ActiveWindow,
            edge: Edge::Top
        })
    );

    // IE 上辺外・work area の下辺（y=1040）→ WorkArea スロット
    assert_eq!(
        resolve_border(&env, BorderKind::Floor, (100, 1040), false, false),
        Some(BorderRef {
            area: AreaSlot::WorkArea(0),
            edge: Edge::Bottom
        })
    );

    // どちらでもない → None（Java NotOnBorder.INSTANCE 相当）
    assert_eq!(
        resolve_border(&env, BorderKind::Floor, (960, 500), false, false),
        None
    );
}

#[test]
fn resolve_ceiling_prefers_active_ie_bottom_then_work_area_top() {
    // MascotEnvironment.getCeiling L169-187 逐語
    let env = SynthEnv::single();

    assert_eq!(
        resolve_border(&env, BorderKind::Ceiling, (600, 800), false, false),
        Some(BorderRef {
            area: AreaSlot::ActiveWindow,
            edge: Edge::Bottom
        })
    );
    assert_eq!(
        resolve_border(&env, BorderKind::Ceiling, (960, 0), false, false),
        Some(BorderRef {
            area: AreaSlot::WorkArea(0),
            edge: Edge::Top
        })
    );
    assert_eq!(
        resolve_border(&env, BorderKind::Ceiling, (960, 500), false, false),
        None
    );
}

#[test]
fn resolve_wall_direction_depends_on_look_right() {
    // MascotEnvironment.getWall L253-272 逐語:
    // activeIE は lookRight ? 左辺 : 右辺（L258・work area と逆向きの意図的構造）
    // work area は lookRight ? 右辺 : 左辺（L264）
    let env = SynthEnv::single();

    // look_right=false → IE 右辺（x=900）
    assert_eq!(
        resolve_border(&env, BorderKind::Wall, (900, 500), false, false),
        Some(BorderRef {
            area: AreaSlot::ActiveWindow,
            edge: Edge::Right
        })
    );
    // look_right=true → IE 左辺（x=300）（L258 の反転を_pin_）
    assert_eq!(
        resolve_border(&env, BorderKind::Wall, (300, 500), true, false),
        Some(BorderRef {
            area: AreaSlot::ActiveWindow,
            edge: Edge::Left
        })
    );
    // look_right=false で IE 左辺上の点は選ばれない → work area 左辺（x=0）も不一致 → None
    assert_eq!(
        resolve_border(&env, BorderKind::Wall, (300, 500), false, false),
        None
    );
    // look_right=true で IE 右辺上の点は選ばれない → None
    assert_eq!(
        resolve_border(&env, BorderKind::Wall, (900, 500), true, false),
        None
    );

    // work area: look_right=false → 左辺（x=0）/ look_right=true → 右辺（x=1920）
    assert_eq!(
        resolve_border(&env, BorderKind::Wall, (0, 500), false, false),
        Some(BorderRef {
            area: AreaSlot::WorkArea(0),
            edge: Edge::Left
        })
    );
    assert_eq!(
        resolve_border(&env, BorderKind::Wall, (1920, 500), true, false),
        Some(BorderRef {
            area: AreaSlot::WorkArea(0),
            edge: Edge::Right
        })
    );
    // look_right=true で work area 左辺上の点は右辺判定のみ → None（L264 の_pin_）
    assert_eq!(
        resolve_border(&env, BorderKind::Wall, (0, 500), true, false),
        None
    );
}

#[test]
fn resolve_border_prefers_active_ie_when_both_borders_match() {
    // 優先順位の_pin_: work area 下辺（1040）と activeIE 上辺（1040）が一致する点では
    // ActiveWindow を返す（getFloor L215-219 が L221-227 より先）。
    // multiscreen=false だと touching は非交差で gating されるため multiscreen=true で確認。
    let mut env = SynthEnv::dual_side_by_side();
    env.multiscreen = true;
    env.active_window = area(300, 1040, 900, 1600); // 上辺 == work area 0 の下辺

    assert_eq!(
        resolve_border(&env, BorderKind::Floor, (600, 1040), false, false),
        Some(BorderRef {
            area: AreaSlot::ActiveWindow,
            edge: Edge::Top
        })
    );
}

#[test]
fn resolve_border_separator_gate_matches_java() {
    // getFloor L224 / getCeiling L181 / getWall L266 逐語:
    //   if (workAreaBorder.isOn(anchor)) { if (!ignoreSeparator || isScreenTopBottom()) { return ... } }
    //   → !ignoreSeparator が true（= ignoreSeparator=false）のときは無条件で返す
    //   → ignoreSeparator=true のとき isScreen*() == false（= スクリーン間 separator）
    //     なら返さない（None）。ignoreSeparator=true は「separator 辺を無視する」意味。
    // [修正 #7a 差戻し] 初版は極性を逆に pin していた（false→None / true→Some）。

    // (1) 縦積み 2 モニタ: y=1080 は screen0 の下辺 かつ screen1 の上辺
    //    （isScreenTopBottom のカウントが 2 → false = separator）。work area 0 の下辺でもある。
    let env = SynthEnv::dual_stacked();
    // ignoreSeparator=false（スクリプト isOn 経路・無引数オーバーロード L198）は
    // separator でもそのまま返す（L224 の !ignoreSeparator が true ため）
    assert_eq!(
        resolve_border(&env, BorderKind::Floor, (960, 1080), false, false),
        Some(BorderRef {
            area: AreaSlot::WorkArea(0),
            edge: Edge::Bottom
        })
    );
    // ignoreSeparator=true（Fall の前提）は separator を返さない（L224 の isScreenTopBottom
    // が false のため）→ NotOnBorder 相当
    assert_eq!(
        resolve_border(&env, BorderKind::Floor, (960, 1080), false, true),
        None
    );

    // (1') 同じ縦積み環境の separator 以外の辺: y=2160 は work area 1 の下辺かつ
    //    screen1 の下辺のみ（isScreenTopBottom のカウントが 1 → true = 非 separator）。
    //    Fall（ignoreSeparator=true）でも非 separator の辺には乗れる（L224 が true）。
    assert_eq!(
        resolve_border(&env, BorderKind::Floor, (960, 2160), false, true),
        Some(BorderRef {
            area: AreaSlot::WorkArea(1),
            edge: Edge::Bottom
        })
    );

    // (2) 横並び 2 モニタ: x=1920 は screen0 の右辺 かつ screen1 の左辺
    //    （isScreenLeftRight カウント 2 → false = separator）。work area 0 の右辺でもある。
    //    work_area_at はスロット順で WorkArea(0) を返す（(W) 差異は resolve_work_area 側で_pin_）。
    let env = SynthEnv::dual_side_by_side();
    // ignoreSeparator=false → separator でも返す（L266 の !ignoreSeparator が true）
    assert_eq!(
        resolve_border(&env, BorderKind::Wall, (1920, 500), true, false),
        Some(BorderRef {
            area: AreaSlot::WorkArea(0),
            edge: Edge::Right
        })
    );
    // ignoreSeparator=true → separator を返さない（L266 の isScreenLeftRight が false）
    assert_eq!(
        resolve_border(&env, BorderKind::Wall, (1920, 500), true, true),
        None
    );
}

// =====================================================================
// active_ie_effective（getActiveIE L281-289 gating 逐語）
// =====================================================================

#[test]
fn active_ie_effective_gates_disjoint_window_unless_multiscreen() {
    let wa = area(0, 0, 1920, 1040);

    // 交差する場合 → active window をそのまま返す（L284 の条件が成立しない）
    let env = SynthEnv::single();
    assert_eq!(active_ie_effective(&env, &wa), area(300, 200, 900, 800));

    // 非交差 + multiscreen=false → 空領域。Java は `new Area()`
    // （L285）= 全座標 0・デルタ 0・visible=true（Area.java L28 既定）を_pin_
    let mut env = SynthEnv::single();
    env.active_window = area(5000, 200, 5600, 800);
    assert_eq!(active_ie_effective(&env, &wa), area(0, 0, 0, 0));

    // 非交差でも multiscreen=true なら gating しない（L284 は !multiscreen を要求）
    let mut env = SynthEnv::single();
    env.multiscreen = true;
    env.active_window = area(5000, 200, 5600, 800);
    assert_eq!(active_ie_effective(&env, &wa), area(5000, 200, 5600, 800));
}

// =====================================================================
// resolve_work_area（getWorkArea ステートレス版・L66-114 からキャッシュ除外）
// =====================================================================

#[test]
fn resolve_work_area_is_fresh_and_matches_java_decision_tree() {
    let env = SynthEnv::single();

    // work area が anchor を含む → WorkArea(0)（L97-100）
    assert_eq!(resolve_work_area(&env, (960, 500)), AreaSlot::WorkArea(0));

    // タスクバー領域（screen には含まれるが work area 外）→ screens フォールバックで
    // Screen(0) を返す（L104-109。Java は getWorkArea が画面矩形を返しうる点を_pin_）
    assert_eq!(resolve_work_area(&env, (960, 1050)), AreaSlot::Screen(0));

    // どこにも含まれない → invisibleScreen（L112-113 + AbstractEnvironment L186-193）
    assert_eq!(resolve_work_area(&env, (5000, 500)), AreaSlot::Invisible);

    // [意図的差異 (X): multiscreen=false でも常に fresh] — Java は currentWorkArea を
    // キャッシュし（L87）モニタを跨いでも初回解決を維持するが、Rust 版は stateless のため
    // 同一環境・同一呼び出しで anchor に応じた結果が変わる（むしろ健全・design §1.7(e)-2）。
    let env = SynthEnv::dual_side_by_side(); // multiscreen=false のまま
    assert_eq!(resolve_work_area(&env, (960, 500)), AreaSlot::WorkArea(0));
    assert_eq!(resolve_work_area(&env, (2500, 500)), AreaSlot::WorkArea(1));
}

#[test]
fn resolve_work_area_monitor_boundary_uses_slot_order() {
    // [意図的差異 (W): マルチモニタ境界上アンカー] — x=1920 は work area 0 の右辺かつ
    // work area 1 の左辺（contains 境界含みで両方が含む）。Java は per-mascot キャッシュに
    // より「現在のモニタ」を優先する（MascotEnvironment L83-85）が、Rust 版はスロット順
    // （monitor index 順）の決定的優先で WorkArea(0) を返す（design §1.7(e)-1）。
    let env = SynthEnv::dual_side_by_side();
    assert_eq!(resolve_work_area(&env, (1920, 500)), AreaSlot::WorkArea(0));
}

// =====================================================================
// is_screen_top_bottom / is_screen_left_right（L375-399 / L418-442）
// =====================================================================

#[test]
fn is_screen_top_bottom_counts_borders_with_work_area_fallback() {
    let env = SynthEnv::single();

    // screen の下辺（y=1080）上 → カウント 1（L387-398 の count==1）
    assert!(is_screen_top_bottom(&env, (960, 1080)));
    // screen 辺上ではないが work area 下辺（y=1040）上 → count 0 から complexWorkArea
    // フォールバックでカウント 1（L388-396）
    assert!(is_screen_top_bottom(&env, (960, 1040)));
    // どの辺でもない → false
    assert!(!is_screen_top_bottom(&env, (960, 500)));

    // 縦積み: y=1080 は screen0 下辺 かつ screen1 上辺 → カウント 2 → false
    // （複数辺上 = separator、L358-360 の doc 契約）
    let stacked = SynthEnv::dual_stacked();
    assert!(!is_screen_top_bottom(&stacked, (960, 1080)));
    assert!(is_screen_top_bottom(&stacked, (960, 2160))); // screen1 下辺のみ → 1
}

#[test]
fn is_screen_left_right_counts_borders_with_work_area_fallback() {
    let env = SynthEnv::single();

    // screen の左辺（x=0）上 → カウント 1
    assert!(is_screen_left_right(&env, (0, 500)));
    // 単一モニタでは screen 右辺（x=1920）上もカウント 1
    assert!(is_screen_left_right(&env, (1920, 500)));
    // screen 辺上でも work area 辺上でもない → false
    assert!(!is_screen_left_right(&env, (500, 500)));

    // work area が左右に内側（x=100 / x=1820）→ screen 辺上ではないが
    // work area 左辺上の点はフォールバックでカウント 1（L431-439）
    let side = SynthEnv::side_taskbar();
    assert!(is_screen_left_right(&side, (100, 500)));

    // 横並び: x=1920 は screen0 右辺 かつ screen1 左辺 → カウント 2 → false
    let dual = SynthEnv::dual_side_by_side();
    assert!(!is_screen_left_right(&dual, (1920, 500)));
}

// =====================================================================
// border_is_on（Wall L138-141 / FloorCeiling L147-150 / NotOnBorder L25-27）
// + トークンは安定・評価は fresh（design §1.7(b)）
// =====================================================================

#[test]
fn border_is_on_matches_java_boundary_semantics_and_fresh_state() {
    let env = SynthEnv::single();
    let wall = BorderRef {
        area: AreaSlot::WorkArea(0),
        edge: Edge::Left,
    };
    let floor = BorderRef {
        area: AreaSlot::WorkArea(0),
        edge: Edge::Bottom,
    };

    // Wall L139-141: getX()==x && top <= y <= bottom（y も境界含み）&& visible
    assert!(border_is_on(&env, Some(wall), (0, 500)));
    assert!(!border_is_on(&env, Some(wall), (1, 500)));
    assert!(border_is_on(&env, Some(wall), (0, 1040))); // y = bottom 境界含み
    assert!(!border_is_on(&env, Some(wall), (0, -1)));

    // FloorCeiling L148-149: getY()==y && left <= x <= right（x も境界含み）
    assert!(border_is_on(&env, Some(floor), (960, 1040)));
    assert!(!border_is_on(&env, Some(floor), (960, 1039)));
    assert!(border_is_on(&env, Some(floor), (1920, 1040))); // x = right 境界含み
    assert!(!border_is_on(&env, Some(floor), (1921, 1040)));

    // NotOnBorder L25-27: None トークンは常に false
    assert!(!border_is_on(&env, None, (0, 500)));

    // [トークンは安定・評価は fresh]（§1.7(b)）: init 時に取得したトークンは値なので不変、
    // isOn は評価時点の領域状態（Java の Area への live 参照相当）を読む。
    let mut moved = SynthEnv::single();
    moved.work_areas[0].left = 50;
    assert!(border_is_on(&moved, Some(wall), (50, 500)));
    assert!(!border_is_on(&moved, Some(wall), (0, 500)));

    // invisible になった領域の辺は常に false（L139 / L148 の visible 条件）
    let mut hidden = SynthEnv::single();
    hidden.work_areas[0].visible = false;
    assert!(!border_is_on(&hidden, Some(wall), (0, 500)));
    assert!(!border_is_on(&hidden, Some(floor), (960, 1040)));
}

// =====================================================================
// border_move（Wall.java L144-169 / FloorCeiling.java L153-179 逐語）
// =====================================================================

#[test]
fn border_move_wall_matches_java_delta_relocation() {
    // (a) デルタ全 0 → 無変換（Wall L150-152）
    let env = SynthEnv::single();
    let wall = BorderRef {
        area: AreaSlot::WorkArea(0),
        edge: Edge::Left,
    };
    assert_eq!(border_move(&env, Some(wall), (0, 500)), (0, 500));

    // (b) 水平並進: dleft=10 → newX = x + dX、高さ不変なので newY は比例式でも y のまま
    let mut env = SynthEnv::single();
    env.work_areas[0].left = 10;
    env.work_areas[0].dleft = 10;
    assert_eq!(border_move(&env, Some(wall), (0, 500)), (10, 500));

    // (c) 比例再配置: 高さ 100 → 200（dbottom=100）。旧上辺から 50/100 の位置にいた点は
    //     新上辺から 100/200 の位置へ（Wall L154-162 の比例式・Java 整数除算）
    let mut env = SynthEnv::single();
    env.work_areas[0] = AreaState {
        dtop: 0,
        dbottom: 100,
        ..area(0, 100, 1920, 300)
    };
    assert_eq!(border_move(&env, Some(wall), (0, 150)), (0, 200));

    // (d) |Δx| >= 80 ガード（Wall L164-166）: 100px の跳躍は無変換
    let mut env = SynthEnv::single();
    env.work_areas[0].left = 100;
    env.work_areas[0].dleft = 100;
    assert_eq!(border_move(&env, Some(wall), (0, 500)), (0, 500));

    // (e) |Δy| >= 80 ガード: 高さ不変のまま 85px 上へ（newY = y - 85 → Δy=-85）
    let mut env = SynthEnv::single();
    env.work_areas[0] = AreaState {
        dtop: -85,
        dbottom: -85,
        ..area(0, -85, 1920, 955)
    };
    assert_eq!(border_move(&env, Some(wall), (0, 500)), (0, 500));

    // (f) prevHeight == 0 → 0 除算回避で無変換（Wall L157-159）。高さ 0 のまま dleft のみ動く
    let mut env = SynthEnv::single();
    env.work_areas[0] = AreaState {
        dleft: 10,
        ..area(10, 500, 1920, 500)
    };
    assert_eq!(border_move(&env, Some(wall), (0, 500)), (0, 500));

    // (g) invisible → 無変換（Wall L145-147）
    let mut env = SynthEnv::single();
    env.work_areas[0].left = 10;
    env.work_areas[0].dleft = 10;
    env.work_areas[0].visible = false;
    assert_eq!(border_move(&env, Some(wall), (0, 500)), (0, 500));

    // (h) 右辺の Wall は dright を使う（Wall L100: right ? dright : dleft）
    let mut env = SynthEnv::single();
    env.work_areas[0].right = 1950;
    env.work_areas[0].dright = 30;
    let right_wall = BorderRef {
        area: AreaSlot::WorkArea(0),
        edge: Edge::Right,
    };
    assert_eq!(
        border_move(&env, Some(right_wall), (1920, 500)),
        (1950, 500)
    );

    // None（NotOnBorder L30-32）は無変換
    assert_eq!(border_move(&env, None, (12, 34)), (12, 34));
}

#[test]
fn border_move_floorceiling_matches_java_asymmetric_guards() {
    let floor = BorderRef {
        area: AreaSlot::WorkArea(0),
        edge: Edge::Bottom,
    };

    // (a) デルタ全 0 → 無変換（FloorCeiling L159-161）
    let env = SynthEnv::single();
    assert_eq!(border_move(&env, Some(floor), (500, 1040)), (500, 1040));

    // (b) 水平並進: dleft=dright=30 → newX = x + 30、newY = y + dY（=0）（L163-171）
    let mut env = SynthEnv::single();
    env.work_areas[0].left = 30;
    env.work_areas[0].right = 1950;
    env.work_areas[0].dleft = 30;
    env.work_areas[0].dright = 30;
    assert_eq!(border_move(&env, Some(floor), (500, 1040)), (530, 1040));

    // (c) Δy == 20 は許可（L173 のガードは「> 20」厳密）
    let mut env = SynthEnv::single();
    env.work_areas[0].bottom = 1060;
    env.work_areas[0].dbottom = 20;
    assert_eq!(border_move(&env, Some(floor), (500, 1040)), (500, 1060));

    // (d) Δy == 21 は不許可（下方向への追随は 20px まで）
    let mut env = SynthEnv::single();
    env.work_areas[0].bottom = 1061;
    env.work_areas[0].dbottom = 21;
    assert_eq!(border_move(&env, Some(floor), (500, 1040)), (500, 1040));

    // (e) Δy == -80 は許可（L173 のガードは「< -80」厳密）
    let mut env = SynthEnv::single();
    env.work_areas[0].bottom = 960;
    env.work_areas[0].dbottom = -80;
    assert_eq!(border_move(&env, Some(floor), (500, 1040)), (500, 960));

    // (f) Δy == -81 は不許可（上方向への追随は 80px まで）→ (c)-(f) で非対称ガードを_pin_
    let mut env = SynthEnv::single();
    env.work_areas[0].bottom = 959;
    env.work_areas[0].dbottom = -81;
    assert_eq!(border_move(&env, Some(floor), (500, 1040)), (500, 1040));

    // (g) |Δx| == 80 は不許可（L173 の「>= 80」。Δy と異なり境界値を含む）
    let mut env = SynthEnv::single();
    env.work_areas[0].left = 80;
    env.work_areas[0].right = 2000;
    env.work_areas[0].dleft = 80;
    env.work_areas[0].dright = 80;
    assert_eq!(border_move(&env, Some(floor), (500, 1040)), (500, 1040));

    // (h) 比例再配置: 幅 1920 → 1840（dright=-80）。x=960（幅の半分）は x=920 へ
    let mut env = SynthEnv::single();
    env.work_areas[0].right = 1840;
    env.work_areas[0].dright = -80;
    assert_eq!(border_move(&env, Some(floor), (960, 1040)), (920, 1040));

    // (i) prevWidth == 0 → 0 除算回避で無変換（FloorCeiling L166-168）
    let mut env = SynthEnv::single();
    env.work_areas[0] = AreaState {
        dbottom: 20,
        ..area(30, 0, 30, 1040)
    };
    assert_eq!(border_move(&env, Some(floor), (30, 1040)), (30, 1040));

    // (j) invisible → 無変換（FloorCeiling L155-157）
    let mut env = SynthEnv::single();
    env.work_areas[0].bottom = 1060;
    env.work_areas[0].dbottom = 20;
    env.work_areas[0].visible = false;
    assert_eq!(border_move(&env, Some(floor), (500, 1040)), (500, 1040));

    // (k) 上辺（Ceiling トークン）は dtop を使う（FloorCeiling L109: bottom ? dbottom : dtop）
    let mut env = SynthEnv::single();
    env.work_areas[0].top = -21;
    env.work_areas[0].dtop = -21;
    let ceiling = BorderRef {
        area: AreaSlot::WorkArea(0),
        edge: Edge::Top,
    };
    assert_eq!(border_move(&env, Some(ceiling), (500, 0)), (500, -21));
}

// =====================================================================
// MascotContext 自前解決 = resolve_env_path 相当（スクリプト 29 パス）
// asset-report.md §1: 値パス 18（workArea 6 / cursor 4 / screen.height 1 / activeIE 7）
// + isOn ターゲット 11（workArea 辺 4 / activeIE 辺 4 / floor・wall・ceiling 3）
// =====================================================================

#[test]
fn mascot_context_resolves_all_script_env_value_paths() {
    // work area (0,0,1920,1040) / IE (300,200,900,800) / cursor (300,200,5,-2)
    let env = SynthEnv::single();
    let snap = snapshot((960, 500), false); // anchor は work area 内
    let ctx = MascotContext {
        snapshot: &snap,
        env: &env,
    };

    // 値パス 18 件（件数+キー集合+一括 assert・期待値は合成モニタ状態から焼き込み）。
    // env.eval_context()（ProbeCtx）はすべて None を返すため、解決成功自体が
    // 自前解決（snapshot + env primitives）の証明になる（design §1.7(f)）。
    let numbers: &[(&str, f64)] = &[
        // workArea（snapshot.anchor から fresh 解決された work area の値・getWorkArea L66-114）
        ("mascot.environment.workArea.left", 0.0),
        ("mascot.environment.workArea.top", 0.0),
        ("mascot.environment.workArea.right", 1920.0),
        ("mascot.environment.workArea.bottom", 1040.0),
        ("mascot.environment.workArea.width", 1920.0),
        ("mascot.environment.workArea.height", 1040.0),
        // screen（union の高さ・MascotEnvironment.getScreen L132-134）
        ("mascot.environment.screen.height", 1080.0),
        // cursor（AbstractEnvironment.tick L177-182 の Location 値）
        ("mascot.environment.cursor.x", 300.0),
        ("mascot.environment.cursor.y", 200.0),
        ("mascot.environment.cursor.dx", 5.0),
        ("mascot.environment.cursor.dy", -2.0),
        // activeIE（gating 後の active window・getActiveIE L281-289）
        ("mascot.environment.activeIE.left", 300.0),
        ("mascot.environment.activeIE.top", 200.0),
        ("mascot.environment.activeIE.right", 900.0),
        ("mascot.environment.activeIE.bottom", 800.0),
        ("mascot.environment.activeIE.width", 600.0),
        // [修正 #7a 差戻し] height = bottom - top（Area.java L392-394）。初期 pin の 800.0 は
        // bottom 値の誤複写。activeIE=(300,200,900,800) なので 600 が正
        // （workArea/screen は top=0 のため bottom と一致するが、activeIE のみ判別できる）
        ("mascot.environment.activeIE.height", 600.0),
    ];
    assert_eq!(numbers.len(), 17, "数値パスは 17 件（+ブール 1 = 18）");
    for (path, expected) in numbers {
        assert_eq!(ctx.number(path), Some(*expected), "path: {path}");
    }
    // ブール 1 件（activeIE.visible・資産使用 7 件目）
    assert_eq!(
        ctx.boolean("mascot.environment.activeIE.visible"),
        Some(true)
    );

    // [Java の quirk を_pin_] anchor がタスクバー領域の場合、getWorkArea は screens
    // フォールバック（L104-109）で画面矩形を返す → workArea.* は画面矩形の値になる
    // （fresh 解決なので anchor 変更が直ちに反映される）。
    let snap = snapshot((960, 1050), false);
    let ctx = MascotContext {
        snapshot: &snap,
        env: &env,
    };
    assert_eq!(ctx.number("mascot.environment.workArea.left"), Some(0.0));
    assert_eq!(
        ctx.number("mascot.environment.workArea.bottom"),
        Some(1080.0)
    );
    assert_eq!(
        ctx.number("mascot.environment.workArea.height"),
        Some(1080.0)
    );
}

#[test]
fn mascot_context_resolves_all_script_is_on_targets() {
    // work area (0,0,1920,1040) / IE (300,200,900,800)
    let env = SynthEnv::single();

    // スクリプト isOn（Java 無引数オーバーロード = ignoreSeparator=false・L156/L198/L242）。
    // wall は snapshot.look_right で方向が決まる（getWall L258/L264）。
    // (target, look_right, x, y, expected)
    let rows: &[(&str, bool, f64, f64, bool)] = &[
        ("mascot.environment.floor", false, 960.0, 1040.0, true),
        ("mascot.environment.ceiling", false, 960.0, 0.0, true),
        ("mascot.environment.wall", false, 0.0, 500.0, true),
        ("mascot.environment.wall", true, 1920.0, 500.0, true),
        (
            "mascot.environment.workArea.leftBorder",
            false,
            0.0,
            500.0,
            true,
        ),
        (
            "mascot.environment.workArea.rightBorder",
            false,
            1920.0,
            500.0,
            true,
        ),
        (
            "mascot.environment.workArea.topBorder",
            false,
            960.0,
            0.0,
            true,
        ),
        (
            "mascot.environment.workArea.bottomBorder",
            false,
            960.0,
            1040.0,
            true,
        ),
        (
            "mascot.environment.activeIE.leftBorder",
            false,
            300.0,
            500.0,
            true,
        ),
        (
            "mascot.environment.activeIE.rightBorder",
            false,
            900.0,
            500.0,
            true,
        ),
        (
            "mascot.environment.activeIE.topBorder",
            false,
            600.0,
            200.0,
            true,
        ),
        (
            "mascot.environment.activeIE.bottomBorder",
            false,
            600.0,
            800.0,
            true,
        ),
        // 負例（ interior / 辺の外側 / look_right による方向選択の確認）
        ("mascot.environment.floor", false, 960.0, 500.0, false),
        ("mascot.environment.floor", false, 960.0, 1050.0, false),
        ("mascot.environment.wall", true, 0.0, 500.0, false),
        (
            "mascot.environment.activeIE.leftBorder",
            false,
            301.0,
            500.0,
            false,
        ),
        (
            "mascot.environment.workArea.bottomBorder",
            false,
            960.0,
            1041.0,
            false,
        ),
    ];
    // 件数+キー集合: 正例で 11 ターゲット（workArea 辺 4 / activeIE 辺 4 / floor・wall・ceiling 3）
    // を過不足なくカバーすること
    let positive_targets: std::collections::BTreeSet<&str> =
        rows.iter().filter(|r| r.4).map(|r| r.0).collect();
    assert_eq!(
        positive_targets.len(),
        11,
        "正例は 11 ターゲット全種であること"
    );

    for (target, look_right, x, y, expected) in rows {
        let snap = snapshot((960, 500), *look_right);
        let ctx = MascotContext {
            snapshot: &snap,
            env: &env,
        };
        assert_eq!(
            ctx.is_on(target, *x, *y),
            *expected,
            "target: {target} / look_right: {look_right} / point: ({x}, {y})"
        );
    }
}

#[test]
fn mascot_context_env_paths_self_resolve_and_other_paths_still_delegate() {
    // 差分契約（design §1.7(f)・現行 mod.rs の eval_context 委譲との差分）:
    // mascot.environment.* は自前解決（probe を使わない）・非 env パスは
    // 従来どおり env.eval_context() へ委譲される。
    let mut env = SynthEnv::single();
    env.ctx.number_probe = Some(("mascot.custom.probe", 777.0));
    env.ctx.is_on_probe = Some(("mascot.custom.border", true));
    let snap = snapshot((960, 500), false);
    let ctx = MascotContext {
        snapshot: &snap,
        env: &env,
    };

    // 非 env パス → 委譲（ProbeCtx が応答）
    assert_eq!(ctx.number("mascot.custom.probe"), Some(777.0));
    assert!(ctx.is_on("mascot.custom.border", 1.0, 1.0));

    // env パス → 自前解決（geometry から。ProbeCtx は None/false を返すため
    // 委譲に頼っていたら Some(0.0) / true にはならない）
    assert_eq!(ctx.number("mascot.environment.workArea.left"), Some(0.0));
    assert!(ctx.is_on("mascot.environment.floor", 960.0, 1040.0));
    assert_eq!(
        ctx.boolean("mascot.environment.activeIE.visible"),
        Some(true)
    );
}

#[test]
fn mascot_context_unknown_env_path_returns_none() {
    let env = SynthEnv::single();
    let snap = snapshot((960, 500), false);
    let ctx = MascotContext {
        snapshot: &snap,
        env: &env,
    };

    // 未知パスは None（評価器が Err に変換する。script_eval_test.rs の既存契約と整合）
    assert_eq!(ctx.number("mascot.environment.unknown.path"), None);
    assert_eq!(ctx.boolean("mascot.environment.unknown"), None);
}

// =====================================================================
// EnvironmentView 拡張 10 メソッド（design §1.7(d)）のシグネチャ+戻り値形
// =====================================================================

#[test]
fn environment_view_extended_methods_are_callable_with_contract_shapes() {
    // すべて &dyn EnvironmentView 経由で呼ぶ（オブジェクト安全性+シグネチャ契約）。
    let env = SynthEnv::single();
    let view: &dyn EnvironmentView = &env;

    // 既存 4 メソッド（シグネチャ不変）の代表確認
    assert_eq!(
        view.work_area(),
        Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040
        }
    );
    assert_eq!(
        view.screen(),
        Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080
        }
    );
    assert!(!view.multiscreen());
    assert!(view
        .eval_context()
        .number("mascot.environment.workArea.left")
        .is_none());

    // 追加メソッドの戻り値形
    assert_eq!(view.screen_area(), area(0, 0, 1920, 1080));
    assert_eq!(view.work_area_at(960, 500), AreaSlot::WorkArea(0));
    assert_eq!(view.work_area_at(960, 1050), AreaSlot::Invisible); // タスクバー領域
    assert_eq!(
        view.work_area_state(AreaSlot::WorkArea(0)),
        area(0, 0, 1920, 1040)
    );
    assert_eq!(
        view.work_area_state(AreaSlot::Screen(0)),
        area(0, 0, 1920, 1080)
    );
    // invisibleScreen（AbstractEnvironment L93-98）: 全 0・visible=false
    let invisible = view.work_area_state(AreaSlot::Invisible);
    assert!(!invisible.visible);
    assert_eq!(invisible.width(), 0);
    assert_eq!(view.active_window(), area(300, 200, 900, 800));
    assert_eq!(view.active_window_id(), 0);
    let cursor = view.cursor();
    assert_eq!(
        (cursor.x, cursor.y, cursor.dx, cursor.dy),
        (300, 200, 5, -2)
    );
    assert_eq!(view.scaling(), 1.0);
    assert!(view.throwing_allowed());
    view.move_active_window(100, 200);
    assert_eq!(*env.moved_to.borrow(), vec![(100, 200)]);

    // 2 モニタ構成: screens 列挙 / work_area_at のスロット解決 / union screen
    let env = SynthEnv::dual_side_by_side();
    let view: &dyn EnvironmentView = &env;
    let screens = view.screens();
    assert_eq!(screens.len(), 2);
    assert_eq!(screens[0], area(0, 0, 1920, 1080));
    assert_eq!(screens[1], area(1920, 0, 3840, 1080));
    assert_eq!(view.screen_area(), area(0, 0, 3840, 1080));
    assert_eq!(view.work_area_at(2500, 500), AreaSlot::WorkArea(1));
    assert_eq!(
        view.work_area_state(AreaSlot::Screen(1)),
        area(1920, 0, 3840, 1080)
    );

    // active window ありの id
    let mut env = SynthEnv::single();
    env.window_id = 12345;
    assert_eq!(env.active_window_id(), 12345);
}
