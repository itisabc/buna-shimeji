//! タスク #7b: Action 実装（design.md §1.8・Java 正本 .tmp/java-ref/action/）の契約テスト（RED）。
//!
//! Java 正本を仕様として、公開契約の振る舞いのみを検証する（tests/env_test.rs 書式踏襲・
//! 自己完結・tests/common 不使用）。期待値には Java 行番号をコメント焼き込み。
//!
//! pin する構築 API（2 経路）:
//! - `shimeji::mascot::action::create(kind: ActionKind, attrs: &VarMap,
//!   animations: Vec<Animation>, scale: f64) -> Result<Box<dyn Action>, BehaviorError>`
//!   （Java ActionBuilder.buildAction switch L386-437 相当・直接構築用）
//! - `shimeji::mascot::action::build_action(actions: &ActionsConfig, name: &str,
//!   extra: &VarMap, scale: f64) -> Result<Box<dyn Action>, BehaviorError>`
//!   （Java Configuration.buildAction(name, params) 相当・config 駆動・Ref/Inline
//!   マージ込み）
//! - trait Action: init / has_next / next / is_draggable の全メソッドが
//!   `rng: &mut dyn Rng` を引数に受ける（design §1.8(e)）
//! - EnvironmentView 追加 4 メソッド（§1.8(f)）: `breeding_allowed() -> bool` /
//!   `transients_enabled() -> bool` / `transformation_allowed() -> bool` /
//!   `queue_spawn(image_set_name: &str, anchor: (i32, i32), look_right: bool,
//!   behavior_name: &str)`（**#8 で 4 引数化**: Breed が実 XML 属性 BornBehavior の
//!   名を第 4 引数で queue へ渡す契約・design §1.8(f) 補完）
//! - config 変更: `Animation { condition, poses, is_turn }` /
//!   `ActionDef::Sequence/Select { …, is_loop: bool }`（design §1.8(g)・Rust 予約語
//!   `loop` のため `is_loop`）/ `ActionDef::* { border: Option<BorderType> }`
//!   （同 §1.8(g) 追記・属性省略 = None = border 無効・Java BorderedAction.java L24）
//! - Mascot 追加 setter: `set_affordances(Vec<String>)`（§1.8(b) の検証に必要）
//!
//! 実装 (src/mascot/action/) 未存在のため cargo test は compile error = RED が正常。
//! すべて合成データで動作し、実資産ファイル・tests/common には依存しない。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use shimeji::config::script::{EvalContext, Variable};
use shimeji::config::{
    parse_actions, ActionDef, ActionsConfig, Animation, BehaviorDef, BehaviorEntry, BorderType,
    Pose, SequenceChild, VarMap,
};
use shimeji::mascot::action::{build_action, create, ActionKind};
use shimeji::mascot::behavior::{
    Action, BehaviorError, BehaviorFactory, BehaviorRunner, BehaviorTable,
};
use shimeji::mascot::env::{AreaSlot, AreaState, CursorState};
use shimeji::mascot::{EnvironmentView, ImageState, Mascot, Rect, Rng};
use shimeji::render::imageset::{Frame, ImageSet};

// =====================================================================
// 合成モニタ状態の test-double
// =====================================================================

/// queue_spawn の呼び出し記録（#8: 第 4 引数 = 実 XML 属性 BornBehavior 名を pin）。
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
    // ---- 既存 4 メソッド（#6・シグネチャ不変契約） ----
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

    // ---- #7a 拡張 10 メソッド（default todo!() 宣言のオーバーライド） ----
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

    // ---- #7b 追加 4 メソッド（design §1.8(f)。シグネチャ pin） ----
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

// =====================================================================
// 合成データヘルパ（自己完結）
// =====================================================================

/// Java Math.random 相当の [0,1) 乱数。キューを順に返し、過剰消費で panic する。
/// `consumed()` = Math.random() 呼び出し回数（短絡 pin 用）。
struct FakeRng {
    values: Vec<f64>,
    consumed: usize,
}

impl FakeRng {
    fn repeated(value: f64, count: usize) -> FakeRng {
        FakeRng {
            values: vec![value; count],
            consumed: 0,
        }
    }

    fn consumed(&self) -> usize {
        self.consumed
    }
}

impl Rng for FakeRng {
    fn unit(&mut self) -> f64 {
        let idx = self.consumed;
        let v = *self
            .values
            .get(idx)
            .unwrap_or_else(|| panic!("FakeRng 枯渇（{} 回要求）", idx + 1));
        self.consumed += 1;
        v
    }
}

/// 閉包ファクトリ（遷移先候補の構築に使う）。
struct FnFactory {
    make: Rc<dyn Fn() -> Result<Box<dyn Action>, BehaviorError>>,
}

impl FnFactory {
    fn constant(make: impl Fn() -> Result<Box<dyn Action>, BehaviorError> + 'static) -> FnFactory {
        FnFactory {
            make: Rc::new(make),
        }
    }
}

impl BehaviorFactory for FnFactory {
    fn build_action(&mut self, _child: &SequenceChild) -> Result<Box<dyn Action>, BehaviorError> {
        (self.make)()
    }
}

#[allow(dead_code)]
fn empty_image_set() -> Arc<ImageSet> {
    empty_image_set_with_scale(1.0)
}

/// 解決済み scale を保持する空 ImageSet（`ImageSet.scale` 契約・既定 1.0）。
fn empty_image_set_with_scale(scale: f64) -> Arc<ImageSet> {
    Arc::new(ImageSet {
        name: "TestSet".to_string(),
        frames: BTreeMap::new(),
        warnings: Vec::new(),
        scale,
    })
}

fn image_set_with(frames: &[(&str, u32, u32)]) -> Arc<ImageSet> {
    image_set_with_scale(frames, 1.0)
}

fn image_set_with_scale(frames: &[(&str, u32, u32)], scale: f64) -> Arc<ImageSet> {
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
        scale,
    })
}

fn mascot_at(anchor: (i32, i32)) -> Mascot {
    mascot_at_scale(anchor, 1.0)
}

/// set scale（`ImageSet.scale`）付きのマスコット（空画像セット）。
fn mascot_at_scale(anchor: (i32, i32), scale: f64) -> Mascot {
    Mascot::new("TestSet", empty_image_set_with_scale(scale), anchor)
}

/// set scale（`ImageSet.scale`）付きのマスコット（フレームあり）。
fn mascot_with_scale(anchor: (i32, i32), frames: &[(&str, u32, u32)], scale: f64) -> Mascot {
    Mascot::new("TestSet", image_set_with_scale(frames, scale), anchor)
}

/// 画面内 bounds を保証する合成フレーム状態（128x128・center (64,64)）。
fn on_screen_image() -> ImageState {
    ImageState {
        image_ref: "shime1.png".to_string(),
        center: (64, 64),
        width: 128,
        height: 128,
    }
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

fn actions_config(entries: Vec<(&str, ActionDef)>) -> ActionsConfig {
    let mut map = BTreeMap::new();
    for (name, def) in entries {
        map.insert(name.to_string(), def);
    }
    ActionsConfig { actions: map }
}

fn single_table(name: &str, frequency: i32) -> BehaviorTable {
    BehaviorTable::new(&shimeji::config::BehaviorsConfig {
        entries: vec![BehaviorEntry::Single(BehaviorDef {
            name: name.to_string(),
            frequency,
            hidden: false,
            toggleable: false,
            action: SequenceChild::Ref {
                name: name.to_string(),
                attrs: VarMap::new(),
            },
            next: None,
        })],
    })
}

/// 遷移先の最小 fallback（固定 Animate・velocity 0）。
fn make_idle_fallback() -> Result<Box<dyn Action>, BehaviorError> {
    create(
        ActionKind::Animate,
        &attrs(&[]),
        vec![anim(None, false, vec![pose("idle.png", (0, 0), (0, 0), 5)])],
        1.0,
    )
}

/// Mascot に BehaviorRunner 形で action を装着する（Java Mascot.setBehavior 相当）。
/// 完了遷移先は velocity 0 の Animate（追加副作用なし）に固定する。
fn set_action(
    m: &mut Mascot,
    env: &dyn EnvironmentView,
    name: &str,
    action: Result<Box<dyn Action>, BehaviorError>,
    rng: &mut dyn Rng,
) -> Result<(), BehaviorError> {
    let table = single_table(name, 1);
    m.set_behavior(
        Some(BehaviorRunner::new(name, action?)),
        env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        rng,
    )
}

// =====================================================================
// 共通 ActionBase 契約（ActionBase.java L26-253）
// =====================================================================

/// 属性既定値（Duration=i32::MAX・L27 / Condition=true・L30 / Draggable=true・L33）+
/// 相対時刻 time = mascot.time − start_time（L211-217・init setTime(0)）+
/// アニメ duration 3 での完了。
#[test]
fn animate_defaults_complete_at_animation_duration() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((1000, 500));
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Animate,
        &attrs(&[]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (1, 0), 3)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (1001, 500),
        "tick1: アニメ適用（anchor += (1,0)）"
    );
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (1002, 500), "tick2");
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (1003, 500), "tick3");

    // tick4: hasNext = time(3) < アニメ duration(3) → false → 完了遷移（idle へ）。
    // 遷移後 init（start_time 更新）は適用を伴わない → anchor は不変。
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (1003, 500), "tick4: 完了 → idle へ遷移");
    assert_eq!(m.time(), 4);
}

/// Draggable 属性の既定値は true（ActionBase L33・Java L231-233）。
#[test]
fn animate_draggable_defaults_true_via_trait_call() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 8);
    let mut m = mascot_at((1000, 500));
    let mut action = create(
        ActionKind::Animate,
        &attrs(&[]),
        vec![anim(None, false, vec![pose("p.png", (0, 0), (0, 0), 3)])],
        1.0,
    )
    .unwrap();
    action.init(&mut m, &env, &mut rng).unwrap();
    assert!(
        action.is_draggable(&mut m, &env, &mut rng).unwrap(),
        "Draggable 既定 true"
    );
}

