//! タスク #11: 手動確認で発見された 2 バグ修正の契約テスト（RED 相当）。
//!
//! 実機症状の原因（orch 確定）に対する修正 A / B を公開契約で pin する。
//! fixture は tests/action_test.rs 流儀（SynthEnv 自己完結・tests/common 不使用）を踏襲し、
//! 実装ファイルは書かない（A1 / B2 が assert 失敗 = RED で正常・Green は coder の仕事）。
//!
//! 修正 A（位置反映）— Java `Mascot.apply`（Mascot.java L661-690）は
//! 「位置は毎 tick 反映（bounds 差分時 setBounds・needsRepaint と無関係）+
//! 画像の再描画のみ needsRepaint 依存」の 2 層構造。Rust 版は `Mascot::set_anchor`
//! （src/mascot/mod.rs L464-466）が needs_repaint を立てないため、画像固定の移動
//! フレーム（Fall: shime4.png 1 枚 Duration 250・Walk: 4 フレーム各 Duration 6）で
//! 窓移動が丸ごとスキップされ、画像切替時にジャンプしていた。
//! - A1: set_anchor 位置変化 → needs_repaint = true（現状 RED）
//! - A2: 同値 set_anchor（needs_repaint == false から）→ false を維持
//!   （Java の bounds 同値スキップの観察等価・立てないでよい側の pin）
//! - A3: 同値 set_anchor（needs_repaint == true から）→ true を維持（クリアしない）
//!
//! 修正 B（hotspot 評価の Java 逐語化）— Java `ActionBase.refreshHotspots`
//! （ActionBase.java L140-155）は `getVariables()`（アクション自身の変数・注入値含む）
//! でアニメ条件を評価し、VariableException 時は catch で clearHotspots のみ（log 無し）。
//! アニメが 1 つも条件一致しない場合（animation == null）は hotspots に触らない。
//! Java `Dragged`（Dragged.java L112-122）は refreshHotspots をオーバーライドし
//! clearHotspots のみ（アニメ条件評価しない・"action does not support hotspots"）。
//! Rust 版は `Base::next_pre`（src/mascot/action/mod.rs L394-426）が throwaway
//! （空の新規 Variables・FootX 未注入）でアニメ条件を評価するため、Pinched の
//! FootX 条件が毎 tick 評価エラー → warn ログ連発していた。
//! - B1: Dragged::next を FootX 未注入のまま実行 → Ok・hotspots 空供給
//!   （smoke pin: 現行実装でも throwaway 評価が catch で握るため GREEN の可能性が高い）
//! - B2: Base 系（Fall）の hotspot 更新経路がアクション自身の変数（注入値）を使う
//!   ことの pin（現行 throwaway では RED・修正後 GREEN）
//!
//! log 出力の assert はしない（warn 抑制は観察困難のため契約外・修正方針の留意点）。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use simeji::config::script::{EvalContext, Variable};
use simeji::config::{Animation, Pose, VarMap};
use simeji::mascot::action::{create, ActionKind};
use simeji::mascot::env::{AreaSlot, AreaState, CursorState};
use simeji::mascot::{EnvironmentView, Hotspot, Mascot, Rect, Rng};
use simeji::render::imageset::{Frame, ImageSet};

// =====================================================================
// 合成モニタ状態の test-double（tests/action_test.rs 流儀の踏襲）
// =====================================================================

/// queue_spawn の呼び出し記録（本ファイルのテストでは未使用だが
/// fixture を action_test.rs と同一形状に保つため保持）。
#[derive(Debug, Clone, PartialEq)]
struct SpawnRec {
    image_set_name: String,
    anchor: (i32, i32),
    look_right: bool,
    behavior_name: String,
}

/// Java Area 相当の AreaState（dbottom=床移動検証用デルタ・visible=true）。
fn area(left: i32, top: i32, right: i32, bottom: i32, dbottom: i32) -> AreaState {
    AreaState {
        left,
        top,
        right,
        bottom,
        dleft: 0,
        dtop: 0,
        dright: 0,
        dbottom,
        visible: true,
    }
}

#[derive(Default)]
struct ProbeCtx;

