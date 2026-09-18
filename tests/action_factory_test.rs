//! タスク #10b-1: `XmlBehaviorFactory`（XML 資産駆動 BehaviorFactory・#10 構築経由
//! フィルタ）と `load_materials` の disabled_animations 記録の契約テスト（RED）。
//!
//! 背景: check_references の欠落参照は「警告のみ = 実無効化未適用」（reload.rs
//! L84-95 現状）。#10 で「disabled (action 名, animation_index) の Animation を
//! 除去済み [`ActionsConfig`] として構築時に 1 回だけ保持し、既存
//! build_action / build_def 経由で組み立てる [`XmlBehaviorFactory`]」を実装する。
//!
//! pin する API 契約（coder への指示・シグネチャは tests が固定する）:
//!
//! ```text
//! // ---- src/mascot/action/factory.rs（新規）----
//! pub struct XmlBehaviorFactory { ... }
//! impl XmlBehaviorFactory {
//!     // disabled に該当する (action 名, animation_index) の Animation を
//!     // 除去済み ActionsConfig として内部保持する（フィルタは構築時 1 回・
//!     // build 毎に行わない）。構築は既存 build_action / build_def 経由
//!     pub fn new(actions: ActionsConfig, scale: f64,
//!         disabled: &[(String, usize)]) -> XmlBehaviorFactory
//! }
//! impl BehaviorFactory for XmlBehaviorFactory {
//!     // Ref child: 既存 build_action(actions, name, ref_attrs, scale) と同水準の
//!     //   観察可能挙動（Ref 側 attrs 優先マージ込み）
//!     // Inline child: 既存 build_def(actions, def, 空パラメータ, scale) と同水準
//! }
//! // ---- src/app/reload.rs ----
//! pub struct ReloadMaterial {  // 既存 3 フィールドは据え置き・下記を追加
//!     pub disabled_animations: Vec<DisabledAnimation>,
//! }
//! // load_materials は set 毎に check_references(...).disabled を ReloadMaterial に
//! // 記録する（warnings は現状維持の warn ログのみ・既存挙動を変えない）
//! ```
//!
//! 実装（factory.rs 未存在・ReloadMaterial.disabled_animations 未追加）のため
//! cargo test は compile error = RED が正常（E0432: 未存在 item / E0609: 未存在
//! フィールド）。既存ターゲット（303 テスト）は維持される。
//!
//! disabled の観察経路（tester 判断）: AnimateAction は「条件一致する最初の
//! アニメ」を適用する（Base::get_animation）ため、アニメ #0 を disabled にすると
//! 次のアニメ #1 の Pose が適用される。set_image への反映（image().image_ref）と
//! anchor 移動（velocity）の両方で観察する（image_set には対応フレームを供給）。

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use shimeji::app::reload::load_materials;
use shimeji::config::script::{EvalContext, Variable};
use shimeji::config::{
    ActionDef, ActionsConfig, Animation, BehaviorDef, BehaviorEntry, BehaviorsConfig, Pose,
    SequenceChild, VarMap,
};
use shimeji::mascot::action::factory::XmlBehaviorFactory;
use shimeji::mascot::action::{create, ActionKind};
use shimeji::mascot::behavior::{
    Action, BehaviorError, BehaviorFactory, BehaviorRunner, BehaviorTable,
};
use shimeji::mascot::env::{AreaSlot, AreaState, CursorState};
use shimeji::mascot::{EnvironmentView, Mascot, Rect, Rng};
use shimeji::render::imageset::{Frame, ImageSet};

// =====================================================================
// 合成環境（action_test.rs の SynthEnv 縮小版・自己完結）
// =====================================================================

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

/// Java Area 相当の AreaState（visible = true・delta 0）。
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

/// 単一モニタ構成: work area (0,0,1920,1040) / screen (0,0,1920,1080)。
/// BorderType 省略の Animate / Move と screen() 判定のみ使用するため最小構成。
struct SynthEnv {
    ctx: ProbeCtx,
}