/// Condition 属性: boolean 定数 false → has_next false（即完了）。
/// `${}` スクリプト条件（例: ドル記法 "1 == 1"）が評価されて true となり継続する。
#[test]
fn attr_condition_gate_has_next() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((1000, 500));
    let table = single_table("X", 1);

    // Condition=false（定数）→ has_next false → init 後に遷移（アニメ適用なし）
    let action = create(
        ActionKind::Animate,
        &attrs(&[("Condition", "false")]),
        vec![anim(None, false, vec![pose("n.png", (0, 0), (5, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (1000, 500), "Condition=false → アニメ適用なし");
    assert_eq!(m.time(), 1, "即完了 → 遷移済み");

    // Condition スクリプト式 "1 == 1"（ドル記法）→ has_next true → アニメ実行
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m2 = mascot_at((1000, 500));
    let action = create(
        ActionKind::Animate,
        &attrs(&[("Condition", "${1 == 1}")]),
        vec![anim(None, false, vec![pose("q.png", (0, 0), (1, 0), 3)])],
        1.0,
    );
    set_action(&mut m2, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);
    m2.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m2.anchor(), (1001, 500), "スクリプト条件成立 → アニメ実行");
}

/// Affordance 属性（文字列定数は VarMap 直接取り出し・design §1.8(d)）:
/// Affordance="cushion" → next() 毎に affordances = ["cushion"]（Java L108-113）。
/// Affordance 無し（既定 ""）→ クリアのみ。
#[test]
fn affordances_cleared_then_set_per_frame() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((1000, 500));
    m.set_affordances(vec!["hog".to_string()]);
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Animate,
        &attrs(&[("Affordance", "cushion")]),
        vec![anim(None, false, vec![pose("p.png", (0, 0), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.affordances(),
        ["cushion".to_string()],
        "Affordance 属性で affordances が置き換わる"
    );
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.affordances(), ["cushion".to_string()]);

    // Affordance 属性なしでは affordances はクリアのみ
    let mut m2 = mascot_at((1000, 500));
    m2.set_affordances(vec!["hog".to_string()]);
    let action = create(
        ActionKind::Animate,
        &attrs(&[]),
        vec![anim(None, false, vec![pose("p.png", (0, 0), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m2, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);
    m2.tick(&env, &table, &mut factory, &mut rng);
    assert!(
        m2.affordances().is_empty(),
        "Affordance 属性が無いと affordances はクリアされる"
    );
}

/// refreshHotspots（ActionBase L140-155）: アニメ条件の評価エラー時は
/// hotspot をクリア（L149-152 catch 相当）し、tick 内の Eval エラーを伝播させる。
#[test]
fn refresh_hotspots_cleared_on_animation_condition_error() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((1000, 500));
    m.set_hotspots(vec![shimeji::mascot::Hotspot {
        behaviour: "Stare".to_string(),
    }]);
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Animate,
        &attrs(&[]),
        vec![anim(
            Some(var("#{mascot.unknown.path == 1}")),
            false,
            vec![pose("p.png", (0, 0), (0, 0), 5)],
        )],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);
    m.tick(&env, &table, &mut factory, &mut rng);
    assert!(
        m.hotspots().is_empty(),
        "アニメ条件評価の失敗で hotspots がクリアされる"
    );
    assert!(m.remove_pending(), "Eval エラーは dispose 経路に伝播する");
}

/// stub 17 種: has_next=false で即完了 + 警告ログ（design §1.8(a)）。
#[test]
fn stub_kinds_complete_immediately() {
    let stub_kinds: [ActionKind; 17] = [
        ActionKind::ScanMove,
        ActionKind::ScanJump,
        ActionKind::ScanInteract,
        ActionKind::BroadcastStay,
        ActionKind::BroadcastMove,
        ActionKind::BroadcastJump,
        ActionKind::Broadcast,
        ActionKind::ComplexMove,
        ActionKind::ComplexJump,
        ActionKind::BreedMove,
        ActionKind::BreedJump,
        ActionKind::Interact,
        ActionKind::SelfDestruct,
        ActionKind::Mute,
        ActionKind::MoveWithTurn,
        ActionKind::Turn,
        ActionKind::Transform,
    ];
    for kind in stub_kinds {
        let mut action = create(kind, &attrs(&[]), vec![], 1.0).unwrap();
        let mut m = mascot_at((1000, 500));
        let env = SynthEnv::new();
        let mut rng = FakeRng::repeated(0.5, 8);
        action.init(&mut m, &env, &mut rng).unwrap();
        assert!(
            !action.has_next(&mut m, &env, &mut rng).unwrap(),
            "stub は 1 tick も続けてはいけない"
        );
    }
}

/// 未知 Embedded FQN は fail-fast（Java ActionBuilder L194-206 踏襲）。
#[test]
fn unknown_embedded_fqn_fails_fast() {
    let cfg = actions_config(vec![(
        "Bogus",
        ActionDef::Embedded {
            class: "com.group_finity.mascot.action.Sketchy".to_string(),
            border: Some(BorderType::Floor),
            attrs: VarMap::new(),
            animations: vec![],
        },
    )]);
    let extra = VarMap::new();
    assert!(
        build_action(&cfg, "Bogus", &extra, 1.0).is_err(),
        "未知 FQN は BehaviorError で fail-fast"
    );
}

// =====================================================================
// BorderedAction 契約（BorderedAction.java L36-69）
// =====================================================================

/// init で resolve_border（Floor）→ tick 毎に fresh な border_move:
/// work area が Area.set で移動（bottom 1040→1042・dbottom +2）した後の tick で
/// anchor が新底辺に追随する（FloorCeiling.move L153-179 逐語）。
#[test]
fn bordered_action_tracks_fresh_workspace_move() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((100, 1040));
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Animate,
        &attrs(&[("BorderType", "Floor")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();

    // Area.set 相当（FloorCeiling.java L153-179 のデルタ再配置の入力）
    env.work_area.borrow_mut().set(0, 0, 1920, 1042);

    m.tick(
        &env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut rng,
    );
    assert_eq!(
        m.anchor(),
        (100, 1042),
        "床移動 +2 を fresh border_move で anchor に追随させる"
    );
}

/// 境界外放出: Animate に Floor 境界を持たせ、anchor が床に無い → tick で
/// LostGround（Java L60-62 / Animate.java L34-36 逐語）→ Fall 遷移へ
/// （テーブルは Fall 行を持たないためエラー伝播 = dispose 相当で pin）。
#[test]
fn animate_off_border_throws_lost_ground() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((500, 500)); // 画面内だが床上ではない
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Animate,
        &attrs(&[("BorderType", "Floor")]),
        vec![anim(None, false, vec![pose("p.png", (0, 0), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();

    m.tick(
        &env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut rng,
    );
    assert_eq!(
        m.time(),
        1,
        "LostGround catch も time++ を進める（Java 同一）"
    );
    assert!(
        m.remove_pending(),
        "Fallback 未定義のテーブルでは LostGround 遷移が Err になり dispose 相当"
    );
}

/// 「未指定 = border 無し」pin（Java BorderedAction.java L24 DEFAULT_BORDERTYPE=null /
/// L40-48 未指定・未知値は setBorder しない / L52-57 border==null なら border.move を
/// スキップ）: BorderType 属性を省略して構築した bordered 系アクションは、床外に
/// いても border_move も LostGround も起きず anchor 不変（明示 Floor との対比は
/// animate_off_border_throws_lost_ground）。
#[test]
fn bordered_action_without_border_type_skips_border_move() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((500, 500)); // 床（bottom 1040）外・明示 Floor なら LostGround の位置
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Animate,
        &attrs(&[]), // BorderType 省略 → border = None
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (500, 500),
        "border 無し → border.move スキップ（L52-57）で anchor 不変"
    );
    assert!(
        !m.remove_pending(),
        "border 検査自体が無いため LostGround にならない"
    );
    assert_eq!(m.time(), 1, "has_next はアニメ duration 基準で継続");
}

// =====================================================================
// Fall 契約（Fall.java L99-153）
// =====================================================================

/// 重力式の焼き込み（既定 ResistanceX=0.05 / ResistanceY=0.1 / Gravity=2・scale 1）:
///   tick1: velocityY = 2 → dy = 2 → (500,502)
///   tick2: velocityY = 3.8, modY = 0.8 → dy = round(4.6) = 5 → (500,507)
#[test]
fn fall_gravity_formula_pinned_two_ticks() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((500, 500));
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Fall,
        &attrs(&[]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (500, 502),
        "tick1: velocityY=2・dy=2（L108/L118）"
    );
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (500, 507),
        "tick2: velocityY=3.8・dy=round(4.6)（L107-121）"
    );
}

/// stepwise 衝突ループ（j in -80..=0・Fall.java L129-149）で床に置かれる +
/// 着地後 isOn で has_next false → Fall 完了（L87-97・getFloor(true) L139）。
/// Gravity=50（Java 手計算・modX/modY 残差込み・Fall.java L107-121 逐語）:
///   tick1: vY=50        ・modY=0      → dy=50  → 550
///   tick2: vY=95        ・modY=0      → dy=95  → 645
///   tick3: vY=135.5     ・modY=0.5    → dy=136 → 781
///   tick4: vY=171.95    ・modY=1.45→0.45 → dy=173 → 954
///   tick5: vY=204.755   ・modY=1.205  → dy=206 → i=86（954+86=1040＝floor.isOn）
///        で j=0 がヒットして 1040 に着地。
#[test]
fn fall_lands_on_floor_exactly_and_completes() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((500, 500));
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Fall,
        &attrs(&[("Gravity", "50")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (500, 550), "tick1");
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (500, 645), "tick2: vY=95・dy=round(95+0)=95");
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (500, 781),
        "tick3: vY=135.5・modY=0.5・dy=round(136)=136"
    );
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (500, 954),
        "tick4: vY=171.95・modY=0.5+0.95・dy=round(173.4)=173"
    );
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (500, 1040),
        "tick5: stepwise ループで床と同位に置かれる（Fall.java L129-149）"
    );
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.time(),
        6,
        "着地 → floor.isOn(anchor) で has_next false → 完了遷移"
    );
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (500, 1040), "遷移後も床に置かれたまま");
}

