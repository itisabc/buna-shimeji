//! タスク #13: MoveAction の TargetX/TargetY 変数化（ComplexMove putVariable 相当）契約テスト（RED）。
//!
//! 背景（確定済み設計）:
//! - 資産 ClimbWall（Type="Move" BorderType="Wall"）は TargetY 条件付きアニメ 2 本で
//!   登り/降りを切り替える。Java 正本では ComplexMove のみが putVariable し
//!   （.tmp/java-ref/action/ComplexMove.java L85-86 / L145-146）、Move は供給しない
//!   （= 本家でも TargetY 条件付き Move は評価エラーになる同一問題）。
//! - ユーザー決定の修正方針 (b): **MoveAction にも TargetX/TargetY 変数化を適用**
//!   （Java 逐語原則からの意図的差異・ComplexMove putVariable 相当）。
//!
//! pin する契約（公開インターフェースのみ・実装詳細に依存しない）:
//! 1. TargetY 属性付き Move + 条件アニメ 2 本（真側 `#{TargetY < mascot.anchor.y}` /
//!    偽側その否定）で、開始 anchor と TargetY の大小に応じ**正しい側のアニメが
//!    選択され画像へ反映される**（init 後 tick1 で image 観察まで）
//! 2. **注入値 = 属性式の評価値**（TargetY="#{300 + 100}" → 400 として条件評価される）
//! 3. **tick 跨ぎ**: `#{}` 条件は毎フレーム再評価されるため、複数 tick でも
//!    アニメ選択が継続して成立する（注入値の永続性）
//! 4. **属性無し Move は既存挙動不変**（TargetX/Y 属性なし・条件なしアニメ =
//!    従来どおり選択・適用され、変数化追加で破壊されない）
//!
//! 注（#34・design §1.10 (z-8)）: 上の契約 4 は「TargetX/Y なし + Duration なし」の
//! 即終了（Java 同一）を指す。TargetX/Y なしでも `Duration` を明示した場合は
//! Duration まで継続する（契約 5）。
//!
//! RED 想定（実測確認済み）: 現行実装は TargetX/TargetY を変数へ供給しないため、
//! アニメ条件式の識別子解決が失敗し next() が Err を返す
//! （move_tick → get_turning_animation → animation_is_effective のエラー伝播。
//! fallback 選択ではない）。したがって RED 証跡は「tick 成功を要求する assert 失敗」。
//!
//! 流儀: tests/action_test.rs の SynthEnv fixture を自己完結コピー（規約）+ 同ファイル
//! DraggedAction テスト（L1081-1135）と同じ create() → Box<dyn Action> 直接
//! init/next スタイル（BehaviorRunner を経由しない既存流儀）。
//! hotspot 経路（refresh_hotspots の TargetY 解決）は #11 で pin 済みのため本ファイルでは
//! 検証しない（#13 の本質は変数提供）。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use shimeji::config::script::{EvalContext, Variable};
use shimeji::config::{Animation, Pose, VarMap};
use shimeji::mascot::action::{create, ActionKind};
use shimeji::mascot::behavior::ActionError;
use shimeji::mascot::env::{AreaSlot, AreaState, CursorState};
use shimeji::mascot::{EnvironmentView, Mascot, Rect, Rng};
use shimeji::render::imageset::{Frame, ImageSet};

// =====================================================================
// 合成モニタ状態の test-double（action_test.rs SynthEnv の自己完結コピー）
// =====================================================================

/// Java Area 相当の AreaState（visible=true）。
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

    fn move_active_window(&self, _x: i32, _y: i32) {
        // 本ファイルの契約（Move の変数注入）では activeIE 移動を観察しない
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
        _image_set_name: &str,
        _anchor: (i32, i32),
        _look_right: bool,
        _behavior_name: &str,
    ) {
        // 本ファイルの契約では spawn を観察しない（Breed 不使用）
    }
}

// =====================================================================
// 合成データヘルパ（自己完結）
// =====================================================================