impl EvalContext for ProbeCtx {
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

/// 単一モニタ構成: work area = (0,0,1920,1040)（床 bottom 1040）/
/// active window = (300,200,900,800)（work area と交差 → activeIE gating 無効）。
struct SynthEnv {
    multiscreen: bool,
    work_area: RefCell<AreaState>,
    cursor: CursorState,
    active_window: AreaState,
    active_window_id: i64,
    scaling_value: f64,
    throwing: bool,
    breeding: bool,
    transients: bool,
    transformation: bool,
    moved_to: RefCell<Vec<(i32, i32)>>,
    spawns: RefCell<Vec<SpawnRec>>,
    ctx: ProbeCtx,
}

impl SynthEnv {
    fn new() -> SynthEnv {
        SynthEnv {
            multiscreen: false,
            work_area: RefCell::new(area(0, 0, 1920, 1040, 0)),
            cursor: CursorState {
                x: 300,
                y: 200,
                dx: 0,
                dy: 0,
            },
            active_window: area(300, 200, 900, 800, 0),
            active_window_id: 7,
            scaling_value: 1.0,
            throwing: true,
            breeding: true,
            transients: true,
            transformation: true,
            moved_to: RefCell::new(Vec::new()),
            spawns: RefCell::new(Vec::new()),
            ctx: ProbeCtx,
        }
    }
}

impl EnvironmentView for SynthEnv {
    fn work_area(&self) -> Rect {
        let a = self.work_area.borrow();
        Rect {
            left: a.left,
            top: a.top,
            right: a.right,
            bottom: a.bottom,
        }
    }

    fn screen(&self) -> Rect {
        Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        }
    }

    fn multiscreen(&self) -> bool {
        self.multiscreen
    }

    fn eval_context(&self) -> &dyn EvalContext {
        &self.ctx
    }

    fn screen_area(&self) -> AreaState {
        area(0, 0, 1920, 1080, 0)
    }

    fn screens(&self) -> Vec<AreaState> {
        vec![area(0, 0, 1920, 1080, 0)]
    }

    fn work_area_at(&self, x: i32, y: i32) -> AreaSlot {
        if (0..=1920).contains(&x) && (0..=1040).contains(&y) {
            AreaSlot::WorkArea(0)
        } else {
            AreaSlot::Invisible
        }
    }

    fn work_area_state(&self, slot: AreaSlot) -> AreaState {
        match slot {
            AreaSlot::WorkArea(0) => *self.work_area.borrow(),
            AreaSlot::Screen(0) => area(0, 0, 1920, 1080, 0),
            _ => AreaState {
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
        self.active_window_id
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

    fn breeding_allowed(&self) -> bool {
        self.breeding
    }

    fn transients_enabled(&self) -> bool {
        self.transients
    }

    fn transformation_allowed(&self) -> bool {
        self.transformation
    }

    fn queue_spawn(
        &self,
        image_set_name: &str,
        anchor: (i32, i32),
        look_right: bool,
        behavior_name: &str,
    ) {
        self.spawns.borrow_mut().push(SpawnRec {
            image_set_name: image_set_name.to_string(),
            anchor,
            look_right,
            behavior_name: behavior_name.to_string(),
        });
    }
}

/// 固定値を返す Rng（本ファイルの契約は rng 消費に依存しない・Dragged の
/// 抵抗抽選は getTime == timeToResist - 1 に届かないため消費されない）。
struct FixedRng;

impl Rng for FixedRng {
    fn unit(&mut self) -> f64 {
        0.5
    }
}

// =====================================================================
// 合成データヘルパ（自己完結）
// =====================================================================

fn empty_image_set() -> Arc<ImageSet> {
    Arc::new(ImageSet {
        name: "TestSet".to_string(),
        frames: BTreeMap::new(),
        warnings: Vec::new(),
        scale: 1.0,
    })
}

fn image_set_with(frames: &[(&str, u32, u32)]) -> Arc<ImageSet> {
    let mut map = BTreeMap::new();
    for (name, width, height) in frames {
        map.insert(
            name.to_string(),
            Frame {
                width: *width,
                height: *height,
                rgba: vec![0u8; (*width as usize) * (*height as usize) * 4],
            },
        );
    }
    Arc::new(ImageSet {
        name: "TestSet".to_string(),
        frames: map,
        warnings: Vec::new(),
        scale: 1.0,
    })
}

fn mascot_at(anchor: (i32, i32)) -> Mascot {
    Mascot::new("TestSet", empty_image_set(), anchor)
}

fn pose(image: &str, anchor: (i32, i32), velocity: (i32, i32), duration: i32) -> Pose {
    Pose {
        image: image.to_string(),
        anchor,
        velocity,
        duration,
    }
}

fn anim(condition: Option<Variable>, is_turn: bool, poses: Vec<Pose>) -> Animation {
    Animation {
        condition,
        poses,
        is_turn,
    }
}

fn var(source: &str) -> Variable {
    Variable::parse(source)
}

fn attrs(pairs: &[(&str, &str)]) -> VarMap {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), Variable::parse(v)))
        .collect()
}