/// 初速は init のマスコット set scale で拡大（Fall.java L82-83:
/// velocityX = InitialVX * scaling）。scale は env.scaling ではなく
/// `Mascot::scale()`（= ImageSet.scale）から取る（env.scaling=7.0 でも不変）。
/// set scale 2.0・InitialVX=-2 → velocityX -4・gravity も scale 済み
/// （2*2=4）→ Java 手計算（Fall.java L107-121 逐語・modX/modY 残差込み）:
///   velocityX = -4 - (-4)*0.05 = -3.8
///   modX = 0 + (-3.8 % 1) = -0.8（Java `%` は符号保持）
///   dx = round(-3.8 + -0.8) = round(-4.6) = -5（Java Math.round = 半上げ）
///   velocityY = 4・modY = 4%1 = 0 → dy = 4
///   → (500 + round(-4.6), 500 + 4) = (495, 504)
#[test]
fn fall_initial_velocity_and_gravity_scale_with_mascot_set_scale() {
    let mut env = SynthEnv::new();
    env.scaling_value = 7.0; // 非依存 pin（set scale 2.0 が採用される）
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at_scale((500, 500), 2.0);
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Fall,
        &attrs(&[("InitialVX", "-2")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (495, 504),
        "init scale: velocityX=-2*2=-4 → 減衰 -3.8・mod -0.8 → dx=round(-4.6)=-5・gravity 2*2=4 → dy=4"
    );
}

/// Fall の抵抗値は実資産 Falling と同じ米綴り属性 `RegistanceX` / `RegistanceY` で
/// 与える。非既定値を渡すと減衰式（Fall.java L107-108）に反映され、既定値
/// 0.05 / 0.1 の誤読では導かれない移動量になる。
///
/// X: InitialVX=10・RegistanceX=0.5・Gravity=0 → vX = 10 − 10*0.5 = 5 → dx=5 →
///    (1000,500) → (1005,500)（旧キー "ResistanceX" 誤読時は vX=9.5・dx=10）
/// Y: InitialVY=10・RegistanceY=0.5・Gravity=0 → vY = 10 − 10*0.5 = 5 → dy=5 →
///    (500,500) → (500,505)（旧キー "ResistanceY" 誤読時は vY=9・dy=9）
#[test]
fn fall_registance_attributes_are_read_and_applied() {
    // X 成分: 水平減衰
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((1000, 500));
    let table = single_table("X", 1);
    let action = create(
        ActionKind::Fall,
        &attrs(&[
            ("InitialVX", "10"),
            ("RegistanceX", "0.5"),
            ("Gravity", "0"),
        ]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (1005, 500),
        "RegistanceX=0.5: vX = 10 - 10*0.5 = 5（ResistanceX 誤読なら既定 0.05 → dx=10）"
    );

    // Y 成分: 垂直減衰
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((500, 500));
    let table = single_table("X", 1);
    let action = create(
        ActionKind::Fall,
        &attrs(&[
            ("InitialVY", "10"),
            ("RegistanceY", "0.5"),
            ("Gravity", "0"),
        ]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (500, 505),
        "RegistanceY=0.5: vY = 10 - 10*0.5 = 5（ResistanceY 誤読なら既定 0.1 → dy=9）"
    );
}

// =====================================================================
// Fall 初速の int 切り捨て（Fall.java L82-83 / getInitialVx L155-157 /
// getInitialVy L159-161）
//
// Java は getInitialVx()/getInitialVy() が `eval(..., Number.class, 0).intValue()`
// を返す（= 0 方向切り捨て・NaN→0・飽和の i32）。その i32 に scaling を掛けて
// 初速 velocityX/velocityY を作る。非整数式（実資産 `${-15-Math.random()*5}` 等）は
// 切り捨てられてから scale されるため、f64 のまま scale すると最初の移動量
// `dx = round(velocityX + modX)`（L117）が 1px ずれる。
// =====================================================================

/// 契約 1: InitialVX が非整数式のとき、初速は切り捨て後の整数に基づく。
/// InitialVX=`${-15.7}`・scale 1.0・既定 RegistanceX=0.05:
///   正: vX = (int)(-15.7) * 1.0 = -15
///       tick1: vX = -15 - (-15*0.05) = -14.25・modX = -0.25
///              dx = Java round(-14.5) = -14 → (500-14, 500+2) = (486, 502)
///   誤: vX = -15.7 → vX=-14.915・modX=-0.915 → dx = round(-15.83) = -16
///       → (484, 502)（RED 検出）
#[test]
fn fall_initial_vx_non_integer_truncates_before_scaling() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((500, 500));
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Fall,
        &attrs(&[("InitialVX", "${-15.7}")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (486, 502),
        "InitialVX=-15.7 は intValue() で -15 に切り捨ててから scale（dx=-14）。\
         f64 のままなら dx=-16 で (484,502) になる"
    );
}

/// 契約 2: マスコット set scale ≠ 1 でも適用順序は「切り捨て → scaling」。
/// InitialVX=`${-15.7}`・set scale 2.0（env.scaling=7.0 で非依存を pin）:
///   正: vX = (int)(-15.7) * 2.0 = -30
///       tick1: vX = -30 - (-30*0.05) = -28.5・modX = -0.5
///              dx = Java round(-29.0) = -29 → (500-29, 500+4) = (471, 504)
///       （重力も scale: 2*2=4）
///   誤: vX = -15.7*2.0 = -31.4 → vX=-29.83・modX=-0.83
///       → dx = round(-30.66) = -31 → (469, 504)（RED 検出）
#[test]
fn fall_initial_vx_truncates_before_scaling_order() {
    let mut env = SynthEnv::new();
    env.scaling_value = 7.0; // 非依存 pin（set scale 2.0 が採用される）
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at_scale((500, 500), 2.0);
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Fall,
        &attrs(&[("InitialVX", "${-15.7}")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (471, 504),
        "切り捨て → scale の順（-15*2=-30 → dx=-29）。scale 先行なら -31.4 → dx=-31"
    );
}

/// 契約 3（回帰）: もともと整数の式は従来どおり。
/// InitialVX=`${-15}`・scale 1.0 → (int)(-15)=-15 → 契約 1 の正解と同じ (486, 502)。
#[test]
fn fall_initial_vx_integer_expression_unchanged() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((500, 500));
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Fall,
        &attrs(&[("InitialVX", "${-15}")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (486, 502), "整数式 -15 は切り捨て不要（回帰）");
}

/// 契約 4: InitialVY も同様に切り捨ててから scale（Fall.java L83 / L159-161）。
/// 重力・抵抗の影響を消すため Gravity=0・RegistanceY=0 とし、初回 tick の
/// anchor.y 移動量として観測する。
/// InitialVY=`${15.7}`・scale 1.0:
///   正: vY = (int)(15.7) * 1.0 = 15 → dy = round(15) = 15 → (500, 515)
///   誤: vY = 15.7・modY = 0.7 → dy = round(16.4) = 16 → (500, 516)（RED 検出）
#[test]
fn fall_initial_vy_non_integer_truncates_before_scaling() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((500, 500));
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Fall,
        &attrs(&[
            ("InitialVY", "${15.7}"),
            ("Gravity", "0"),
            ("RegistanceY", "0"),
        ]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (500, 515),
        "InitialVY=15.7 は intValue() で 15 に切り捨て（dy=15）。\
         f64 のままなら modY=0.7 で dy=16 になる"
    );
}

// =====================================================================
// Jump 契約（Jump.java L43-97）
// =====================================================================

/// 放物線 distanceY = targetY − anchor.y − |distanceX| / 2（L60/77 逐語）+
/// 着地スナップ（distance <= velocity → location=target・L94-96）+ lookRight 更新
/// （L72-74・anchor.x < targetX → true）。
#[test]
fn jump_parabolic_arc_snaps_at_target() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((100, 500));
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Jump,
        &attrs(&[
            ("TargetX", "140"),
            ("TargetY", "520"),
            ("VelocityParam", "20"),
        ]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (120, 500), "tick1: velocity=(20,0)");
    assert!(m.look_right(), "L72-74: setLookRight(anchor.x < targetX)");

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (138, 509),
        "tick2: velocity=(round(17.88), round(8.94))"
    );

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (140, 520),
        "tick3: L94-96 distance <= velocity → target にスナップ"
    );
}

// =====================================================================
// Move 契約（Move.java L36-146）
// =====================================================================

/// target に対する overshoot クランプ（L90-103）+ lookRight 強制更新（L69-75）。
#[test]
fn move_overshoot_clamps_to_target() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 32);
    let mut m = mascot_at((100, 1040));
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Move,
        &attrs(&[("TargetX", "120")]),
        vec![anim(
            None,
            false,
            vec![pose("p.png", (64, 104), (-2, 0), 3)],
        )],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    for t in 1..=10 {
        m.tick(&env, &table, &mut factory, &mut rng);
        assert_eq!(
            m.anchor(),
            (100 + 2 * t, 1040),
            "tick{}: lookRight=true → anchor += 2",
            t
        );
    }
    // tick11: hasNext（anchor.x != targetX）false → 完了遷移 → 移動停止
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (120, 1040), "tick11: 完了 → 移動停止");
}