/// Java Math.random 相当の [0,1) 乱数。本ファイルの式は乱数不使用のため
/// どの値を返しても等価（消費 0 回）。
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

fn image_set_with(frames: &[(&str, u32, u32)]) -> Arc<ImageSet> {
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
        scale: 1.0,
    })
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

/// image() を image_ref へ正規化して比較するヘルパ。
fn image_ref(m: &Mascot) -> Option<String> {
    m.image().map(|i| i.image_ref.clone())
}

// =====================================================================
// 契約 1: TargetY 属性 + 条件アニメ 2 本の選択（ComplexMove.java L145-146 相当）
// =====================================================================

/// ClimbWall 流儀: TargetY 属性付き Move が、開始 anchor と TargetY の大小に応じ
/// 正しい側のアニメを選択して画像へ反映する（init 後 tick1 で image 観察まで）。
///
/// - 登り側: anchor.y(800) > TargetY(400) → `TargetY < mascot.anchor.y` = true
/// - 降り側: anchor.y(200) < TargetY(400) → 同式 = false（否定側アニメ）
///
/// velocity 0 により anchor は不変（overshoot クランプ非発火）→ 選択の原因は
/// TargetY 注入値と anchor の比較一択に絞られる。
#[test]
fn move_target_y_selects_conditional_animation_by_relative_position() {
    let env = SynthEnv::new();
    let climb_frames = [("up.png", 128, 128), ("down.png", 128, 128)];
    // 真側: `#{TargetY < mascot.anchor.y}` / 偽側: その否定
    let anims = || {
        vec![
            anim(
                Some(var("#{TargetY < mascot.anchor.y}")),
                false,
                vec![pose("up.png", (64, 64), (0, 0), 5)],
            ),
            anim(
                Some(var("#{!(TargetY < mascot.anchor.y)}")),
                false,
                vec![pose("down.png", (64, 64), (0, 0), 5)],
            ),
        ]
    };

    // ---- 登り側（TargetY が上 = y 小） ----
    let mut rng = FakeRng::repeated(0.5, 8);
    let mut m = Mascot::new("TestSet", image_set_with(&climb_frames), (100, 800));
    let mut action = create(
        ActionKind::Move,
        &attrs(&[("TargetY", "400")]),
        anims(),
        1.0,
    )
    .unwrap();
    action
        .init(&mut m, &env, &mut rng)
        .expect("init は成功する");
    let outcome = action.next(&mut m, &env, &mut rng);
    assert!(
        outcome.is_ok(),
        "tick1 は成功するはず（TargetY 注入によりアニメ条件が評価可能）: Err={:?}",
        outcome.err()
    );
    assert_eq!(
        image_ref(&m),
        Some("up.png".to_string()),
        "TargetY(400) < anchor.y(800) → 登り側アニメが選択される"
    );
    assert_eq!(m.anchor(), (100, 800), "velocity 0 で anchor 不変");

    // ---- 降り側（TargetY が下 = y 大） ----
    let mut rng = FakeRng::repeated(0.5, 8);
    let mut m = Mascot::new("TestSet", image_set_with(&climb_frames), (100, 200));
    let mut action = create(
        ActionKind::Move,
        &attrs(&[("TargetY", "400")]),
        anims(),
        1.0,
    )
    .unwrap();
    action
        .init(&mut m, &env, &mut rng)
        .expect("init は成功する");
    let outcome = action.next(&mut m, &env, &mut rng);
    assert!(
        outcome.is_ok(),
        "tick1 は成功するはず（TargetY 注入によりアニメ条件が評価可能）: Err={:?}",
        outcome.err()
    );
    assert_eq!(
        image_ref(&m),
        Some("down.png".to_string()),
        "TargetY(400) >= anchor.y(200) → 降り側アニメが選択される"
    );
    assert_eq!(m.anchor(), (100, 200), "velocity 0 で anchor 不変");
}

// =====================================================================
// 契約 2: 注入値 = 属性式の評価値
// =====================================================================