// =====================================================================
// 修正 A 契約: set_anchor × needs_repaint（Java Mascot.apply L661-690）
// =====================================================================

/// A1: `set_anchor` 位置変化 → `needs_repaint() == true`。
/// Java apply は位置（bounds）を needsRepaint と無関係に毎 tick 反映するため、
/// anchor 変化は描画要求を立てる必要がある（画像固定の移動フレームで
/// 窓移動がスキップされる実機バグの修正点・現状は立てないため RED）。
#[test]
fn set_anchor_position_change_requests_repaint() {
    let mut m = mascot_at((1000, 500));
    m.clear_needs_repaint();
    assert!(!m.needs_repaint(), "fixture: needs_repaint をクリア済み");

    m.set_anchor((1001, 500));
    assert_eq!(m.anchor(), (1001, 500), "anchor は新値に更新される");
    assert!(
        m.needs_repaint(),
        "位置変化の set_anchor は needs_repaint を立てる（Java bounds 差分 → setBounds 相当）"
    );
}

/// A2: 同値 `set_anchor`（needs_repaint == false の状態から）→ false を維持。
/// Java は bounds 差分が無ければ setBounds も再描画も行わないため、
/// 同値呼び出しで needs_repaint を立ててはならない（立てないでよい側の pin）。
#[test]
fn set_anchor_same_value_keeps_repaint_false() {
    let mut m = mascot_at((1000, 500));
    m.clear_needs_repaint();
    assert!(!m.needs_repaint(), "fixture: needs_repaint をクリア済み");

    m.set_anchor((1000, 500));
    assert!(
        !m.needs_repaint(),
        "同値 set_anchor は needs_repaint を変えない（Java bounds 同値スキップの観察等価）"
    );
}

/// A3: 同値 `set_anchor`（needs_repaint == true の状態から）→ true を維持。
/// 同値呼び出しは既存の描画要求をクリアしてはならない（画像差し替え等で
/// 既に立った要求を同値 set_anchor が消すと再描画欠落になる）。
#[test]
fn set_anchor_same_value_preserves_repaint_true() {
    let mut m = mascot_at((1000, 500));
    assert!(
        m.needs_repaint(),
        "fixture: 構築直後は needs_repaint = true"
    );

    m.set_anchor((1000, 500));
    assert!(
        m.needs_repaint(),
        "同値 set_anchor は既存の needs_repaint = true を維持する（クリアしない）"
    );
}

// =====================================================================
// 修正 B 契約: hotspot 評価の Java 逐語化（ActionBase L140-155 / Dragged L112-122）
// =====================================================================