/// turning（IsTurn 考慮の getAnimation オーバーライド・L107-126）:
/// 方向転換中は is_turn=true のアニメが選ばれ、完了後は通常アニメへ切替る。
#[test]
fn move_turning_selects_is_turn_animation() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 32);
    // image() が Some になるよう turn/walk 両フレームを用意（apply_pose 契約:
    // 欠落フレーム → None。空 image set だと image pin が検証不能）
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("walk.png", 128, 128), ("turn.png", 128, 128)]),
        (100, 1040),
    );
    let table = single_table("X", 1);

    let anims = vec![
        anim(None, false, vec![pose("walk.png", (64, 64), (-2, 0), 30)]),
        anim(None, true, vec![pose("turn.png", (64, 64), (0, 0), 3)]),
    ];
    let action = create(ActionKind::Move, &attrs(&[("TargetX", "120")]), anims, 1.0);
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    // tick1-3: anchor(100) < targetX(120) → turning=true + lookRight=true →
    // ターンアニメ（velocity 0・duration 3）が適用される
    for t in 1..=3 {
        m.tick(&env, &table, &mut factory, &mut rng);
        assert_eq!(
            m.anchor(),
            (100, 1040),
            "tick{}: ターンアニメ中（velocity 0）",
            t
        );
        let img = m.image().expect("ターンアニメのフレーム");
        assert_eq!(
            img.image_ref, "turn.png",
            "turning == isTurn() のアニメが選ばれる（L115 逐語）"
        );
    }
    // tick4-5: turning 完了 → 通常アニメ（velocity -2,0 → lookRight=true で +2）
    for t in 4..=5 {
        m.tick(&env, &table, &mut factory, &mut rng);
        assert_eq!(
            m.anchor(),
            (100 + 2 * (t - 3), 1040),
            "tick{}: 通常アニメ",
            t
        );
        let img = m.image().expect("通常アニメのフレーム");
        assert_eq!(img.image_ref, "walk.png", "turning 完了 → 通常アニメ");
    }
}

// =====================================================================
// pose scale（design §1.8(h)・構築時 scale 変換）
// =====================================================================

/// アクション構築時に imageset::scale_pose 相当（AnimationBuilder L206-211
/// Java round half-up・非ゼロ→0 丸まりは符号付き ±1 補正）が適用される。
/// scale=2.0: velocity (2,1) → (4,2)・anchor (32,48) → (64,96)。
#[test]
fn pose_scale_applied_at_action_construction() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("p.png", 128, 128)]),
        (100, 500),
    );
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Animate,
        &attrs(&[]),
        vec![anim(None, false, vec![pose("p.png", (32, 48), (2, 1), 5)])],
        2.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (104, 502),
        "構築時プリスケール: velocity (2,1)*2 = (4,2)"
    );
    let img = m.image().expect("フレームは存在する");
    assert_eq!(
        img.center,
        (64, 96),
        "構築時プリスケール: anchor (32,48)*2 = (64,96)（round half-up）"
    );
}

// =====================================================================
// Dragged 契約（Dragged.java L25-122）
// =====================================================================

/// anchor = cursor + offset（L102）+ footDx = (footDx + (newX − footX) * 0.1) * 0.8
/// （L91）+ FootX/FootDX 注入がアニメ条件として機能する（design §1.8(c)）+
/// lookRight=false 強制（L67）。
#[test]
fn dragged_offset_foot_variables_injected_into_animation_selection() {
    let mut env = SynthEnv::new();
    env.cursor.x = 1010;
    env.cursor.y = 100;
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("a.png", 32, 32), ("b.png", 32, 32)]),
        (1000, 500),
    );
    let mut rng = FakeRng::repeated(0.5, 16);
    // cursor.x(1010) で footX = 1010（OffsetX 既定 0）
    let mut action = create(
        ActionKind::Dragged,
        &attrs(&[]),
        vec![
            anim(
                Some(var("#{FootDX < 0.5}")),
                false,
                vec![pose("a.png", (32, 16), (0, 0), 5)],
            ),
            anim(
                Some(var("#{FootDX >= 0.5}")),
                false,
                vec![pose("b.png", (32, 16), (0, 0), 5)],
            ),
        ],
        1.0,
    )
    .unwrap();
    action.init(&mut m, &env, &mut rng).unwrap();

    // tick1: カーソル静止（newX = footX）→ footDx = 0 → `FootDX < 0.5` → a.png
    action.next(&mut m, &env, &mut rng).unwrap();
    assert_eq!(
        m.anchor(),
        (1010, 220),
        "anchor = cursor + (OffsetX, OffsetY=120)（L102・既定 offsetY 120）"
    );
    assert_eq!(
        m.image().map(|i| i.image_ref.clone()),
        Some("a.png".to_string()),
        "footDx=0（注入 FootDX）を条件に A を選択"
    );
    assert!(!m.look_right(), "Dragged L67: setLookRight(false) 強制");

    // tick2: cursor を +10 → footDx = (0 + 10*0.1) * 0.8 = 0.8 → `FootDX >= 0.5` → b.png
    env.cursor.x = 1020;
    action.next(&mut m, &env, &mut rng).unwrap();
    assert_eq!(m.anchor(), (1020, 220));
    assert_eq!(
        m.image().map(|i| i.image_ref.clone()),
        Some("b.png".to_string()),
        "footDx = (footDx + Δ*0.1)*0.8 = 0.8（L91 逐語）"
    );
}

/// OffsetType="Origin" は「直前 tick までに適用された画像」の center 基準
/// （Java Dragged.java tick の実行順 pin・Java 正本確認済み）:
/// L73-78 offset 計算（getMascot().getImage().getCenter() 使用・当該 tick の
/// apply は未実行のため直前 tick の画像 center）→ L80-82 距離判定/setTime(0)
/// → L91-92 footDx 減衰 → L95-96 putVariable → L99 apply（ここで画像が変わる）
/// → L102 anchor = cursor + offset。
/// よってドラッグ中に pose が切り替わると offset の基準 center は 1 tick 遅れる
/// （観測可能だが軽微・Java 順序に寄せる決定済み）。
///
/// fixture: ドラッグ開始前に画像 A（center (100,50)）を apply_pose 経由で適用済み。
/// アニメは pose B（center (40,70)）のみ → tick1 の apply で A→B に切り替わる。
/// scaling=1.0・OffsetX=5・OffsetY=10 → raw offset = (5, 10)。
///
/// Java 手計算（cursor (1100,200) 固定・L73-78/L99/L102）:
///   tick1: offset = (A.center.x−5, A.center.y−10) = (100−5, 50−10) = (95, 40)
///          → anchor = (1100+95, 200+40) = (1195, 240)。apply 後の画像は B
///   tick2: offset = (B.center.x−5, B.center.y−10) = (40−5, 70−10) = (35, 60)
///          → anchor = (1100+35, 200+60) = (1135, 260)
/// （apply 先行の誤実装では tick1 が B.center 基準 (1135, 260) になり RED）
#[test]
fn dragged_offset_type_origin_uses_pre_apply_image_center() {
    let mut env = SynthEnv::new();
    env.cursor.x = 1100;
    env.cursor.y = 200;
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("a.png", 128, 128), ("b.png", 128, 128)]),
        (1000, 500),
    );
    // fixture: 画像 A（center (100,50)）をドラッグ開始前に適用済みにする
    //（Java Pose.apply 経由・look_right 既定 false → center = pose.anchor）
    shimeji::mascot::animation::apply_pose(&pose("a.png", (100, 50), (0, 0), 1), &mut m);
    assert_eq!(
        m.image().map(|i| i.center),
        Some((100, 50)),
        "fixture: 画像 A（center 100,50）適用済み"
    );

    let mut rng = FakeRng::repeated(0.5, 8);
    let mut action = create(
        ActionKind::Dragged,
        &attrs(&[
            ("OffsetType", "Origin"),
            ("OffsetX", "5"),
            ("OffsetY", "10"),
        ]),
        // 単一 pose B（center (40,70)）のみ: tick1 の apply（L99）で A→B へ切替
        vec![anim(None, false, vec![pose("b.png", (40, 70), (0, 0), 5)])],
        1.0,
    )
    .unwrap();
    action.init(&mut m, &env, &mut rng).unwrap();
    // init（Java L56）: footX = cursor.x + round(OffsetX*scaling) = 1100 + 5 = 1105

    // tick1: offset 計算（L73-78）は直前画像 A の center (100,50) 基準
    // → offset = (95, 40) → anchor = cursor + offset = (1195, 240)。
    // apply（L99）はこの後で走り、画像が A→B へ切り替わる。
    action.next(&mut m, &env, &mut rng).unwrap();
    assert_eq!(
        m.anchor(),
        (1195, 240),
        "tick1: offset = A.center(100,50) − (5,10) = (95,40)（L73-78 が L99 apply より先）"
    );
    assert_eq!(
        m.image().map(|i| i.image_ref.clone()),
        Some("b.png".to_string()),
        "tick1: apply（L99）で画像が A→B へ切り替わる"
    );

    // tick2: offset 計算は直前 tick の apply 済み画像 B の center (40,70) 基準
    // → offset = (35, 60) → anchor = (1135, 260)
    action.next(&mut m, &env, &mut rng).unwrap();
    assert_eq!(
        m.anchor(),
        (1135, 260),
        "tick2: offset = B.center(40,70) − (5,10) = (35,60)"
    );
}