/// 注入されるのは属性の**評価値**であることの pin:
/// TargetY 属性を式 `#{300 + 100}`（= 400）で、TargetX 属性を定数 600 で与え、
/// 両方の注入値を 1 本のアニメ条件 `TargetX == 600 && TargetY == 400` で要求する。
/// 評価値でない値（生ソース・DEFAULT・0 等）が注入された場合は偽側アニメに落ちる。
#[test]
fn move_target_injected_value_is_attribute_expression_evaluation() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 8);
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("both.png", 128, 128), ("other.png", 128, 128)]),
        (100, 800),
    );

    let mut action = create(
        ActionKind::Move,
        &attrs(&[("TargetX", "600"), ("TargetY", "#{300 + 100}")]),
        vec![
            anim(
                Some(var("#{TargetX == 600 && TargetY == 400}")),
                false,
                vec![pose("both.png", (64, 64), (0, 0), 5)],
            ),
            anim(
                Some(var("#{TargetX != 600 || TargetY != 400}")),
                false,
                vec![pose("other.png", (64, 64), (0, 0), 5)],
            ),
        ],
        1.0,
    )
    .unwrap();
    action
        .init(&mut m, &env, &mut rng)
        .expect("init は成功する");
    let outcome = action.next(&mut m, &env, &mut rng);
    assert!(
        outcome.is_ok(),
        "tick1 は成功するはず（TargetX/TargetY 注入によりアニメ条件が評価可能）: Err={:?}",
        outcome.err()
    );
    assert_eq!(
        image_ref(&m),
        Some("both.png".to_string()),
        "注入値は属性式の評価値（TargetX=600・TargetY=#{{300+100}} の評価結果 400）"
    );
}

// =====================================================================
// 契約 3: tick 跨ぎ（注入値の永続性）
// =====================================================================

/// `#{}` 条件は毎フレーム再評価される（Variables::reset_values）ため、
/// 複数 tick にわたってアニメ選択が継続して成立することを pin する
/// （注入値が持続しない実装・初回 tick 限りの注入では 2 tick 目以降に崩れる）。
/// velocity 0 で anchor 不変 → 3 tick すべて同一側が選択され続ける。
#[test]
fn move_target_injection_persists_across_ticks() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 8);
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("up.png", 128, 128), ("down.png", 128, 128)]),
        (100, 800),
    );

    let mut action = create(
        ActionKind::Move,
        &attrs(&[("TargetY", "400")]),
        vec![
            anim(
                Some(var("#{TargetY < mascot.anchor.y}")),
                false,
                vec![pose("up.png", (64, 64), (0, 0), 5)],
            ),
            anim(
                Some(var("#{!(TargetY < mascot.anchor.y)}")),
                false,
                vec![pose("down.png", (64, 64), (0, 0), 5)],
            ),
        ],
        1.0,
    )
    .unwrap();
    action
        .init(&mut m, &env, &mut rng)
        .expect("init は成功する");

    for tick in 1..=3 {
        let outcome = action.next(&mut m, &env, &mut rng);
        assert!(
            outcome.is_ok(),
            "tick{} は成功するはず（注入値が tick を跨いで持続）: Err={:?}",
            tick,
            outcome.err()
        );
        assert_eq!(
            image_ref(&m),
            Some("up.png".to_string()),
            "tick{}: 登り側アニメ選択が継続する",
            tick
        );
    }
}

// =====================================================================
// 契約 4: 属性無し Move は既存挙動不変（回帰ガード）
// =====================================================================