/// B1: `DraggedAction::next` を FootX 未注入のまま実行できる。
/// Pinched 風 fixture（アニメ条件に FootX を含む 2 アニメ・SynthEnv + cursor 設定・
/// テスト側からの inject 無し）で next が Ok で戻り、hotspots が空供給される
/// （Java Dragged L112-122: refreshHotspots オーバーライド = clearHotspots のみ・
/// アニメ条件評価しない）。評価エラーが Panic / Err 伝播しないことの smoke pin。
#[test]
fn dragged_next_without_foot_injection_clears_hotspots() {
    let mut env = SynthEnv::new();
    env.cursor.x = 1010;
    env.cursor.y = 100;
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("a.png", 32, 32), ("b.png", 32, 32)]),
        (1000, 500),
    );
    // hotspot 評価の如何にかかわらず「クリア相当」を観察できるよう事前供給する
    m.set_hotspots(vec![Hotspot {
        behaviour: "Stare".to_string(),
    }]);

    let mut rng = FixedRng;
    let mut action = create(
        ActionKind::Dragged,
        &attrs(&[]),
        vec![
            anim(
                Some(var("#{FootX < 500}")),
                false,
                vec![pose("a.png", (32, 16), (0, 0), 5)],
            ),
            anim(
                Some(var("#{FootX >= 500}")),
                false,
                vec![pose("b.png", (32, 16), (0, 0), 5)],
            ),
        ],
        1.0,
    )
    .unwrap();
    action.init(&mut m, &env, &mut rng).unwrap();

    // テスト側は FootX / FootDX を inject しない（アクション自身の注入に任せる）
    let result = action.next(&mut m, &env, &mut rng);
    assert!(
        result.is_ok(),
        "Dragged::next は FootX 未注入でも評価エラーを Panic / Err 伝播してはならない"
    );
    assert!(
        m.is_dragging(),
        "Dragged 本体（set_dragging）まで実行が到達している"
    );
    assert!(
        m.hotspots().is_empty(),
        "Dragged は hotspot 評価を行わず clearHotspots のみ（Java Dragged.java L112-122）"
    );
}

/// B2: Base 系（Fall）の hotspot 更新経路はアクション自身の変数（注入値含む）で
/// アニメ条件を評価する（Java ActionBase.refreshHotspots L140-155 逐語・
/// getVariables() 使用）。 Fall は毎 tick の tick 本体で VelocityX / VelocityY を
/// アクション変数へ inject する（src/mascot/action/mod.rs L689-690・変数は
/// init / reset_values でも消えず tick を跨いで残る）ため:
///
/// - 条件 `#{VelocityX > 999}` は VelocityX 未注入なら評価エラー、
///   注入済み（0.0）なら false（全アニメ不成立 = Java animation == null →
///   hotspots に触らない）になる
/// - tick1: どのみち VelocityX 未注入 → 評価エラー → catch で hotspots クリア
///   （Java L149-152。修正後は warn 無しになるが log は assert しない）
/// - tick2 直前に hotspots を設定し、tick2 で保持されることを確認:
///   アクション変数での評価なら「false → 不成立 → 保持」、現行の throwaway
///   （空 Variables）なら「不明識別子エラー → クリア」で RED になる
#[test]
fn fall_hotspot_refresh_uses_action_variables_with_injections() {
    let env = SynthEnv::new();
    let mut rng = FixedRng;
    let mut m = mascot_at((500, 500));

    let mut action = create(
        ActionKind::Fall,
        &attrs(&[]),
        vec![anim(
            Some(var("#{VelocityX > 999}")),
            false,
            vec![pose("p.png", (0, 0), (0, 0), 5)],
        )],
        1.0,
    )
    .unwrap();
    action.init(&mut m, &env, &mut rng).unwrap();

    // tick1: hotspot 評価はアクション変数（この時点で VelocityX 未注入）でエラー →
    // catch で hotspots クリア（Java ActionBase L149-152）。
    // Fall 本体は単独不成立アニメで apply が Err になるが本契約と無関係のため無視する。
    let _ = action.next(&mut m, &env, &mut rng);
    assert!(
        m.hotspots().is_empty(),
        "アニメ条件の評価エラー時は catch で hotspots がクリアされる"
    );

    // tick1 の Fall 本体で VelocityX / VelocityY がアクション変数へ inject 済み。
    // tick2 の hotspot 評価がこの変数を使うなら条件は false（不成立）で hotspots 保持、
    // throwaway（空 Variables）を使うなら評価エラーでクリアされる。
    m.set_hotspots(vec![Hotspot {
        behaviour: "Pinch".to_string(),
    }]);

    let _ = action.next(&mut m, &env, &mut rng);
    assert_eq!(
        m.hotspots().len(),
        1,
        "hotspot 評価はアクション自身の変数（注入値含む）で行う（Java getVariables() 逐語）: \
         注入済み VelocityX で条件 false → 全アニメ不成立 → hotspots は触られない"
    );
    assert_eq!(
        m.hotspots()[0].behaviour,
        "Pinch",
        "保持された hotspots の内容は無傷"
    );
}