/// 画像 None 時の Origin フォールバック: Java Dragged.java L76-77 は getImage()
/// が null だと NPE（正本では到達不能経路・apply が必ず画像を用意するため）。
/// Rust は契約として「image None なら Origin 補正をスキップし raw offset×scaling
/// を使う」を pin する（防御的フォールバック・Java NPE 相当経路の替わり）。
/// set scale 2.0（env.scaling=7.0 で非依存を pin）・OffsetX=5・OffsetY=10 →
/// raw offset = (round(10), round(20)) = (10, 20)
/// → anchor = cursor + (10, 20) = (110, 120)。
#[test]
fn dragged_offset_type_origin_skips_center_correction_when_image_none() {
    let mut env = SynthEnv::new();
    env.scaling_value = 7.0; // 非依存 pin（set scale 2.0 が採用される）
    env.cursor.x = 100;
    env.cursor.y = 100;
    // 空 image set → 画像は最初から None・アニメ pose も欠落フレームで None のまま
    let mut m = mascot_at_scale((100, 100), 2.0);
    assert!(m.image().is_none(), "fixture: 画像 None");

    let mut rng = FakeRng::repeated(0.5, 8);
    let mut action = create(
        ActionKind::Dragged,
        &attrs(&[
            ("OffsetType", "Origin"),
            ("OffsetX", "5"),
            ("OffsetY", "10"),
        ]),
        vec![anim(
            None,
            false,
            vec![pose("missing.png", (0, 0), (0, 0), 5)],
        )],
        1.0,
    )
    .unwrap();
    action.init(&mut m, &env, &mut rng).unwrap();
    action.next(&mut m, &env, &mut rng).unwrap();
    assert_eq!(
        m.anchor(),
        (110, 120),
        "image None → Origin 補正スキップ・raw offset×scaling = (10, 20) を使用"
    );
    assert!(
        m.image().is_none(),
        "欠落フレームの apply でも画像は None のまま"
    );
}

/// 抵抗時間 RNG 消費の短絡（L107-109）+ setTime(0) 抵抗リセット（L80-82）+ 抵抗延長:
/// (a)round　cursor 静止時（アンカーに届かない）は１度 setTime(0)（リセット）を
/// 吃し、時間を 250 tick 進めると getTime == timeToResist−1 で rng 消費が始まる。
#[test]
fn dragged_resist_rng_consumption_is_short_circuited() {
    let mut env = SynthEnv::new();
    env.cursor.x = 1010;
    env.cursor.y = 100;
    let mut m = mascot_at((1000, 500));
    m.set_image(Some(on_screen_image())); // 画面内 bounds 保持
    let mut rng = FakeRng::repeated(0.5, 400);
    let table = single_table("X", 1);
    let action = create(
        ActionKind::Dragged,
        &attrs(&[]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    // tick1: setTime(0) 抵抗リセット（|cursor.x + offsetX(0) − anchor.x|=10 >= 5）
    // → getTime が初期化されても毎 tick 進むため、250 回目 まで rng は消費しない
    for _ in 1..=249 {
        m.tick(&env, &table, &mut factory, &mut rng);
    }
    assert_eq!(
        rng.consumed(),
        0,
        "getTime が timeToResist-1 に届くまで rng を消費しない（短絡）"
    );
    m.tick(&env, &table, &mut factory, &mut rng); // tick250: getTime=249 == 249
    assert_eq!(
        rng.consumed(),
        1,
        "getTime==timeToResist-1 で Math.random() を消費"
    );
    m.tick(&env, &table, &mut factory, &mut rng); // tick251: getTime==250 == timeToResist−1
    assert_eq!(
        rng.consumed(),
        2,
        "rng 0.5 >= 0.1 → timeToResist++ → 抵抗延長で連続消費"
    );
}

// =====================================================================
// ThrowIE / WalkWithIE 契約
// =====================================================================

/// ThrowIE: throwing ゲート + activeIE 可視 + ウィンドウ ID 一致（Java L49-57）+
/// moveActiveIE の放物線式（L60-78 → 初速+getTime*gravity）。lookRight で
/// 投擲方向が反転する（L66-76）。
#[test]
fn throwie_moves_window_and_gates_on_window_id_change() {
    let mut env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((100, 500));
    let table = single_table("X", 1);

    let action = create(
        ActionKind::ThrowIE,
        &attrs(&[]), // InitialVX?=32・InitialVY=-10・Gravity=0.5 の既定
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    // ThrowIE 自身は lookRight を操作しない（ThrowIE.java L50-78 に
    // setLookRight 呼び出しなし）。Mascot 既定は false（Mascot.java L121）。
    m.set_look_right(true);
    assert!(
        m.look_right(),
        "pin 前提: lookRight を明示 true にして投擲方向を規定"
    );
    let mut factory = FnFactory::constant(make_idle_fallback);

    // tick1: lookRight=true → IE.left + 32, IE.top + round(-10 + 0*0.5) = top - 10
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        *env.moved_to.borrow(),
        [(300 + 32, 200 - 10)],
        "ThrowIE.java L67-70: lookRight=true で +32"
    );

    // 別経路: lookRight=false では -32（Java L73-75）
    let env_left = SynthEnv::new();
    let mut rng_left = FakeRng::repeated(0.5, 16);
    let mut m_left = mascot_at((100, 500));
    m_left.set_look_right(false);
    let action = create(
        ActionKind::ThrowIE,
        &attrs(&[]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m_left, &env_left, "X", action, &mut rng_left).unwrap();
    m_left.tick(
        &env_left,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut rng_left,
    );
    assert_eq!(
        *env_left.moved_to.borrow(),
        [(300 - 32, 200 - 10)],
        "lookRight=false では -32（L72-75）"
    );

    // ウィンドウ切替検出: activeWindowId 変化 → has_next false → moveActiveIE が止まる
    env.active_window_id = 9;
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        env.moved_to.borrow().len(),
        1,
        "ウィンドウ ID 変化で moveActiveIE が走らない（L49-57）"
    );

    // throwing 設定 false → has_next false
    env.throwing = false;
    env.active_window_id = 7;
    m.tick(&env, &table, &mut factory, &mut rng);
    assert!(
        env.moved_to.borrow().len() <= 1,
        "throwing=false では窓を動かさない"
    );

    let _ = (&env_left, &mut rng_left, &m_left);
}

/// WalkWithIE: throwing ゲート・hold-check（Java L61-71）+ moveActiveIE による
/// 窓移動（L76-88）。hold-check 失敗時は LostGround。
#[test]
fn walkwithie_holds_check_and_moves_window() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((310, 800)); // IE(offset 10) の左下角
                                       // WalkWithIE 自身は lookRight を操作しない（WalkWithIE.java L48-88 に
                                       // setLookRight 呼び出しなし）。Mascot 既定は false → 明示 true。
    m.set_look_right(true);
    let table = single_table("X", 1);

    // TargetX は Move.hasNext（L52-53）を成立させるために必要（無いと Java でも
    // hasNext=false で tick せず hold-check に到達しない）。velocity 0 で窓だけ動く。
    let action = create(
        ActionKind::WalkWithIE,
        &attrs(&[("IeOffsetX", "10"), ("TargetX", "320")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    // hold-check: lookRight → anchor.x − offsetX == IE.left が真 → 連動移動:
    // moveActiveIE(anchor.x − offsetX, anchor.y + offsetY − IE.height)
    //   = (310 − 10, 800 + 0 − 600) = (300, 200)（WalkWithIE.java L77-81）
    assert_eq!(
        *env.moved_to.borrow(),
        [(300, 200)],
        "WalkWithIE.java L77-81: hold 検証成功後に窓を連動移動（lookRight=true）"
    );

    // hold-check 失敗: anchor をずらす → LostGround（L62-65）→ time++・dispose 経路
    let env2 = SynthEnv::new();
    let mut rng2 = FakeRng::repeated(0.5, 16);
    let mut m2 = mascot_at((311, 800)); // offsetX 10 だと 301 != IE.left(300)
    m2.set_look_right(true); // lookRight 側ホールド検証（L62-64）に乗せる
    let action = create(
        ActionKind::WalkWithIE,
        &attrs(&[("IeOffsetX", "10"), ("TargetX", "320")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m2, &env2, "X", action, &mut rng2).unwrap();
    m2.tick(
        &env2,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut rng2,
    );
    assert_eq!(
        m2.time(),
        1,
        "hold-check 失敗 → LostGround（time++ は catch 外）"
    );
    assert!(
        m2.remove_pending(),
        "Fallback 未定義の single-table では dispose 相当"
    );
}

// =====================================================================
// Breed 契約（Breed.java L26-166）
// =====================================================================

/// BornX/BornY の lookRight 分岐（Delegate.breed Java L83-89 逐語）+
/// isPenultimateFrame（time == アニメ duration − 1・L69-71）+
/// queue_spawn（design §1.8(f): 次 tick 一括反映の意図的差異）。
#[test]
fn breed_spawns_once_at_penultimate_frame() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("p.png", 128, 128)]),
        (1000, 500),
    );
    m.set_look_right(true);
    let table = single_table("X", 1);

    // アニメ duration 2 → 最終フレーム（time 1）で breed
    let action = create(
        ActionKind::Breed,
        &attrs(&[("BornX", "16"), ("BornY", "32"), ("BornBehavior", "PullUp")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 2)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng);
    assert!(
        env.spawns.borrow().is_empty(),
        "ultimate frame（time 0）では生まない"
    );
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        *env.spawns.borrow(),
        [SpawnRec {
            image_set_name: "TestSet".to_string(),
            anchor: (984, 532),
            look_right: true,
            behavior_name: "PullUp".to_string(),
        }],
        "lookRight=true → BornX を減算（L84-89）・BornY を加算・親の lookRight を引継・\
         実 XML 属性 BornBehavior 名（\"PullUp\"）が queue の第 4 引数で伝播する（#8）"
    );
}

/// Breed のゲート（Delegate.isEnabled Java L59-63 逐語）:
/// BornTransient 既定 false → breeding ゲート / BornTransient=true → transients ゲート。
#[test]
fn breed_gates_breeding_and_transient_settings() {
    let table = single_table("X", 1);

    // (a) breeding=false・BornTransient=false（既定）→ 生まれない
    let mut env = SynthEnv::new();
    env.breeding = false;
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("p.png", 128, 128)]),
        (1000, 500),
    );
    let action = create(
        ActionKind::Breed,
        &attrs(&[("BornX", "16"), ("BornY", "32")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 2)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    m.tick(
        &env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut rng,
    );
    m.tick(
        &env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut rng,
    );
    assert!(
        env.spawns.borrow().is_empty(),
        "breeding forbidden → 生まれない"
    );

    // (b) BornTransient=true + transients=false → 生まれない / transients=true は
    // breeding=false でも生まれる
    let mut env = SynthEnv::new();
    env.breeding = false;
    env.transients = false;
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("p.png", 128, 128)]),
        (1000, 500),
    );
    let action = create(
        ActionKind::Breed,
        &attrs(&[("BornX", "16"), ("BornY", "32"), ("BornTransient", "true")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 2)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    m.tick(
        &env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut rng,
    );
    m.tick(
        &env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut rng,
    );
    assert!(
        env.spawns.borrow().is_empty(),
        "transients disabled → 生まれない"
    );

    let mut env = SynthEnv::new();
    env.breeding = false;
    env.transients = true;
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("p.png", 128, 128)]),
        (1000, 500),
    );
    let action = create(
        ActionKind::Breed,
        &attrs(&[("BornX", "16"), ("BornY", "32"), ("BornTransient", "true")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 2)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    m.tick(
        &env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut rng,
    );
    m.tick(
        &env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut rng,
    );
    assert_eq!(
        env.spawns.borrow().len(),
        1,
        "transients only で breeding 許可は不要"
    );
    assert_eq!(
        env.spawns.borrow()[0].behavior_name,
        "",
        "実 XML 属性 BornBehavior 省略時は既定値（空文字列・BorderedAction \
         BREED_DEFAULT_BORN_BEHAVIOR）が第 4 引数で渡る（#8）"
    );
}

/// 実資産 conf/actions.xml をパースする（Breed の「実資産パース → 構築 → spawn 名」
/// 統合経路用。合成属性だけでは属性名の取り違えを検出できない穴を塞ぐ）。
fn real_actions_config() -> ActionsConfig {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("conf")
        .join("actions.xml");
    parse_actions(&path).expect("conf/actions.xml をパースできる")
}

/// 実資産から構築した Breed を完走させ、spawn キューに積まれた子 behavior 名を返す。
fn real_breed_child_behavior_name(action_name: &str) -> String {
    let cfg = real_actions_config();
    let env = SynthEnv::new();
    let action = build_action(&cfg, action_name, &VarMap::new(), 1.0)
        .unwrap_or_else(|e| panic!("実資産 {action_name} を構築できる: {e:?}"));
    let mut m = mascot_at((1000, 500));
    let mut rng = FakeRng::repeated(0.5, 256);
    set_action(&mut m, &env, action_name, Ok(action), &mut rng).unwrap();
    let table = single_table(action_name, 1);
    let mut factory = FnFactory::constant(make_idle_fallback);
    // 実資産 Breed のアニメ長は最大 140 tick（PullUpShimeji1）。余裕を持って回す。
    for _ in 0..200 {
        m.tick(&env, &table, &mut factory, &mut rng);
        if !env.spawns.borrow().is_empty() {
            break;
        }
    }
    let spawns = env.spawns.borrow();
    assert_eq!(
        spawns.len(),
        1,
        "実資産 {action_name} は子を 1 体 spawn する"
    );
    spawns[0].behavior_name.clone()
}

/// 実資産の Breed アクションは実 XML 属性 BornBehavior を子 behavior 名として
/// spawn キューへ伝播する（Breed.java L93 getBornBehavior 相当）:
/// - Divide1: BornBehavior="Divided" → "Divided"
/// - PullUpShimeji1: BornBehavior="PullUp" → "PullUp"
///
/// 論理キー BornBehaviour を生 XML 名として誤読すると空文字になり RED。
#[test]
fn breed_from_real_actions_spawns_declared_born_behavior_name() {
    assert_eq!(
        real_breed_child_behavior_name("Divide1"),
        "Divided",
        "実資産 Divide1 の BornBehavior=\"Divided\" が子 behavior 名として伝播する"
    );
    assert_eq!(
        real_breed_child_behavior_name("PullUpShimeji1"),
        "PullUp",
        "実資産 PullUpShimeji1 の BornBehavior=\"PullUp\" が子 behavior 名として伝播する"
    );
}

// =====================================================================
// Regist 契約（Regist.java L25-99）
// =====================================================================

/// hasNext = |cursor.x − anchor.x + offsetX| < 5（L61）+ 終了時
/// `Math.random() < 0.5` の rng 消費で lookRight 更新 + LostGround（L71-77）。
#[test]
fn regist_completes_with_rng_lookright_and_lost_ground() {
    // (a) カーソルが近い → アニメ duration を超えると終了・lookRight は rng で択一
    let mut env = SynthEnv::new();
    env.cursor.x = 100; // anchor (100,500) に近い → hold 継続（L61）
    env.cursor.y = 200;
    let mut m = mascot_at((100, 500));
    let mut rng = FakeRng::repeated(0.5, 16);
    let table = single_table("X", 1);
    let action = create(
        ActionKind::Regist,
        &attrs(&[]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 3)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);

    m.tick(&env, &table, &mut factory, &mut rng); // getTime 0: 終了未満
    m.tick(&env, &table, &mut factory, &mut rng); // getTime 1
    m.tick(&env, &table, &mut factory, &mut rng); // getTime 2: 2+1 >= 3 → 終了
                                                  // resist rng 1 回（0.5 < 0.5 → false → lookRight=false）のみ・
                                                  // LostGround catch は build_behavior_direct("Fall")
                                                  // 経路（rng 引数なし）のため rng 消費しない
    assert_eq!(
        rng.consumed(),
        1,
        "Java Regist L74（lookRight 択・rng 1 回）のみ。LostGround catch は \
         table.build_behavior_direct(\"Fall\") 経路（fallback 構築・rng 引数なし）に \
         より動作するため Math.random() を消費しない（behavior.rs L246）"
    );
    assert!(
        !m.look_right(),
        "Regist L74: rng 0.5 < 0.5 → false → setLookRight(false)"
    );
    assert_eq!(m.time(), 3, "LostGround catch は time++ を進める");

    // (b) カーソルが遠い（差 >= 5）→ has_next false（L61）
    let mut env2 = SynthEnv::new();
    env2.cursor.x = 500;
    let mut m2 = mascot_at((100, 500));
    let mut rng2 = FakeRng::repeated(0.5, 8);
    let mut action = create(
        ActionKind::Regist,
        &attrs(&[]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 3)])],
        1.0,
    )
    .unwrap();
    action.init(&mut m2, &env2, &mut rng2).unwrap();
    assert!(
        !action.has_next(&mut m2, &env2, &mut rng2).unwrap(),
        "cursor.x が anchor.x と 5 以上離れると hold 失敗（L61）"
    );
}