/// TargetX/TargetY 属性なし・条件なしアニメ 1 本の Move は、変数化追加の前後で
/// 挙動が変わらない（既存 fallback 選択のまま・tick が Err にならない・
/// velocity が通常どおり適用される）。変数化の inject 追加が無属性経路を
/// 破壊しないことの回帰ガード。
#[test]
fn move_without_target_attributes_keeps_existing_behavior() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 8);
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("walk.png", 128, 128)]),
        (100, 500),
    );

    let mut action = create(
        ActionKind::Move,
        &attrs(&[]),
        vec![anim(
            None,
            false,
            vec![pose("walk.png", (64, 64), (2, 0), 30)],
        )],
        1.0,
    )
    .unwrap();
    action
        .init(&mut m, &env, &mut rng)
        .expect("init は成功する");

    for tick in 1..=2 {
        let outcome = action.next(&mut m, &env, &mut rng);
        assert!(
            outcome.is_ok(),
            "tick{}: 属性無し Move は既存どおり Err にならない: Err={:?}",
            tick,
            outcome.err()
        );
        assert_eq!(
            image_ref(&m),
            Some("walk.png".to_string()),
            "tick{}: 条件なしアニメが既存どおり選択される",
            tick
        );
        assert_eq!(
            m.anchor(),
            (100 + 2 * tick, 500),
            "tick{}: velocity は既存どおり適用される（look_right 既定 false → +2）",
            tick
        );
    }
}

// =====================================================================
// 契約 5: target 無し Move の終了条件（design §1.10 (z-8)）
// =====================================================================

/// `Duration` 明示の target 無し Move は継続する（Java の即終了からの意図的差異）。
#[test]
fn move_without_target_with_duration_has_next_true() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 8);
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("walk.png", 128, 128)]),
        (100, 500),
    );
    let mut action = create(
        ActionKind::Move,
        &attrs(&[("Duration", "3")]),
        vec![anim(
            None,
            false,
            vec![pose("walk.png", (64, 64), (2, 0), 30)],
        )],
        1.0,
    )
    .unwrap();
    action
        .init(&mut m, &env, &mut rng)
        .expect("init は成功する");
    assert!(
        action
            .has_next(&mut m, &env, &mut rng)
            .expect("has_next は成功する"),
        "Duration 明示の target 無し Move は time 0 で継続する"
    );
}

/// `Duration` 未指定の target 無し Move は Java と同一（即終了）を保つ。
#[test]
fn move_without_target_without_duration_completes_immediately() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 8);
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("walk.png", 128, 128)]),
        (100, 500),
    );
    let mut action = create(
        ActionKind::Move,
        &attrs(&[]),
        vec![anim(
            None,
            false,
            vec![pose("walk.png", (64, 64), (2, 0), 30)],
        )],
        1.0,
    )
    .unwrap();
    action
        .init(&mut m, &env, &mut rng)
        .expect("init は成功する");
    assert!(
        !action
            .has_next(&mut m, &env, &mut rng)
            .expect("has_next は成功する"),
        "Duration 未指定は Java 同様の即終了（新規挙動ゼロ）"
    );
}

/// 継続する target 無し Move は毎 tick 境界検査を行うため、床外では LostGround に
/// なる（BorderedAction 共通の既存挙動・design §1.10 (z-8) red-team R4）。
#[test]
fn move_without_target_with_duration_outside_border_returns_lost_ground() {
    let env = SynthEnv::new();
    let mut rng = FakeRng::repeated(0.5, 8);
    // 床 (bottom=1040) の外
    let mut m = Mascot::new(
        "TestSet",
        image_set_with(&[("walk.png", 128, 128)]),
        (100, 500),
    );
    let mut action = create(
        ActionKind::Move,
        &attrs(&[("Duration", "3"), ("BorderType", "Floor")]),
        vec![anim(
            None,
            false,
            vec![pose("walk.png", (64, 64), (2, 0), 30)],
        )],
        1.0,
    )
    .unwrap();
    action
        .init(&mut m, &env, &mut rng)
        .expect("init は成功する");
    assert!(action
        .has_next(&mut m, &env, &mut rng)
        .expect("has_next は成功する"));
    assert!(
        matches!(
            action.next(&mut m, &env, &mut rng),
            Err(ActionError::LostGround)
        ),
        "床外で継続すると境界検査で LostGround になる"
    );
}