impl SynthEnv {
    fn new() -> SynthEnv {
        SynthEnv { ctx: ProbeCtx }
    }
}

impl EnvironmentView for SynthEnv {
    fn work_area(&self) -> Rect {
        Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
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
        false
    }

    fn eval_context(&self) -> &dyn EvalContext {
        &self.ctx
    }

    fn screen_area(&self) -> AreaState {
        area(0, 0, 1920, 1080)
    }

    fn screens(&self) -> Vec<AreaState> {
        vec![area(0, 0, 1920, 1080)]
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
            AreaSlot::WorkArea(0) => area(0, 0, 1920, 1040),
            AreaSlot::Screen(0) => area(0, 0, 1920, 1080),
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
        area(300, 200, 900, 800)
    }

    fn active_window_id(&self) -> i64 {
        7
    }

    fn move_active_window(&self, _x: i32, _y: i32) {}

    fn cursor(&self) -> CursorState {
        CursorState {
            x: 300,
            y: 200,
            dx: 0,
            dy: 0,
        }
    }

    fn scaling(&self) -> f64 {
        1.0
    }

    fn throwing_allowed(&self) -> bool {
        true
    }

    fn breeding_allowed(&self) -> bool {
        true
    }

    fn transients_enabled(&self) -> bool {
        true
    }

    fn transformation_allowed(&self) -> bool {
        true
    }

    fn queue_spawn(
        &self,
        _image_set_name: &str,
        _anchor: (i32, i32),
        _look_right: bool,
        _behavior_name: &str,
    ) {
    }

    fn queue_spawn_next(&self, _image_set_name: &str, _anchor: (i32, i32), _look_right: bool) {}

    fn restore_windows(&self) {}
}

// =====================================================================
// 合成データヘルパ（action_test.rs 踏襲・自己完結）
// =====================================================================

/// Java Math.random 相当の [0,1) 乱数。キューを順に返し、過剰消費で panic する。
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

/// 閉包ファクトリ（完了遷移先 idle の構築に使う）。
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