// =====================================================================
// Sequence / Select / InstantAction / 参照属性マージ
// =====================================================================

/// Sequence 子は順に走る（ComplexAction.seek L43-53）+ 子の Duration attr で
/// seek が進む（子の hasNext が false になった時点で currentAction を進める）。
#[test]
fn sequence_children_run_sequentially() {
    let cfg = actions_config(vec![
        (
            "Seq",
            ActionDef::Sequence {
                // 不変契約（Java ComplexAction.java L18: ActionBase 直下・border 無し）+
                // 子が Floor 境界だと床外 anchor で即 LostGround
                //（Animate.java L34-36）してしまいループ検証不能のため border None
                border: None,
                attrs: VarMap::new(),
                is_loop: false,
                animations: vec![],
                children: vec![
                    // Inline 子: Duration="1" → 1 tick 分だけ動く
                    SequenceChild::Inline(Box::new(ActionDef::Animate {
                        border: None,
                        attrs: attrs(&[("Duration", "1")]),
                        animations: vec![anim(
                            None,
                            false,
                            vec![pose("a.png", (64, 64), (1, 0), 5)],
                        )],
                    })),
                    SequenceChild::Ref {
                        name: "RefB".to_string(),
                        attrs: attrs(&[("Duration", "2")]),
                    },
                ],
            },
        ),
        (
            "RefB",
            ActionDef::Animate {
                border: None, // LostGround 回避のため兄弟子と同一・床外 anchor は無境界
                attrs: VarMap::new(),
                animations: vec![anim(None, false, vec![pose("b.png", (64, 64), (2, 0), 5)])],
            },
        ),
    ]);
    let extra = VarMap::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let env = SynthEnv::new();
    let mut m = mascot_at((1000, 500));
    let action = build_action(&cfg, "Seq", &extra, 1.0);
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let table = single_table("X", 1);
    let mut factory = FnFactory::constant(make_idle_fallback);

    // tick1: c1（velocity 1・Duration 1）→ anchor 1001
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (1001, 500), "tick1: Inline c1");
    // tick2: seek → c2（RefB velocity 2・Duration 2）→ anchor 1003
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (1003, 500), "tick2: seek 後 c2");
    // tick3: c2 tick2 → anchor 1005
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (1005, 500), "tick3: c2");
    // tick4: c2 完了（Duration 2 超過）→ Sequence 完了 → idle
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (1005, 500), "tick4: 完了 → idle");
}

/// Sequence の Loop 属性: setCurrentAction は
/// `is_loop ? currentAction % len : currentAction` 逐語（Sequence.java L34-36）。
/// is_loop=true → 1 子を再 init して延々と続く（anchor が進み続ける）。
#[test]
fn sequence_loop_wraps_modulo() {
    // (a) is_loop = false → c1 完了後に Sequence も完了（idle に遷移して停止）
    {
        let cfg = actions_config(vec![(
            "Seq",
            ActionDef::Sequence {
                // border None（上記 sequence_children_run_sequentially の契約と同文）
                border: None,
                attrs: VarMap::new(),
                is_loop: false,
                animations: vec![],
                children: vec![SequenceChild::Inline(Box::new(ActionDef::Animate {
                    border: None,
                    attrs: attrs(&[("Duration", "1")]),
                    animations: vec![anim(None, false, vec![pose("a.png", (64, 64), (1, 0), 5)])],
                }))],
            },
        )]);
        let mut rng = FakeRng::repeated(0.5, 16);
        let env = SynthEnv::new();
        let mut m = mascot_at((1000, 500));
        let action = build_action(&cfg, "Seq", &VarMap::new(), 1.0);
        set_action(&mut m, &env, "X", action, &mut rng).unwrap();
        let table = single_table("X", 1);
        let mut factory = FnFactory::constant(make_idle_fallback);
        m.tick(&env, &table, &mut factory, &mut rng);
        assert_eq!(m.anchor(), (1001, 500));
        m.tick(&env, &table, &mut factory, &mut rng);
        assert_eq!(m.anchor(), (1001, 500), "is_loop=false → 完了");
        for _ in 0..5 {
            m.tick(&env, &table, &mut factory, &mut rng);
        }
        assert_eq!(m.anchor(), (1001, 500), "is_loop=false → 以後も動かない");
    }

    // (b) is_loop = true → c1 を再 init し続け、anchor が進み続ける
    // （ActionDef::Sequence{is_loop} がパース参照として実装される契約 pin）
    {
        let cfg = actions_config(vec![(
            "Seq",
            ActionDef::Sequence {
                border: None,
                attrs: VarMap::new(),
                is_loop: true,
                animations: vec![],
                children: vec![SequenceChild::Inline(Box::new(ActionDef::Animate {
                    border: None,
                    attrs: attrs(&[("Duration", "1")]),
                    animations: vec![anim(None, false, vec![pose("a.png", (64, 64), (1, 0), 5)])],
                }))],
            },
        )]);
        let mut rng = FakeRng::repeated(0.5, 32);
        let env = SynthEnv::new();
        let mut m = mascot_at((1000, 500));
        let action = build_action(&cfg, "Seq", &VarMap::new(), 1.0);
        set_action(&mut m, &env, "X", action, &mut rng).unwrap();
        let table = single_table("X", 1);
        let mut factory = FnFactory::constant(make_idle_fallback);
        // Loop により子が再 init され続けて anchor が進む（6 tick）
        for t in 1..=5 {
            m.tick(&env, &table, &mut factory, &mut rng);
            assert_eq!(m.anchor(), (1000 + t, 500), "tick{}: loop 再 init", t);
        }
    }
}

/// InstantAction: Look は init 中に mascot 状態を変えて has_next false（Java
/// Look.java L25-32 / InstantAction.java L26-43）。
#[test]
fn look_flips_look_right_and_completes_instantly() {
    let env = SynthEnv::new();
    let mut m = mascot_at((1000, 500));
    let mut rng = FakeRng::repeated(0.5, 8);

    let mut action = create(
        ActionKind::Look,
        &attrs(&[("LookRight", "true")]),
        vec![],
        1.0,
    )
    .unwrap();
    action.init(&mut m, &env, &mut rng).unwrap();
    assert!(m.look_right(), "Look.java L27: setLookRight(isLookRight())");
    assert!(
        !action.has_next(&mut m, &env, &mut rng).unwrap(),
        "InstantAction は has_next 常時 false（Java L37-39）"
    );
}

/// Offset（InstantAction）: anchor += (X, Y)（Java Offset.java L40-58 逐語・
/// scale を使わない quirk コメント保持）。
#[test]
fn offset_window_translates_without_scaling() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 8);
    let mut m = mascot_at((1000, 500));
    let mut action = create(
        ActionKind::Offset,
        &attrs(&[("X", "-50"), ("Y", "200")]),
        vec![],
        2.0, // scale が入っても X/Y は素通し（scaling 抑止 quirk）
    )
    .unwrap();
    action.init(&mut m, &env, &mut rng).unwrap();
    assert_eq!(
        m.anchor(),
        (950, 700),
        "Offset.java L55-57: translate(X, Y)"
    );
    assert!(
        !action.has_next(&mut m, &env, &mut rng).unwrap(),
        "Offset は has_next false"
    );
}

/// Select（Select.java = ComplexAction そのまま）: 条件が false の子は seek で
/// スキップされ、条件成立する最初の子が常に選択される。
#[test]
fn select_skips_ineffective_children_and_uses_first_effective() {
    let cfg = actions_config(vec![(
        "Sel",
        ActionDef::Select {
            border: None, // 子の Animate（Floor 外 anchor）が即 LostGround するのを回避
            attrs: VarMap::new(),
            is_loop: false,
            animations: vec![],
            children: vec![
                // c1: Condition=false → seek でスキップされる
                SequenceChild::Inline(Box::new(ActionDef::Animate {
                    border: None,
                    attrs: attrs(&[("Condition", "false"), ("Duration", "5")]),
                    animations: vec![anim(None, false, vec![pose("x.png", (0, 0), (1, 0), 5)])],
                })),
                // c2: 常時有効（velocity 2・Duration 2）
                SequenceChild::Inline(Box::new(ActionDef::Animate {
                    border: None,
                    attrs: attrs(&[("Duration", "2")]),
                    animations: vec![anim(None, false, vec![pose("y.png", (0, 0), (2, 0), 5)])],
                })),
            ],
        },
    )]);
    let mut rng = FakeRng::repeated(0.5, 16);
    let env = SynthEnv::new();
    let mut m = mascot_at((1000, 500));
    let action = build_action(&cfg, "Sel", &VarMap::new(), 1.0);
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let table = single_table("X", 1);
    let mut factory = FnFactory::constant(make_idle_fallback);

    // c1 をスキップして c2 が選ばれる → anchor += 2
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (1002, 500),
        "tick1: c1 はスキップされ c2 が走る"
    );
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (1004, 500), "tick2: c2");
    // tick3: c2 Duration 2 完了 → Select 完了 → idle
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(m.anchor(), (1004, 500), "tick3: 完了遷移");
}

/// Ref 属性マージは Ref 側優先（Java ActionRef.buildAction L113-126 逐語）:
/// 参照先 ActionDef の属性（LookRight="true"）よりも Ref 自身の属性
/// （LookRight="false"）が勝つ。逆方向（Ref 側なし）では参照先の属性が使われる。
#[test]
fn sequence_ref_attrs_override_referenced_action_def() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((1000, 500));

    // 参照先 "Look" は LookRight="true" を持つが、Ref 側 "false" が勝つ
    let cfg = actions_config(vec![
        (
            "Look",
            ActionDef::Embedded {
                class: "com.group_finity.mascot.action.Look".to_string(),
                border: Some(BorderType::Floor),
                attrs: attrs(&[("LookRight", "true")]),
                animations: vec![],
            },
        ),
        (
            "S",
            ActionDef::Sequence {
                border: Some(BorderType::Floor),
                attrs: VarMap::new(),
                is_loop: false,
                animations: vec![],
                children: vec![SequenceChild::Ref {
                    name: "Look".to_string(),
                    attrs: attrs(&[("LookRight", "false")]),
                }],
            },
        ),
    ]);
    let action = build_action(&cfg, "S", &VarMap::new(), 1.0);
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    assert!(
        !m.look_right(),
        "Ref 側属性（false）が参照先（true）より優先される"
    );
    // Sequence c1 = Look（InstantAction）→ 即 seek 完了 → Sequence hasNext false
    // （m.set_action の遷移で idle に落ちていれば pin した形で呼ばせる）
}

/// Inline（ActionBuilder 子・Java createVariables L496-505）は caller 優先:
/// build_action の extra パラメータが Inline 自身の属性を上書きする。
#[test]
fn inline_top_level_action_caller_params_win() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at((1000, 500));

    let cfg = actions_config(vec![(
        "InlineLook",
        ActionDef::Embedded {
            class: "com.group_finity.mascot.action.Look".to_string(),
            border: Some(BorderType::Floor),
            attrs: attrs(&[("LookRight", "false")]),
            animations: vec![],
        },
    )]);
    let extra = attrs(&[("LookRight", "true")]);
    let action = build_action(&cfg, "InlineLook", &extra, 1.0);
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    assert!(
        m.look_right(),
        "caller params（true）が Inline 自身の属性（false）を上書きする"
    );
}

// =====================================================================
// ActionReference 属性の子アクションへの伝播（Java ActionRef.java L66 /
// ActionBuilder.createVariables L486-507）
//
// ChaseMouse 実資産の Dash 参照は Ref 側属性 `TargetX`
// （#{mascot.environment.cursor.x+Gap}）と `Gap`（${...}）を持つ
// （conf/actions.xml L668-671）。Ref の全属性は子アクションの識別子空間
// （VariableMap）へ載る必要があり、Gap が解決できないと式評価エラー →
// Mascot dispose になる（本修正の再現経路）。
// =====================================================================