fn empty_image_set() -> Arc<ImageSet> {
    Arc::new(ImageSet {
        name: "TestSet".to_string(),
        frames: BTreeMap::new(),
        warnings: Vec::new(),
        scale: 1.0,
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
            Frame::from_rgba(
                *width,
                *height,
                vec![0u8; (*width as usize) * (*height as usize) * 4],
            ),
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
    Mascot::new("TestSet", empty_image_set(), anchor)
}

fn mascot_with(anchor: (i32, i32), frames: &[(&str, u32, u32)]) -> Mascot {
    Mascot::new("TestSet", image_set_with(frames), anchor)
}

fn mascot_with_scale(anchor: (i32, i32), frames: &[(&str, u32, u32)], scale: f64) -> Mascot {
    Mascot::new("TestSet", image_set_with_scale(frames, scale), anchor)
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

fn animate_def(animations: Vec<Animation>) -> ActionDef {
    ActionDef::Animate {
        border: None,
        attrs: VarMap::new(),
        animations,
    }
}

fn ref_child(name: &str) -> SequenceChild {
    SequenceChild::Ref {
        name: name.to_string(),
        attrs: VarMap::new(),
    }
}

fn config_from(entries: Vec<(&str, ActionDef)>) -> ActionsConfig {
    let mut map = BTreeMap::new();
    for (name, def) in entries {
        map.insert(name.to_string(), def);
    }
    ActionsConfig { actions: map }
}

/// app_test.rs の fixture 踏襲: "Walk" = velocity (1,0)・"Stare" = velocity (3,0)。
///（空 image set のため観察は anchor 移動のみ・image は set_image(None) 経路）
fn fixture_walk_stare() -> ActionsConfig {
    config_from(vec![
        (
            "Walk",
            animate_def(vec![anim(
                None,
                false,
                vec![pose("p.png", (0, 0), (1, 0), 30)],
            )]),
        ),
        (
            "Stare",
            animate_def(vec![anim(
                None,
                false,
                vec![pose("s.png", (0, 0), (3, 0), 30)],
            )]),
        ),
    ])
}

/// 2 アニメ × 2 アクション（disabled 観察用）:
/// - "A": #0 a.png velocity (1,0) dur 5 / #1 b.png velocity (0,0) dur 5
/// - "B": #0 b1.png velocity (0,0) dur 5 / #1 b2.png velocity (2,0) dur 5
fn fixture_two_actions() -> ActionsConfig {
    config_from(vec![
        (
            "A",
            animate_def(vec![
                anim(None, false, vec![pose("a.png", (64, 64), (1, 0), 5)]),
                anim(None, false, vec![pose("b.png", (64, 64), (0, 0), 5)]),
            ]),
        ),
        (
            "B",
            animate_def(vec![
                anim(None, false, vec![pose("b1.png", (64, 64), (0, 0), 5)]),
                anim(None, false, vec![pose("b2.png", (64, 64), (2, 0), 5)]),
            ]),
        ),
    ])
}

fn single_table(name: &str) -> BehaviorTable {
    BehaviorTable::new(&BehaviorsConfig {
        entries: vec![BehaviorEntry::Single(BehaviorDef {
            name: name.to_string(),
            frequency: 1,
            hidden: false,
            toggleable: false,
            action: ref_child(name),
            next: None,
        })],
    })
}

/// 完了遷移先の最小 fallback（velocity 0 の Animate・副作用なし）。
fn make_idle_fallback() -> Result<Box<dyn Action>, BehaviorError> {
    create(
        ActionKind::Animate,
        &VarMap::new(),
        vec![anim(None, false, vec![pose("idle.png", (0, 0), (0, 0), 5)])],
        1.0,
    )
}

/// Mascot に構築済み action を装着する（Java Mascot.setBehavior 相当・
/// action_test.rs 踏襲。完了遷移先は idle fallback に固定）。
fn set_action(
    m: &mut Mascot,
    env: &dyn EnvironmentView,
    name: &str,
    action: Result<Box<dyn Action>, BehaviorError>,
    rng: &mut dyn Rng,
) {
    let table = single_table(name);
    m.set_behavior(
        Some(BehaviorRunner::new(name, action.expect("action 構築成功"))),
        env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        rng,
    )
    .expect("set_behavior 成功");
}

/// 1 tick 進めて画像参照を観察するヘルパ。
fn tick_image_ref(m: &mut Mascot, env: &dyn EnvironmentView) -> Option<String> {
    let table = single_table(m.image_set_name());
    let mut rng = FakeRng::repeated(0.5, 16);
    let mut factory = FnFactory::constant(make_idle_fallback);
    m.tick(env, &table, &mut factory, &mut rng);
    m.image().map(|img| img.image_ref.clone())
}

// =====================================================================
// 契約 1: 正常構築（空 disabled・Ref / Inline）
// =====================================================================

/// Ref child から Box<dyn Action> が構築でき、既存 build_action 経由と同水準の
/// 観察可能挙動（velocity による anchor 移動）を持つ。1 factory から複数回
/// 構築できる（&mut self・BehaviorTable 行数分の再使用）。
#[test]
fn factory_builds_ref_child_with_same_motion_as_existing_build_action() {
    let env = SynthEnv::new();
    let mut factory = XmlBehaviorFactory::new(fixture_walk_stare(), &[]);

    // "Walk"（velocity (1,0)）: tick1 で anchor += (1,0)
    let action = factory
        .build_action(&ref_child("Walk"))
        .expect("Ref child から Walk を構築できる");
    let mut m = mascot_at((1000, 500));
    set_action(
        &mut m,
        &env,
        "Walk",
        Ok(action),
        &mut FakeRng::repeated(0.5, 16),
    );
    assert_eq!(
        tick_image_ref(&mut m, &env),
        None,
        "image set は空のため image 無し（anchor 移動で観察）"
    );
    assert_eq!(m.anchor(), (1001, 500), "Walk の velocity (1,0) が効く");

    // 同一 factory から "Stare"（velocity (3,0)）も再構築できる。
    // 画面内（SynthEnv screen 幅 1920）から tick 1 回・off-screen 判定に引っかからない
    // 位置で velocity 適用を観察する
    let action = factory
        .build_action(&ref_child("Stare"))
        .expect("Ref child から Stare を構築できる");
    let mut m = mascot_at((1900, 500));
    set_action(
        &mut m,
        &env,
        "Stare",
        Ok(action),
        &mut FakeRng::repeated(0.5, 16),
    );
    m.tick(
        &env,
        &single_table("Stare"),
        &mut FnFactory::constant(make_idle_fallback),
        &mut FakeRng::repeated(0.5, 16),
    );
    assert_eq!(m.anchor(), (1903, 500), "Stare の velocity (3,0) が効く");
}

/// Inline child（匿名 Action 定義）は既存 build_def 直経由と同水準で構築される
///（BehaviorTable の Inline 行相当・空パラメータ）。
#[test]
fn factory_builds_inline_child_with_same_motion_as_direct_def() {
    let env = SynthEnv::new();
    let mut factory = XmlBehaviorFactory::new(fixture_walk_stare(), &[]);

    let inline = animate_def(vec![anim(
        None,
        false,
        vec![pose("p.png", (0, 0), (2, 0), 30)],
    )]);
    let action = factory
        .build_action(&SequenceChild::Inline(Box::new(inline)))
        .expect("Inline child から構築できる");
    let mut m = mascot_at((1000, 500));
    set_action(
        &mut m,
        &env,
        "Walk",
        Ok(action),
        &mut FakeRng::repeated(0.5, 16),
    );
    // velocity 適用は tick 時（Java 逐語・契約 1 前半 Walk と同一経路）
    m.tick(
        &env,
        &single_table("Walk"),
        &mut FnFactory::constant(make_idle_fallback),
        &mut FakeRng::repeated(0.5, 16),
    );
    assert_eq!(
        m.anchor(),
        (1002, 500),
        "Inline 定義の velocity (2,0) が効く"
    );
}

// =====================================================================
// 契約 2: scale 注入（構築時 scale_pose）
//
// 構築 scale の注入経路は 2 つ: `BehaviorFactory::set_scale(scale)` を直接呼ぶ
// 経路と、`BehaviorTable::build_behavior_direct(name, factory, scale)` が構築前に
// `factory.set_scale(scale)` を適用する経路。`XmlBehaviorFactory::new` は scale
// 引数を取らず初期値 1.0（未注入 = 等倍）で始まる。
// =====================================================================

/// `set_scale(2.0)` 後に構築すると既存 create/scale 経由と同水準でアンカー /
/// velocity の scale 変換が効く（action_test.rs
/// `pose_scale_applied_at_action_construction` 契約踏襲・java_round half-up）。
/// scale=2.0: velocity (2,1)→(4,2)・anchor (32,48)→(64,96)。
#[test]
fn factory_applies_scale_via_set_scale_to_anchor_and_velocity() {
    let env = SynthEnv::new();
    let actions = config_from(vec![(
        "Walk",
        animate_def(vec![anim(
            None,
            false,
            vec![pose("p.png", (32, 48), (2, 1), 5)],
        )]),
    )]);
    let mut factory = XmlBehaviorFactory::new(actions, &[]);
    factory.set_scale(2.0);

    let action = factory
        .build_action(&ref_child("Walk"))
        .expect("scale 付きで構築できる");
    let mut m = mascot_with((1000, 500), &[("p.png", 128, 128)]);
    set_action(
        &mut m,
        &env,
        "Walk",
        Ok(action),
        &mut FakeRng::repeated(0.5, 16),
    );
    // velocity 適用は tick 時（既存正本 action_test.rs
    // `pose_scale_applied_at_action_construction` と同一経路）
    m.tick(
        &env,
        &single_table("Walk"),
        &mut FnFactory::constant(make_idle_fallback),
        &mut FakeRng::repeated(0.5, 16),
    );

    assert_eq!(m.anchor(), (1004, 502), "tick1: velocity (2,1)*2 = (4,2)");
    let img = m.image().expect("フレームは存在する");
    assert_eq!(
        img.center,
        (64, 96),
        "構築時プリスケール: anchor (32,48)*2 = (64,96)"
    );
}

/// `BehaviorTable::build_behavior_direct(name, factory, scale)` は構築前に
/// `factory.set_scale(scale)` を適用する。scale 未注入の factory（初期値 1.0）でも
/// scale 引数で構築物の pose が scale される。マスコットの `scale()`
/// （= 保持 ImageSet.scale）を渡す経路で検証する。
#[test]
fn build_behavior_direct_applies_mascot_scale_to_pose() {
    let env = SynthEnv::new();
    let actions = config_from(vec![(
        "Walk",
        animate_def(vec![anim(
            None,
            false,
            vec![pose("p.png", (32, 48), (2, 1), 5)],
        )]),
    )]);
    // scale 未注入の factory（初期値 1.0）でも build_behavior_direct が set_scale する
    let mut factory = XmlBehaviorFactory::new(actions, &[]);
    let table = single_table("Walk");

    let mut m = mascot_with_scale((1000, 500), &[("p.png", 128, 128)], 2.0);
    assert_eq!(m.scale(), 2.0, "ImageSet.scale を保持");
    let runner = table
        .build_behavior_direct("Walk", &mut factory, &m)
        .expect("scale 引数付きで構築できる");
    m.set_behavior(
        Some(runner),
        &env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut FakeRng::repeated(0.5, 16),
    )
    .expect("set_behavior 成功");

    m.tick(
        &env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut FakeRng::repeated(0.5, 16),
    );
    assert_eq!(m.anchor(), (1004, 502), "velocity (2,1)*2 = (4,2)");
    let img = m.image().expect("フレームは存在する");
    assert_eq!(img.center, (64, 96), "anchor (32,48)*2 = (64,96)");
}

// =====================================================================
// 契約 3+4: disabled 適用（該当アニメ除去）+ 不該当は無影響
// =====================================================================

/// disabled ("A", 0) を指定すると構築物にアニメ #0 が現れない（条件一致する最初の
/// アニメが #1 に切り替わる）。disabled に含まれない ("B", 0) は通常通り構築され、
/// 除去が他 action に波及しない。対照として空 disabled では #0 が選択される。
#[test]
fn factory_excludes_disabled_animation_and_keeps_others_intact() {
    let env = SynthEnv::new();
    let actions = fixture_two_actions();
    let frames = [
        ("a.png", 32, 32),
        ("b.png", 32, 32),
        ("b1.png", 32, 32),
        ("b2.png", 32, 32),
    ];

    // 対照: 空 disabled → A の最初のアニメ #0（a.png・velocity (1,0)）
    let mut clean = XmlBehaviorFactory::new(actions.clone(), &[]);
    let action = clean.build_action(&ref_child("A")).expect("対照構築");
    let mut m = mascot_with((1000, 500), &frames);
    set_action(
        &mut m,
        &env,
        "A",
        Ok(action),
        &mut FakeRng::repeated(0.5, 16),
    );
    assert_eq!(
        tick_image_ref(&mut m, &env).as_deref(),
        Some("a.png"),
        "空 disabled では最初のアニメ #0 が選択される"
    );
    assert_eq!(m.anchor(), (1001, 500), "対照: #0 の velocity (1,0)");

    // 本命: ("A", 0) を disabled → #0 が除去され #1（b.png・velocity (0,0)）が適用
    let mut filtered = XmlBehaviorFactory::new(actions, &[("A".to_string(), 0)]);
    let action = filtered
        .build_action(&ref_child("A"))
        .expect("disabled 付き構築");
    let mut m = mascot_with((1000, 500), &frames);
    set_action(
        &mut m,
        &env,
        "A",
        Ok(action),
        &mut FakeRng::repeated(0.5, 16),
    );
    assert_eq!(
        tick_image_ref(&mut m, &env).as_deref(),
        Some("b.png"),
        "disabled #0 は構築物に現れず、次のアニメ #1 が適用される"
    );
    assert_eq!(m.anchor(), (1000, 500), "残った #1 の velocity (0,0)");

    // 不該当 ("B", 0) は無影響: B の最初のアニメ #0（b1.png）が残る
    let action = filtered.build_action(&ref_child("B")).expect("B 構築");
    let mut m = mascot_with((1000, 500), &frames);
    set_action(
        &mut m,
        &env,
        "B",
        Ok(action),
        &mut FakeRng::repeated(0.5, 16),
    );
    assert_eq!(
        tick_image_ref(&mut m, &env).as_deref(),
        Some("b1.png"),
        "disabled に含まれない action は除去されない（除外が広すぎない）"
    );
    assert_eq!(m.anchor(), (1000, 500), "B #0 の velocity (0,0)");
}

// =====================================================================
// 契約 3 補助: フィルタは構築済み ActionsConfig 経由のため Sequence の
// Ref 子（actions 内参照）にも効く
// =====================================================================

/// "Seq"（Sequence・Ref 子は "A"）を構築すると、参照先 "A" の disabled #0 が
/// 除去された状態で子が組み立てられる（構築時 1 回フィルタ・build 毎ではない）。
#[test]
fn factory_filter_propagates_through_sequence_ref_children() {
    let env = SynthEnv::new();
    let actions = config_from(vec![
        (
            "Seq",
            ActionDef::Sequence {
                border: None,
                attrs: VarMap::new(),
                is_loop: false,
                animations: Vec::new(),
                children: vec![ref_child("A")],
            },
        ),
        (
            "A",
            animate_def(vec![
                anim(None, false, vec![pose("a.png", (64, 64), (1, 0), 5)]),
                anim(None, false, vec![pose("b.png", (64, 64), (0, 0), 5)]),
            ]),
        ),
    ]);
    let mut factory = XmlBehaviorFactory::new(actions, &[("A".to_string(), 0)]);

    let action = factory
        .build_action(&ref_child("Seq"))
        .expect("Sequence（Ref 子）から構築できる");
    let mut m = mascot_with((1000, 500), &[("a.png", 32, 32), ("b.png", 32, 32)]);
    set_action(
        &mut m,
        &env,
        "Seq",
        Ok(action),
        &mut FakeRng::repeated(0.5, 16),
    );
    assert_eq!(
        tick_image_ref(&mut m, &env).as_deref(),
        Some("b.png"),
        "Ref 子の参照先定義からも disabled #0 が除去されている"
    );
}

// =====================================================================
// 契約 6 (#32): per-set 定義集合の選択（from_sets + set_image_set）
// =====================================================================

/// `Stand` の参照画像だけが違う 2 set 分の定義集合フィクスチャ。
fn per_set_actions() -> [Arc<ActionsConfig>; 2] {
    [
        Arc::new(config_from(vec![(
            "Stand",
            animate_def(vec![anim(
                None,
                false,
                vec![pose("a.png", (32, 48), (1, 0), 30)],
            )]),
        )])),
        Arc::new(config_from(vec![(
            "Stand",
            animate_def(vec![anim(
                None,
                false,
                vec![pose("b.png", (32, 48), (2, 0), 30)],
            )]),
        )])),
    ]
}

/// 同名 Action でも set ごとに内容が違う資産（デレマスしめじ v1.9 の `Stand` は
/// set ごとに別画像を参照する）では、`from_sets` + `set_image_set` が set 名で
/// 定義集合を選ぶ。スコープ未設定 / 未知 set は先頭 set（= 既定 set）へ落ちる。
#[test]
fn from_sets_selects_definition_set_by_image_set() {
    let env = SynthEnv::new();
    let [set_a, set_b] = per_set_actions();
    let mut factory =
        XmlBehaviorFactory::from_sets([("SetA".to_string(), set_a), ("SetB".to_string(), set_b)]);

    // スコープ未設定 → 先頭 set（既定 set）
    let action = factory
        .build_action(&ref_child("Stand"))
        .expect("既定 set から構築できる");
    let mut m = Mascot::new("SetA", image_set_with(&[("a.png", 32, 48)]), (1000, 500));
    set_action(
        &mut m,
        &env,
        "Stand",
        Ok(action),
        &mut FakeRng::repeated(0.5, 16),
    );
    assert_eq!(
        tick_image_ref(&mut m, &env).as_deref(),
        Some("a.png"),
        "スコープ未設定は先頭 set の定義"
    );

    // SetB を指定 → b.png / velocity (2,0)
    factory.set_image_set("SetB");
    let action = factory
        .build_action(&ref_child("Stand"))
        .expect("SetB から構築できる");
    let mut m = Mascot::new("SetB", image_set_with(&[("b.png", 32, 48)]), (1000, 500));
    set_action(
        &mut m,
        &env,
        "Stand",
        Ok(action),
        &mut FakeRng::repeated(0.5, 16),
    );
    assert_eq!(
        tick_image_ref(&mut m, &env).as_deref(),
        Some("b.png"),
        "指定 set の定義が選ばれる"
    );
    assert_eq!(
        m.anchor(),
        (1002, 500),
        "SetB の velocity (2,0) が適用される"
    );

    // 未知 set → 既定集合（先頭 set）へフォールバック
    factory.set_image_set("NoSuchSet");
    let action = factory
        .build_action(&ref_child("Stand"))
        .expect("未知 set は既定集合で構築できる");
    let mut m = Mascot::new(
        "NoSuchSet",
        image_set_with(&[("a.png", 32, 48)]),
        (1000, 500),
    );
    set_action(
        &mut m,
        &env,
        "Stand",
        Ok(action),
        &mut FakeRng::repeated(0.5, 16),
    );
    assert_eq!(
        tick_image_ref(&mut m, &env).as_deref(),
        Some("a.png"),
        "未知 set は既定集合へフォールバックする"
    );
}

/// 構築の唯一のファネル `build_behavior_direct` がマスコットの image set を
/// ファクトリへ注入する（= set ごとの定義集合が呼び出し側の指定なしで選ばれる）。
#[test]
fn build_behavior_direct_injects_mascot_image_set_into_factory() {
    let env = SynthEnv::new();
    let [set_a, set_b] = per_set_actions();
    let mut factory =
        XmlBehaviorFactory::from_sets([("SetA".to_string(), set_a), ("SetB".to_string(), set_b)]);
    let table = single_table("Stand");
    let mut rng = FakeRng::repeated(0.5, 16);
    // a.png も持たせて「SetB の定義（b.png）が選ばれた」ことを区別可能にする
    let mut m = Mascot::new(
        "SetB",
        image_set_with(&[("a.png", 32, 48), ("b.png", 32, 48)]),
        (1000, 500),
    );

    let runner = table
        .build_behavior_direct("Stand", &mut factory, &m)
        .expect("SetB の定義で構築できる");
    m.set_behavior(
        Some(runner),
        &env,
        &table,
        &mut FnFactory::constant(make_idle_fallback),
        &mut rng,
    )
    .expect("set_behavior 成功");
    let mut fallback = FnFactory::constant(make_idle_fallback);
    m.tick(&env, &table, &mut fallback, &mut rng);
    assert_eq!(
        m.image().map(|img| img.image_ref.clone()).as_deref(),
        Some("b.png"),
        "マスコットの image set (SetB) の定義が選ばれる"
    );
}

// =====================================================================
// 契約 5: load_materials が check_references の disabled を ReloadMaterial へ記録
// =====================================================================

const ACTIONS_WITH_MISSING_REF_XML: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n",
    "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
    "  <ActionList>\n",
    "    <Action Name=\"Walk\" Type=\"Stay\">\n",
    "      <Animation>\n",
    "        <Pose Image=\"/walk1.png\" ImageAnchor=\"0,0\" Velocity=\"0,0\" Duration=\"1\"/>\n",
    "      </Animation>\n",
    "      <Animation>\n",
    "        <Pose Image=\"/missing.png\" ImageAnchor=\"0,0\" Velocity=\"0,0\" Duration=\"1\"/>\n",
    "      </Animation>\n",
    "    </Action>\n",
    "  </ActionList>\n",
    "</Mascot>\n"
);

const BEHAVIORS_XML: &str = concat!(
    "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
    "  <BehaviorList>\n",
    "    <Behavior Name=\"ChaseMouse\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Fall\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Dragged\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Thrown\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Walk\" Frequency=\"1\"/>\n",
    "  </BehaviorList>\n",
    "</Mascot>\n"
);

/// conf/ + img/ を持つ使い捨てディレクトリ（Drop で再帰削除・app_reload_test.rs 踏襲）。
struct TempAssets {
    root: PathBuf,
}

impl TempAssets {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("shimeji_t10b_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // 前回残留の掃除
        std::fs::create_dir_all(root.join("conf")).expect("conf ディレクトリを作れる");
        std::fs::create_dir_all(root.join("img")).expect("img ディレクトリを作れる");
        TempAssets { root }
    }

    fn conf_dir(&self) -> PathBuf {
        self.root.join("conf")
    }

    fn img_dir(&self) -> PathBuf {
        self.root.join("img")
    }

    fn write_conf(&self, name: &str, content: &str) {
        std::fs::write(self.conf_dir().join(name), content).expect("conf ファイルを書ける");
    }

    /// 単色 PNG を書き込む（img/<set>/<file>）。
    fn write_png(&self, set: &str, file: &str, width: u32, height: u32) {
        let dir = self.img_dir().join(set);
        std::fs::create_dir_all(&dir).expect("set ディレクトリを作れる");
        image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]))
            .save(dir.join(file))
            .expect("テンポラリ PNG を書ける");
    }
}

impl Drop for TempAssets {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// 欠落画像参照（/missing.png）を含む fixture で、load_materials が
/// ReloadMaterial.disabled_animations に該当 (action, animation_index) を記録する
///（warnings の warn ログは既存挙動維持・ここでは記録を観察する）。
#[test]
fn load_materials_records_disabled_animations_from_check_references() {
    let assets = TempAssets::new("disabled");
    assets.write_conf("actions.xml", ACTIONS_WITH_MISSING_REF_XML);
    assets.write_conf("behaviors.xml", BEHAVIORS_XML);
    // 第 2 アニメの参照 missing.png は無い → アニメ丸ごと無効化対象
    assets.write_png("Shimeji", "walk1.png", 32, 24);

    let materials = load_materials(&assets.conf_dir(), &assets.img_dir(), &HashMap::new())
        .expect("有効な conf + set でロードできる");
    assert_eq!(materials.len(), 1, "set 1 件が material 化される");

    let disabled = &materials[0].disabled_animations;
    assert_eq!(disabled.len(), 1, "欠落参照アニメ 1 件が記録される");
    assert_eq!(disabled[0].action, "Walk", "該当 action 名");
    assert_eq!(disabled[0].animation_index, 1, "第 2 アニメ（0 始まり）");
}