/// ActionDef を再帰的に辿り、指定名の ActionReference が持つ attrs を返す
/// （`required_attr` を持つ Ref のみ）。実資産からの Ref 属性取り出し用。
fn find_ref_attrs(def: &ActionDef, ref_name: &str, required_attr: &str) -> Option<VarMap> {
    let children = match def {
        ActionDef::Sequence { children, .. } | ActionDef::Select { children, .. } => children,
        _ => return None,
    };
    for child in children {
        match child {
            SequenceChild::Ref { name, attrs }
                if name == ref_name && attrs.contains_key(required_attr) =>
            {
                return Some(attrs.clone());
            }
            SequenceChild::Inline(inner) => {
                if let Some(found) = find_ref_attrs(inner, ref_name, required_attr) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

/// 実資産統合: ChaseMouse の Gap 付き Dash 参照を実パースから取り出して
/// build_action で構築 → init がエラーにならない（現状は Gap 未解決で RED）。
/// 続けて tick させ、カーソル（左）方向へ anchor.x が動き、Gap = ±数百 px の
/// 有限値に由来する TargetX へ収束することを観測する（Math.random の値そのものは
/// pin せず範囲で検証）。
#[test]
fn chase_mouse_gap_dash_from_real_assets_resolves_and_moves() {
    let cfg = real_actions_config();
    let chase = cfg
        .actions
        .get("ChaseMouse")
        .expect("実資産に ChaseMouse が存在する");
    let ref_attrs =
        find_ref_attrs(chase, "Dash", "Gap").expect("ChaseMouse に Gap 付き Dash 参照が存在する");
    assert!(ref_attrs.contains_key("TargetX"), "Ref 側に TargetX を持つ");
    assert!(ref_attrs.contains_key("Gap"), "Ref 側に Gap を持つ");

    let env = SynthEnv::new(); // cursor (300, 200) / work area bottom 1040
    let mut rng = FakeRng::repeated(0.5, 400);
    let mut m = mascot_at((1000, 1040)); // 床上

    let mut action = build_action(&cfg, "Dash", &ref_attrs, 1.0)
        .expect("実資産 Dash 定義 + Ref 側 Gap/TargetX を build_action で構築できる");

    // init が TargetX="#{mascot.environment.cursor.x+Gap}" を評価する。
    // Gap が attrs として解決されなければ Err（現状 RED）。
    action
        .init(&mut m, &env, &mut rng)
        .expect("Gap 属性が識別子として解決され init がエラーにならない");

    let start_x = m.anchor().0;
    let cursor_x = env.cursor.x;
    for _ in 0..150 {
        action
            .next(&mut m, &env, &mut rng)
            .expect("tick がエラーにならない");
    }

    let final_x = m.anchor().0;
    assert!(
        final_x < start_x,
        "カーソルが左にあるため anchor.x は減少する（{start_x} -> {final_x}）"
    );
    assert!(
        (cursor_x..cursor_x + 200).contains(&final_x),
        "Gap は有限（anchor.x > cursor.x 側は [0,200)）→ TargetX=cursor.x+Gap は \
         [cursor.x, cursor.x+200) に収束する（cursor={cursor_x}, anchor.x={final_x}）"
    );
    assert_eq!(m.anchor().1, 1040, "床境界（Floor）に留まる");
}

// =====================================================================
// mascot set scale（ImageSet.scale）由来の物理量・位置
//
// 修正契約: アクション init の scale 取得元は env.scaling() ではなく
// `Mascot::scale()`（= 自分が保持する ImageSet.scale）。env.scaling を別値に
// しても結果が変わらないことで非依存を pin する。
// =====================================================================

/// Mascot::scale() は保持 ImageSet の解決済み scale を返し、rebind（Reload）後も
/// 新しい ImageSet の scale に自動追随する。
#[test]
fn mascot_scale_reflects_image_set_scale_and_rebind() {
    assert_eq!(mascot_at_scale((0, 0), 0.5).scale(), 0.5);
    assert_eq!(mascot_at((0, 0)).scale(), 1.0, "未指定は等倍");

    let mut m = mascot_at_scale((0, 0), 1.0);
    m.rebind_image_set(
        "TestSet",
        image_set_with_scale(&[("p.png", 128, 128)], 0.25),
    );
    assert_eq!(m.scale(), 0.25, "rebind 後の ImageSet.scale に追随");
}

/// Jump 初速（VelocityParam * scaling・Jump.java L81）はマスコットの set scale で
/// 決まり、env.scaling には依存しない。
/// set scale 2.0・env.scaling 7.0・VelocityParam 10・Target(300,500)・anchor(100,500):
///   velocity = 20
///   distanceX=200・distanceY=-100・distance=√50000
///   velocityX = 20*200/√50000 = 17.888… → dx=18
///   velocityY = 20*(-100)/√50000 = -8.944… → dy=-9
///   → (118, 491)
#[test]
fn jump_velocity_scales_with_mascot_set_scale() {
    let mut env = SynthEnv::new();
    env.scaling_value = 7.0; // 非依存 pin（set scale 2.0 が採用される）
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at_scale((100, 500), 2.0);
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Jump,
        &attrs(&[
            ("TargetX", "300"),
            ("TargetY", "500"),
            ("VelocityParam", "10"),
        ]),
        vec![anim(None, false, vec![pose("p.png", (0, 0), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        m.anchor(),
        (118, 491),
        "VelocityParam*2.0 由来（env.scaling=7.0 に依存しない）"
    );
}

/// Dragged の offset（round(OffsetX/Y * scaling)・Java L73-74/L102）は
/// マスコットの set scale で決まり、env.scaling には依存しない。
/// set scale 2.0・env.scaling 7.0・OffsetX=5・OffsetY=10・cursor(1000,200):
///   offset = (round(10), round(20)) = (10, 20) → anchor = (1010, 220)
#[test]
fn dragged_offset_scales_with_mascot_set_scale() {
    let mut env = SynthEnv::new();
    env.scaling_value = 7.0; // 非依存 pin
    env.cursor.x = 1000;
    env.cursor.y = 200;
    let mut m = mascot_at_scale((1000, 500), 2.0);
    let mut rng = FakeRng::repeated(0.5, 8);

    let mut action = create(
        ActionKind::Dragged,
        &attrs(&[("OffsetX", "5"), ("OffsetY", "10")]),
        vec![anim(None, false, vec![pose("p.png", (0, 0), (0, 0), 5)])],
        1.0,
    )
    .unwrap();
    action.init(&mut m, &env, &mut rng).unwrap();
    action.next(&mut m, &env, &mut rng).unwrap();
    assert_eq!(
        m.anchor(),
        (1010, 220),
        "offset = round((5,10)*2.0) = (10,20)（env.scaling=7.0 に依存しない）"
    );
}

/// Regist の hold 判定（|cursor.x − anchor.x + offsetX| < 5・Java L61）の offsetX は
/// マスコットの set scale で決まる。cursor.x=100・anchor.x=110・OffsetX=5:
///   set scale 2.0 → offsetX=10 → |100-110+10|=0 < 5 → 継続
///   scale 1.0（回帰）→ offsetX=5 → |100-110+5|=5 → 失敗
#[test]
fn regist_offset_scales_with_mascot_set_scale() {
    let mut env = SynthEnv::new();
    env.scaling_value = 7.0; // 非依存 pin
    env.cursor.x = 100;
    let mut rng = FakeRng::repeated(0.5, 8);
    let mut m = mascot_at_scale((110, 500), 2.0);
    let mut action = create(
        ActionKind::Regist,
        &attrs(&[("OffsetX", "5")]),
        vec![anim(None, false, vec![pose("p.png", (0, 0), (0, 0), 5)])],
        1.0,
    )
    .unwrap();
    action.init(&mut m, &env, &mut rng).unwrap();
    assert!(
        action.has_next(&mut m, &env, &mut rng).unwrap(),
        "set scale 2.0: offsetX=10 → 距離 0 → hold 継続（env.scaling=7.0 非依存）"
    );

    // 回帰: 未指定（scale 1.0）では従来どおり offsetX=5 で hold 失敗
    let mut env1 = SynthEnv::new();
    env1.cursor.x = 100;
    let mut rng1 = FakeRng::repeated(0.5, 8);
    let mut m1 = mascot_at((110, 500));
    let mut action1 = create(
        ActionKind::Regist,
        &attrs(&[("OffsetX", "5")]),
        vec![anim(None, false, vec![pose("p.png", (0, 0), (0, 0), 5)])],
        1.0,
    )
    .unwrap();
    action1.init(&mut m1, &env1, &mut rng1).unwrap();
    assert!(
        !action1.has_next(&mut m1, &env1, &mut rng1).unwrap(),
        "scale 1.0: offsetX=5 → 距離 5 → hold 失敗（従来どおり）"
    );
}

/// ThrowIE のウィンドウ投擲（Java L67-75: round(InitialV * scaling)）は
/// マスコットの set scale で決まり、env.scaling には依存しない。
/// set scale 2.0・env.scaling 7.0・既定 InitialVX=32 / InitialVY=-10 /
/// lookRight=true → dx=round(32*2)=64・dy=round(-10*2)=-20 →
/// IE(300,200) → (364, 180)。
#[test]
fn throwie_window_throw_scales_with_mascot_set_scale() {
    let mut env = SynthEnv::new();
    env.scaling_value = 7.0; // 非依存 pin
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_at_scale((100, 500), 2.0);
    m.set_look_right(true);
    let table = single_table("X", 1);

    let action = create(
        ActionKind::ThrowIE,
        &attrs(&[]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 5)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);
    m.tick(&env, &table, &mut factory, &mut rng);
    assert_eq!(
        *env.moved_to.borrow(),
        [(300 + 64, 200 - 20)],
        "dx=round(32*2.0)=64・dy=round(-10*2.0)=-20（env.scaling=7.0 非依存）"
    );
}

/// Breed の出生位置（Java L83-90: round(BornX/Y * scaling) を lookRight 分岐）は
/// マスコットの set scale で決まり、env.scaling には依存しない。
/// set scale 2.0・env.scaling 7.0・lookRight=true・BornX=16・BornY=32・
/// anchor(1000,500) → (1000 - round(32), 500 + round(64)) = (968, 564)。
#[test]
fn breed_born_position_scales_with_mascot_set_scale() {
    let mut env = SynthEnv::new();
    env.scaling_value = 7.0; // 非依存 pin
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut m = mascot_with_scale((1000, 500), &[("p.png", 128, 128)], 2.0);
    m.set_look_right(true);
    let table = single_table("X", 1);

    let action = create(
        ActionKind::Breed,
        &attrs(&[("BornX", "16"), ("BornY", "32")]),
        vec![anim(None, false, vec![pose("p.png", (64, 64), (0, 0), 2)])],
        1.0,
    );
    set_action(&mut m, &env, "X", action, &mut rng).unwrap();
    let mut factory = FnFactory::constant(make_idle_fallback);
    m.tick(&env, &table, &mut factory, &mut rng); // time 0（最終フレームでない）
    m.tick(&env, &table, &mut factory, &mut rng); // time 1 == duration-1 → breed
    let spawns = env.spawns.borrow();
    assert_eq!(spawns.len(), 1, "penultimate frame で 1 体 spawn");
    assert_eq!(
        spawns[0].anchor,
        (968, 564),
        "BornX/Y を round(*2.0)（env.scaling=7.0 非依存）"
    );
}
