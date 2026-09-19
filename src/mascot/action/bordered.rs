//! BorderedAction 系（design §1.8・Java 正本 .tmp/java-ref/action/）。
//!
//! Java `BorderedAction`（境界に付着して動くアクションの基底クラス）を
//! 「共通構造 [`Bordered`] + 境界トークン [`env::BorderRef`]」で再現する
//! （design §1.7(b)・トークン方式）:
//! - initで BorderType 属性を評価し解決して保持（Java BorderedAction.java L40-48 逐語。
//!   省略 / 未知値は border 無効 = None・L24 DEFAULT_BORDERTYPE = null）
//! - tick 毎に fresh snapshot で `border.move(anchor)`（L52-57 逐語）
//! - `getBorder() != null && !isOn(anchor)` → LostGround
//!   （Animate.java L34-36 / Move.java L60-62 / Stay.java L29-31 逐語。
//!   **BorderType 明示時は NotOnBorder も「border 非 null」** なので床外では
//!   LostGround になる・Java 同一）
//!
//! 収録 Java クラス: Animate / Stay / Move / ThrowIE / WalkWithIE / FallWithIE / Breed
//! （ThrowIE = Animate 派生・WalkWithIE = Move 派生・FallWithIE = Fall 派生・
//! Breed = Animate 派生。Delegate（Java Breed 内部クラス）は BreedAction 内に実装）

use super::super::animation::{
    animation_duration, animation_is_effective, animation_pose_at, apply_pose,
};
use super::super::behavior::ActionError;
use super::super::env::{self, BorderRef};
use super::super::{
    AffordanceArrival, EnvironmentView, Mascot, MascotContext, Rng, TransformRequest,
};
use super::{Base, FallAction};
use crate::config::script::EvalError;
use crate::mascot::behavior::Action;
use crate::render::imageset::java_round;

// =====================================================================
// BorderedAction 共通部分（Java BorderedAction.java L20-70 相当）
// =====================================================================

/// Java BorderedAction 相当の共通部分。
pub(crate) struct Bordered {
    pub base: Base,
    /// Java `BorderedAction.border`:
    /// - `None` = BorderType 未指定 / 未知値（Java border = null・border_move /
    ///   LostGround 検査はスキップ）
    /// - `Some(None)` = BorderType 明示だがアンカー上に境界が無い
    ///   （Java border = NotOnBorder.INSTANCE・isOn は常に false）
    /// - `Some(Some(rf))` = 解決済みトークン（tick 毎に fresh 評価）
    pub border: Option<Option<BorderRef>>,
}

impl Bordered {
    pub(crate) fn new(attrs: crate::config::VarMap, animations: Vec<super::Animation>) -> Bordered {
        Bordered {
            base: Base::new(attrs, animations),
            border: None,
        }
    }

    /// Java BorderedAction.init L37-49 逐語: BorderType 属性を評価し
    /// getCeiling()/getWall()/getFloor()（anchor / lookRight 基準・
    /// ignoreSeparator = false）の結果を保持する。
    /// 未指定 / 未知値は setBorder しない（border = null 相当）。
    pub(crate) fn init_common(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<(), ActionError> {
        self.base.init(mascot);
        self.border = match self.base.text_attr("BorderType").as_deref() {
            Some("Ceiling") => Some(super::super::env::resolve_border(
                env,
                env::BorderKind::Ceiling,
                mascot.anchor(),
                mascot.look_right(),
                false,
            )),
            Some("Wall") => Some(env::resolve_border(
                env,
                env::BorderKind::Wall,
                mascot.anchor(),
                mascot.look_right(),
                false,
            )),
            Some("Floor") => Some(env::resolve_border(
                env,
                env::BorderKind::Floor,
                mascot.anchor(),
                mascot.look_right(),
                false,
            )),
            // 未指定 / 未知値（Java L52-57: border == null なら border.move スキップ）
            _ => None,
        };
        Ok(())
    }

    /// Java BorderedAction.tick L52-57 逐語: border 非 null →
    /// `anchor.setLocation(border.move(anchor))`（fresh snapshot 評価）。
    pub(crate) fn border_tick(&mut self, mascot: &mut Mascot, env: &dyn EnvironmentView) {
        if let Some(border) = self.border {
            let (x, y) = mascot.anchor();
            mascot.set_anchor(env::border_move(env, border, (x, y)));
        }
    }

    /// Java Animate/Move/Stay.tick の境界外 LostGround 逐語:
    /// `getBorder() != null && !getBorder().isOn(anchor) → LostGroundException`。
    pub(crate) fn check_on_border(
        &self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<(), ActionError> {
        if let Some(border) = self.border {
            if !env::border_is_on(env, border, mascot.anchor()) {
                return Err(ActionError::LostGround);
            }
        }
        Ok(())
    }
}

// =====================================================================
// Animate（Java Animate.java L18-41 相当）
// =====================================================================

pub(crate) struct AnimateAction {
    pub bordered: Bordered,
}

impl AnimateAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> AnimateAction {
        AnimateAction {
            bordered: Bordered::new(attrs, animations),
        }
    }
}

/// Animate / ThrowIE / Breed が共有する Animate.tick 本体
/// （Java Animate.java L31-40 逐語: super.tick()（border move）→ 境界外
/// LostGround → アニメ適用）。
pub(crate) fn animate_tick(
    bordered: &mut Bordered,
    mascot: &mut Mascot,
    env: &dyn EnvironmentView,
) -> Result<(), ActionError> {
    bordered.border_tick(mascot, env);
    bordered.check_on_border(mascot, env)?;
    bordered.base.apply_effective_animation(mascot, env)
}

impl Action for AnimateAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.init_common(mascot, env)
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L27: super.hasNext() && getTime() < getAnimation().getDuration()
        Ok(self.bordered.base.base_has_next(mascot, env)?
            && self.bordered.base.get_time(mascot)
                < self.bordered.base.tolerant_anim_duration(mascot, env)?)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.base.next_pre(mascot, env)?;
        animate_tick(&mut self.bordered, mascot, env)
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.bordered.base.draggable(mascot, env)
    }
}

// =====================================================================
// Transform（Java Transform.java L19-62 相当・Animate 派生）
// =====================================================================

/// Java 定数は UK 綴り `TransformBehaviour` だが実 XML（デレマスしめじ v1.9）は
/// US 綴り `TransformBehavior` を使うため、実資産に合わせて US を読む
/// （ScanMove の `PARAM_BEHAVIOR` と同じ判断）。UK 綴りが来た場合のフォールバックも持つ。
const PARAM_TRANSFORM_BEHAVIOR: &str = "TransformBehavior";
const PARAM_TRANSFORM_BEHAVIOUR: &str = "TransformBehaviour";
/// TransformMascot は Java 定数も実 XML も同綴り。
const PARAM_TRANSFORM_MASCOT: &str = "TransformMascot";

/// アニメ最終フレーム到達時に画像セットと Behavior を差し替える（Java `Transform`）。
/// 差し替え自体は Manager がループ後に適用する（[`TransformRequest`]・意図的差異）。
pub(crate) struct TransformAction {
    animate: AnimateAction,
}

impl TransformAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> TransformAction {
        TransformAction {
            animate: AnimateAction::new(attrs, animations),
        }
    }

    /// Java Transform.tick L33-42 逐語:
    /// `transformation` 許可時のみ、`getTime() == animation.getDuration() - 1
    /// || animation.getDuration() == 1` で transform()。
    fn maybe_transform(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<(), ActionError> {
        if !env.transformation_allowed() {
            return Ok(());
        }
        let time = self.animate.bordered.base.get_time(mascot);
        let duration = self
            .animate
            .bordered
            .base
            .get_animation(mascot, env)?
            .map(animation_duration);
        // Java は getAnimation() が null なら NPE だが、animate_tick が既に
        // 有効アニメ不在を Err にしているため None はここでは通常到達しない。
        let Some(duration) = duration else {
            return Ok(());
        };
        if time != duration - 1 && duration != 1 {
            return Ok(());
        }
        // Java getTransformMascot/getTransformBehaviour: 既定は ""（自分の set / 空 Behavior）。
        let image_set = self
            .animate
            .bordered
            .base
            .text_attr(PARAM_TRANSFORM_MASCOT)
            .unwrap_or_default();
        let behavior = self
            .animate
            .bordered
            .base
            .text_attr(PARAM_TRANSFORM_BEHAVIOR)
            .or_else(|| {
                self.animate
                    .bordered
                    .base
                    .text_attr(PARAM_TRANSFORM_BEHAVIOUR)
            })
            .unwrap_or_default();
        mascot.request_transform(TransformRequest {
            image_set,
            behavior,
        });
        Ok(())
    }
}

impl Action for TransformAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.animate.init(mascot, env, rng)
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java Transform は hasNext を override しない（Animate のもの）
        self.animate.has_next(mascot, env, rng)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        // Java Transform.tick: super.tick()（= Animate.tick）→ 変身判定
        self.animate.next(mascot, env, rng)?;
        self.maybe_transform(mascot, env)
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.animate.is_draggable(mascot, env, rng)
    }
}

// =====================================================================
// Stay（Java Stay.java L18-36 相当）
// =====================================================================

pub(crate) struct StayAction {
    pub bordered: Bordered,
}

impl StayAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> StayAction {
        StayAction {
            bordered: Bordered::new(attrs, animations),
        }
    }
}

impl Action for StayAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.init_common(mascot, env)
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java Stay は hasNext をオーバーライドしない（ActionBase のもの）
        self.bordered.base.base_has_next(mascot, env)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.base.next_pre(mascot, env)?;
        self.bordered.border_tick(mascot, env); // super.tick()（L26）
        self.bordered.check_on_border(mascot, env)?; // L29-31
        self.bordered.base.apply_effective_animation(mascot, env) // L34
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.bordered.base.draggable(mascot, env)
    }
}

// =====================================================================
// Move（Java Move.java L18-146 相当）
// =====================================================================

const DEFAULT_TARGET_X: i32 = i32::MAX;
const DEFAULT_TARGET_Y: i32 = i32::MAX;

pub(crate) struct MoveAction {
    pub bordered: Bordered,
    /// Java hasTurning キャッシュ（L128-133）。
    has_turning: Option<bool>,
    /// Java turning（方向転換中フラグ）。
    pub turning: bool,
}

impl MoveAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> MoveAction {
        MoveAction {
            bordered: Bordered::new(attrs, animations),
            has_turning: None,
            turning: false,
        }
    }

    /// Java Move.tick L57-104 逐語（目標は `TargetX` / `TargetY` 属性）。
    pub(crate) fn move_tick(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<(), ActionError> {
        self.move_tick_with(mascot, env, TurnAnimMode::Automatic)
    }

    /// [`move_tick`](Self::move_tick) の転回アニメ選択方式つき版
    /// （MoveWithTurn が [`TurnAnimMode::WithTurnLast`] で使う）。
    pub(crate) fn move_tick_with(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        turn_mode: TurnAnimMode,
    ) -> Result<(), ActionError> {
        let target_x = self
            .bordered
            .base
            .num_attr(mascot, env, "TargetX", DEFAULT_TARGET_X)?;
        let target_y = self
            .bordered
            .base
            .num_attr(mascot, env, "TargetY", DEFAULT_TARGET_Y)?;
        move_tick_to(
            &mut self.bordered,
            &mut self.turning,
            &mut self.has_turning,
            mascot,
            env,
            MoveTarget {
                position: (target_x, target_y),
                // Java L69/L76: 既定値の軸は目標にしない（無条件クランプしない）
                aim_x: target_x != DEFAULT_TARGET_X,
                aim_y: target_y != DEFAULT_TARGET_Y,
            },
            turn_mode,
        )
    }
}

/// Java `hasTurningAnimation`（Move.java L128-133 / ScanMove.java L160-165 逐語）。
/// `has_turning` は呼び出し側が保持する遅延キャッシュ（Java の `Boolean hasTurning`）。
pub(crate) fn has_turning_animation(has_turning: &mut Option<bool>, bordered: &Bordered) -> bool {
    if has_turning.is_none() {
        *has_turning = Some(bordered.base.animations.iter().any(|a| a.is_turn));
    }
    has_turning.expect("just set")
}

/// Java `getAnimation`（Move.java L107-126 / ScanMove.java L141-158 逐語）:
/// `turning == animation.isTurn()` かつ条件が成立する最初のアニメの index。
pub(crate) fn get_turning_animation(
    bordered: &mut Bordered,
    turning: bool,
    mascot: &Mascot,
    env: &dyn EnvironmentView,
) -> Result<Option<usize>, ActionError> {
    for (index, animation) in bordered.base.animations.iter().enumerate() {
        if turning == animation.is_turn {
            let snapshot = mascot.eval_snapshot();
            let ctx = MascotContext {
                snapshot: &snapshot,
                env,
            };
            if animation_is_effective(animation, &mut bordered.base.vars, &ctx)? {
                return Ok(Some(index));
            }
        }
    }
    Ok(None)
}

/// 転回アニメの選択方式（Java Move / MoveWithTurn の `getAnimation` /
/// `hasTurningAnimation` 差）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnAnimMode {
    /// `turning == animation.isTurn()` の最初の有効アニメを選ぶ
    /// （Java Move.java L107-126 / ScanMove.java L141-158）。
    Automatic,
    /// turning 中は常に最後のアニメ、それ以外は `0..len-1` の最初の有効アニメ
    /// （Java MoveWithTurn.java L29-52 逐語。`isTurn` は見ない）。
    WithTurnLast,
}

/// 転回アニメの index 選択（[`TurnAnimMode`] 分岐）。
fn select_turn_animation(
    mode: TurnAnimMode,
    bordered: &mut Bordered,
    turning: bool,
    mascot: &Mascot,
    env: &dyn EnvironmentView,
) -> Result<Option<usize>, ActionError> {
    match mode {
        TurnAnimMode::Automatic => get_turning_animation(bordered, turning, mascot, env),
        TurnAnimMode::WithTurnLast => {
            if turning {
                // Java MoveWithTurn L32-33: force to last animation if turning
                return Ok(bordered.base.animations.len().checked_sub(1));
            }
            // Java MoveWithTurn L35-50: 0..size-1 の最初の有効アニメ（isTurn 不問）
            let last = bordered.base.animations.len().saturating_sub(1);
            for index in 0..last {
                let snapshot = mascot.eval_snapshot();
                let ctx = MascotContext {
                    snapshot: &snapshot,
                    env,
                };
                if animation_is_effective(
                    &bordered.base.animations[index],
                    &mut bordered.base.vars,
                    &ctx,
                )? {
                    return Ok(Some(index));
                }
            }
            Ok(None)
        }
    }
}

/// Move 系の目標（座標 + 軸ごとの有効フラグ）。Java Move は属性が既定値
/// (`Integer.MAX_VALUE`) の軸を無視するため軸ごとのフラグを持つ。ScanMove は
/// 常に両軸を目標にする（Java にガードが無い）ため `aim_x = aim_y = true`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MoveTarget {
    pub position: (i32, i32),
    /// x を目標にするか。
    pub aim_x: bool,
    /// y を目標にするか。
    pub aim_y: bool,
}

/// Java Move.tick L57-104 / ScanMove.tick L76-120 の共通本体:
/// border move → 境界チェック → 目標座標の変数注入 → 方向転換判定 → turning アニメ
/// 選択（完了で解除）→ pose 適用 → overshoot クランプ。
///
/// - `target`: 目標座標 + 軸ごとの有効フラグ（[`MoveTarget`]）
/// - `turn_mode`: 転回アニメの選択方式（Move / ScanMove = Automatic、
///   MoveWithTurn = WithTurnLast）
pub(crate) fn move_tick_to(
    bordered: &mut Bordered,
    turning: &mut bool,
    has_turning: &mut Option<bool>,
    mascot: &mut Mascot,
    env: &dyn EnvironmentView,
    target: MoveTarget,
    turn_mode: TurnAnimMode,
) -> Result<(), ActionError> {
    bordered.border_tick(mascot, env); // super.tick()（Move L58 / ScanMove L74）
    bordered.check_on_border(mascot, env)?; // Move L60-62 / ScanMove L79-81

    // Move L64-65（変数化は ComplexMove 相当の意図的差異）/ ScanMove L88-92:
    // 目標座標を変数へ注入し、アニメ条件から参照可能にする（turning アニメの
    // 条件評価より前に注入する）。
    bordered
        .base
        .vars
        .inject("TargetX", f64::from(target.position.0));
    bordered
        .base
        .vars
        .inject("TargetY", f64::from(target.position.1));

    let mut down = false;

    // Java L69-75 / L94-99: 方向転換アニメ有効化 + 向き更新
    if target.aim_x && mascot.anchor().0 != target.position.0 {
        let look_right = mascot.look_right();
        let direction_changed = *turning || (mascot.anchor().0 < target.position.0) != look_right;
        *turning = match turn_mode {
            // Java Move L128-133: hasTurningAnimation = いずれかのアニメが isTurn
            TurnAnimMode::Automatic => has_turning_animation(has_turning, bordered),
            // Java MoveWithTurn L54-57: hasTurningAnimation は常に true
            TurnAnimMode::WithTurnLast => true,
        } && direction_changed;
        mascot.set_look_right(mascot.anchor().0 < target.position.0);
    }
    if target.aim_y {
        down = mascot.anchor().1 < target.position.1; // Java L77 / L99
    }

    // Java L81-85 / L102-106: turning アニメ完了チェック（時間 >= duration で解除）
    let mut anim_index = select_turn_animation(turn_mode, bordered, *turning, mascot, env)?;
    if *turning {
        let dur = anim_index
            .map(|i| animation_duration(&bordered.base.animations[i]))
            .unwrap_or(0);
        if bordered.base.get_time(mascot) >= dur {
            *turning = false;
            anim_index = select_turn_animation(turn_mode, bordered, *turning, mascot, env)?;
        }
    }

    // Java L88 / L108: getAnimation().apply(getMascot(), getTime())
    if let Some(index) = anim_index {
        let rel = bordered.base.get_time(mascot);
        if let Some(pose) = animation_pose_at(&bordered.base.animations[index], rel) {
            apply_pose(pose, mascot);
        }
    }

    // Java L90-103 / L110-117: overshoot クランプ
    if target.aim_x {
        let (ax, ay) = mascot.anchor();
        let look_right = mascot.look_right();
        if (look_right && ax >= target.position.0) || (!look_right && ax <= target.position.0) {
            mascot.set_anchor((target.position.0, ay));
        }
    }
    if target.aim_y {
        let (ax, ay) = mascot.anchor();
        if (down && ay >= target.position.1) || (!down && ay <= target.position.1) {
            mascot.set_anchor((ax, target.position.1));
        }
    }
    Ok(())
}

impl Action for MoveAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.has_turning = None;
        self.turning = false;
        self.bordered.init_common(mascot, env)?;
        // ComplexMove.java L85-86 相当（Java 準拠）: init 時に TargetX/TargetY を
        // 解決して変数へ注入する。next_pre の refresh_hotspots（eval_quiet 経路）が
        // init 直後フレームから TargetY 条件を解決済み値で評価できるようにするため
        // （Java も init 時 putVariable・refreshHotspots は解決済み値で評価）。
        // 属性無し時は num_attr の既定値（DEFAULT_TARGET_X/Y）が注入される。
        let target_x = self
            .bordered
            .base
            .num_attr(mascot, env, "TargetX", DEFAULT_TARGET_X)?;
        let target_y = self
            .bordered
            .base
            .num_attr(mascot, env, "TargetY", DEFAULT_TARGET_Y)?;
        self.bordered
            .base
            .vars
            .inject("TargetX", f64::from(target_x));
        self.bordered
            .base
            .vars
            .inject("TargetY", f64::from(target_y));
        Ok(())
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java Move L36-54 の骨格（base_has_next → turning → target 判定）
        if !self.bordered.base.base_has_next(mascot, env)? {
            return Ok(false);
        }
        if self.turning {
            return Ok(true);
        }
        let target_x = self
            .bordered
            .base
            .num_attr(mascot, env, "TargetX", DEFAULT_TARGET_X)?;
        let target_y = self
            .bordered
            .base
            .num_attr(mascot, env, "TargetY", DEFAULT_TARGET_Y)?;
        let aim_x = target_x != DEFAULT_TARGET_X;
        let aim_y = target_y != DEFAULT_TARGET_Y;
        if !aim_x && !aim_y {
            // target 両方省略。Java L48-53 は false（即終了）だが、デレマス資産は
            // 「target 無し Move + Duration 明示」をその場アニメ / その場移動として
            // 使う（`うさぎ` / `うつぶせ` / 会話系 `Walk` 等・実測 22 箇所）。
            // Duration 明示時のみ継続し、終了は冒頭の base_has_next（time < Duration）が
            // 担う。未指定は Java 同様の即終了（新規挙動ゼロ）。
            // 意図的差異・design §1.10 (z-8)。
            return Ok(self.bordered.base.attrs.contains_key("Duration"));
        }
        let anchor = mascot.anchor();
        Ok((aim_x && anchor.0 != target_x) || (aim_y && anchor.1 != target_y))
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.base.next_pre(mascot, env)?;
        self.move_tick(mascot, env)
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.bordered.base.draggable(mascot, env)
    }
}

// =====================================================================
// IE 共通ヘルパ（getActiveIE 相当・WalkWithIE / FallWithIE / ThrowIE）
// =====================================================================

/// 挿入点（anchor）基準の gated activeIE（Java MascotEnvironment.getActiveIE()
/// 相当・getWorkArea() anchorベース + active_ie_effective gating）。
pub(crate) fn gated_active_ie(env: &dyn EnvironmentView, anchor: (i32, i32)) -> env::AreaState {
    let slot = env::resolve_work_area(env, anchor);
    let work_area = env.work_area_state(slot);
    env::active_ie_effective(env, &work_area)
}

// =====================================================================
// ScanMove（Java ScanMove.java L21-182 相当・アフォーダンス探索移動）
// =====================================================================

/// Java の `Behaviour` / `TargetBehaviour` / `TargetLook` 属性の既定値
/// （ScanMove L24-31 / ScanJump L24-31 / ComplexMove L29-36 / ScanInteract L24-31。
/// 属性名は Java 定数では UK 綴りだが実 XML は US 綴りを使うため、実資産
/// （デレマスしめじ v1.9）に合わせて US 綴りを読む）。
pub(crate) const PARAM_BEHAVIOR: &str = "Behavior";
pub(crate) const PARAM_TARGET_BEHAVIOR: &str = "TargetBehavior";
pub(crate) const PARAM_TARGET_LOOK: &str = "TargetLook";

/// 放送中（`Affordance` 属性を持つ）個体を探して近づき、到達したら自分と相手の
/// Behavior を差し替える（Java `ScanMove` 逐語）。
pub(crate) struct ScanMoveAction {
    pub bordered: Bordered,
    /// Java `hasTurning` キャッシュ（L35）。
    has_turning: Option<bool>,
    /// Java `turning`（L37）。
    turning: bool,
    /// 探索する affordance（Java `getAffordance()`・`Affordance` 属性を init で解決）。
    affordance: String,
    /// 探索相手の index（init で決定・[`AffordanceScanEntry::index`]）。
    /// Java の `WeakReference<Mascot> target` 相当（毎 tick スナップショットを
    /// 引き直して affordance 保持を確認するため、index で足りる）。
    target_index: Option<usize>,
}

impl ScanMoveAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> ScanMoveAction {
        ScanMoveAction {
            bordered: Bordered::new(attrs, animations),
            has_turning: None,
            turning: false,
            affordance: String::new(),
            target_index: None,
        }
    }

    /// Java `getBehaviour()`（L171-173）: `Behaviour` 属性（既定 ""）。
    fn behavior(&mut self) -> String {
        self.bordered
            .base
            .text_attr(PARAM_BEHAVIOR)
            .unwrap_or_default()
    }

    /// Java `getTargetBehaviour()`（L175-177）: `TargetBehaviour` 属性（既定 ""）。
    fn target_behavior(&mut self) -> String {
        self.bordered
            .base
            .text_attr(PARAM_TARGET_BEHAVIOR)
            .unwrap_or_default()
    }

    /// スキャン対象の現在 anchor（Java `target.get().getAnchor()`）。
    /// 相手が消えた / affordance を失った場合は None（Java L69 の contains 判定と
    /// L83-86 の null 判定を 1 回のスナップショット参照で兼ねる）。
    fn target_anchor(&self, env: &dyn EnvironmentView) -> Option<(i32, i32)> {
        super::affordance_target_anchor(env, self.target_index, &self.affordance)
    }

    /// Java ScanMove.tick L72-138 逐語。
    fn scan_tick(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<(), ActionError> {
        // Java L77: cannot broadcast while scanning for an affordance
        // （next_pre が放送した自分の affordance を打ち消す）
        mascot.clear_affordances();

        let Some(target) = self.target_anchor(env) else {
            // Java L83-86: targetMascot == null → super.tick()（border move）のみ。
            // ただし境界検査（L79-81）は相手の有無に関わらず先に走る（相手を失っても
            // 床外なら LostGround で Fall に復帰する）。
            self.bordered.border_tick(mascot, env);
            self.bordered.check_on_border(mascot, env)?;
            return Ok(());
        };

        move_tick_to(
            &mut self.bordered,
            &mut self.turning,
            &mut self.has_turning,
            mascot,
            env,
            MoveTarget {
                position: target,
                // Java ScanMove は Move と違い「既定値の軸を無視する」ガードを
                // 持たない（target は常に実座標）ため両軸を目標にする
                aim_x: true,
                aim_y: true,
            },
            TurnAnimMode::Automatic,
        )?;

        // Java L119-137: 到達判定（両軸一致 && turning 中でない）→ Behavior 差し替え
        if self.turning || mascot.anchor() != target {
            return Ok(());
        }
        let behavior = self.behavior();
        let target_behavior = self.target_behavior();
        let flip_look = self
            .bordered
            .base
            .bool_attr(mascot, env, PARAM_TARGET_LOOK, false)?;
        // 自分 / 相手の差し替えは Manager がループ後に Java と同じ順序で適用する
        mascot.request_affordance_arrival(AffordanceArrival {
            behavior,
            target_index: self.target_index,
            target_behavior: Some(target_behavior),
            flip_look,
        });
        Ok(())
    }
}

impl Action for ScanMoveAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.has_turning = None;
        self.turning = false;
        // Java L45: super.init（BorderedAction.init = Base 初期化 + border 解決）
        self.bordered.init_common(mascot, env)?;

        // Java L48: cannot broadcast while scanning for an affordance
        mascot.clear_affordances();

        // Java L50-55: 相手探索 + TargetX/TargetY の注入。
        // 相手不在時は Java が putVariable(name, null) するが、その場合 hasNext が
        // false になり tick に到達しない（＝注入値は使われない）ため、Rust は
        // 注入しない（既存注入値は前 tick のまま。資産の ScanMove アニメは
        // TargetX/Y 条件を持たないため観測差なし）。
        self.affordance = self
            .bordered
            .base
            .text_attr("Affordance")
            .unwrap_or_default();
        self.target_index = super::find_affordance_target(env, &self.affordance);
        if let Some(anchor) = self.target_anchor(env) {
            self.bordered
                .base
                .vars
                .inject("TargetX", f64::from(anchor.0));
            self.bordered
                .base
                .vars
                .inject("TargetY", f64::from(anchor.1));
        }
        Ok(())
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L58-70: super.hasNext() かつ（turning || 相手が affordance を保持）
        if !self.bordered.base.base_has_next(mascot, env)? {
            return Ok(false);
        }
        if self.turning {
            return Ok(true);
        }
        Ok(self.target_anchor(env).is_some())
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        // Java は next() を override しない（ActionBase.next の順序: resetVariables →
        // affordances 更新 → refreshHotspots → tick）。tick 内で affordance を
        // 消すため、next 終了時の affordances は空になる（Java L77 と同じ観察結果）。
        self.bordered.base.next_pre(mascot, env)?;
        self.scan_tick(mascot, env)
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.bordered.base.draggable(mascot, env)
    }
}

// =====================================================================
// ThrowIE（Java ThrowIE.java L21-91 相当）
// =====================================================================

const DEFAULT_INITIAL_VX: i32 = 32;
const DEFAULT_INITIAL_VY: i32 = -10;
const DEFAULT_GRAVITY: f64 = 0.5;

pub(crate) struct ThrowIEAction {
    pub bordered: Bordered,
    scaling: f64,
    /// init 時の activeWindowId（Java L46）。
    active_window_id: i64,
}

impl ThrowIEAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> ThrowIEAction {
        ThrowIEAction {
            bordered: Bordered::new(attrs, animations),
            scaling: 1.0,
            active_window_id: 0,
        }
    }
}

impl Action for ThrowIEAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.init_common(mascot, env)?;
        self.scaling = mascot.scale(); // Java L45
        self.active_window_id = env.active_window_id(); // Java L46
        Ok(())
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L50-57 逐語: throwing && super.hasNext() && activeIE.visible &&
        // activeWindowId 一致（窓切替検出）
        Ok(env.throwing_allowed()
            && self.bordered.base.base_has_next(mascot, env)?
            && gated_active_ie(env, mascot.anchor()).visible
            && self.active_window_id == env.active_window_id())
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.base.next_pre(mascot, env)?;
        // Java L61: super.tick() = Animate.tick（border move / LostGround / アニメ）
        self.bordered.border_tick(mascot, env);
        self.bordered.check_on_border(mascot, env)?;
        self.bordered.base.apply_effective_animation(mascot, env)?;

        // Java L63-77: activeIE 投擲
        let active_ie = gated_active_ie(env, mascot.anchor());
        if active_ie.visible {
            let scaling = self.scaling;
            let ivx = self
                .bordered
                .base
                .num_attr(mascot, env, "InitialVX", DEFAULT_INITIAL_VX)?;
            let ivy = self
                .bordered
                .base
                .num_attr(mascot, env, "InitialVY", DEFAULT_INITIAL_VY)?;
            let gravity = self
                .bordered
                .base
                .f64_attr(mascot, env, "Gravity", DEFAULT_GRAVITY)?;
            // Java L67-70 / L72-75 逐語（lookRight で +32 / -32。y は共通・
            // round(initialVy * scaling + getTime * gravity * scaling)）
            let dx = java_round(f64::from(ivx) * scaling);
            let dy = java_round(
                f64::from(ivy) * scaling
                    + f64::from(self.bordered.base.get_time(mascot)) * gravity * scaling,
            );
            if mascot.look_right() {
                env.move_active_window(active_ie.left + dx, active_ie.top + dy);
            } else {
                env.move_active_window(active_ie.left - dx, active_ie.top + dy);
            }
        }
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.bordered.base.draggable(mascot, env)
    }
}

// =====================================================================
// WalkWithIE（Java WalkWithIE.java L20-98 相当）
// =====================================================================

pub(crate) struct WalkWithIEAction {
    pub move_action: MoveAction,
}

impl WalkWithIEAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> WalkWithIEAction {
        WalkWithIEAction {
            move_action: MoveAction::new(attrs, animations),
        }
    }
}

impl Action for WalkWithIEAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.move_action.init(mascot, env, _rng)
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L43-44: throwing && super.hasNext()
        Ok(env.throwing_allowed() && self.move_action.has_next(mascot, env, rng)?)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        // Java L49-52: activeIE 可視チェック（不可視 → LostGround）
        let active_ie = gated_active_ie(env, mascot.anchor());
        if !active_ie.visible {
            return Err(ActionError::LostGround); // "Window is not visible"
        }

        // Java L54-58: scaling は使わない quirk（Java コメント L54-56 踏襲）
        let offset_x = self
            .move_action
            .bordered
            .base
            .num_attr(mascot, env, "IeOffsetX", 0)?;
        let offset_y = self
            .move_action
            .bordered
            .base
            .num_attr(mascot, env, "IeOffsetY", 0)?;

        // Java L61-71: hold-check（失敗 → LostGround）
        let anchor = mascot.anchor();
        let look_right = mascot.look_right();
        if look_right {
            if anchor.0 - offset_x != active_ie.left || anchor.1 + offset_y != active_ie.bottom {
                return Err(ActionError::LostGround);
            }
        } else if anchor.0 + offset_x != active_ie.right || anchor.1 + offset_y != active_ie.bottom
        {
            return Err(ActionError::LostGround);
        }

        self.move_action.bordered.base.next_pre(mascot, env)?;
        self.move_action.move_tick(mascot, env)?;

        // Java L76-88: activeIE 移動（連動）
        if active_ie.visible {
            if mascot.look_right() {
                let (_, ay) = mascot.anchor();
                env.move_active_window(
                    mascot.anchor().0 - offset_x,
                    ay + offset_y - active_ie.height(),
                );
            } else {
                let (ax, ay) = mascot.anchor();
                env.move_active_window(
                    ax + offset_x - active_ie.width(),
                    ay + offset_y - active_ie.height(),
                );
            }
        }
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.move_action.bordered.base.draggable(mascot, env)
    }
}

// =====================================================================
// FallWithIE（Java FallWithIE.java L21-90 相当）
// =====================================================================

pub(crate) struct FallWithIEAction {
    pub fall_action: FallAction,
}

impl FallWithIEAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> FallWithIEAction {
        FallWithIEAction {
            fall_action: FallAction::new(attrs, animations),
        }
    }
}

impl Action for FallWithIEAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.fall_action.init(mascot, env, _rng)
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L35-37: throwing && super.hasNext()
        Ok(env.throwing_allowed() && self.fall_action.has_next(mascot, env, rng)?)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        // Java L41-63: WalkWithIE と同一の hold-check
        let active_ie = gated_active_ie(env, mascot.anchor());
        if !active_ie.visible {
            return Err(ActionError::LostGround);
        }
        let offset_x = self
            .fall_action
            .base
            .num_attr(mascot, env, "IeOffsetX", 0)?;
        let offset_y = self
            .fall_action
            .base
            .num_attr(mascot, env, "IeOffsetY", 0)?;
        let anchor = mascot.anchor();
        let look_right = mascot.look_right();
        if look_right {
            if anchor.0 - offset_x != active_ie.left || anchor.1 + offset_y != active_ie.bottom {
                return Err(ActionError::LostGround);
            }
        } else if anchor.0 + offset_x != active_ie.right || anchor.1 + offset_y != active_ie.bottom
        {
            return Err(ActionError::LostGround);
        }

        // Java L65: super.tick() = Fall.tick
        self.fall_action.base.next_pre(mascot, env)?;
        super::fall_tick(&mut self.fall_action, mascot, env)?;

        // Java L68-80: activeIE 移動
        if active_ie.visible {
            if mascot.look_right() {
                let (_, ay) = mascot.anchor();
                env.move_active_window(
                    mascot.anchor().0 - offset_x,
                    ay + offset_y - active_ie.height(),
                );
            } else {
                let (ax, ay) = mascot.anchor();
                env.move_active_window(
                    ax + offset_x - active_ie.width(),
                    ay + offset_y - active_ie.height(),
                );
            }
        }
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.fall_action.base.draggable(mascot, env)
    }
}

// =====================================================================
// Breed（Java Breed.java L22-167 相当・Delegate 内部クラス込み）
// =====================================================================

const BREED_DEFAULT_BORN_X: i32 = 0;
const BREED_DEFAULT_BORN_Y: i32 = 0;
const BREED_DEFAULT_BORN_BEHAVIOR: &str = "";
const BREED_DEFAULT_BORN_MASCOT: &str = "";
const BREED_DEFAULT_BORN_TRANSIENT: bool = false;
const BREED_DEFAULT_BORN_INTERVAL: i32 = 1;
const BREED_DEFAULT_BORN_COUNT: i32 = 1;

/// Java `Breed.Delegate`（Breed.java L26-142）の共用実装。
/// Java は Breed / BreedMove / BreedJump / ComplexMove / ComplexJump が
/// それぞれ同名の内部クラス Delegate を持つが中身は同一のため 1 型に集約する。
#[derive(Debug, Clone, Copy)]
pub(crate) struct BreedDelegate {
    /// Java Delegate.scaling（L49）。
    scaling: f64,
}

impl BreedDelegate {
    pub(crate) fn new() -> BreedDelegate {
        BreedDelegate { scaling: 1.0 }
    }

    /// Java Delegate.initScaling L55-57 逐語。
    pub(crate) fn init_scaling(&mut self, mascot: &Mascot) {
        self.scaling = mascot.scale();
    }

    /// Java Delegate.isEnabled L59-63 逐語: BornTransient が true → transients 設定、
    /// それ以外は breeding 設定。
    pub(crate) fn is_enabled(
        &self,
        base: &mut Base,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<bool, ActionError> {
        let born_transient =
            base.bool_attr(mascot, env, "BornTransient", BREED_DEFAULT_BORN_TRANSIENT)?;
        Ok(if born_transient {
            env.transients_enabled()
        } else {
            env.breeding_allowed()
        })
    }

    /// Java Delegate.isIntervalFrame L65-67 逐語: `time % BornInterval == 0`。
    /// 0 除算は評価エラーとして返す（Java は ArithmeticException が tick エラーに
    /// なるが、単一イベントループの Rust では panic = アプリ全体の異常終了になる）。
    pub(crate) fn is_interval_frame(
        &self,
        base: &mut Base,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<bool, ActionError> {
        let interval = self.born_interval(base, mascot, env)?;
        Ok(base.get_time(mascot) % interval == 0)
    }

    /// Java Delegate.getBornInterval L135-137 + 正値検証（Java Delegate.validateBornInterval
    /// L109-113 と同じ判定を 1 箇所に集約し、0 除算の panic を防ぐ）。
    fn born_interval(
        &self,
        base: &mut Base,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<i32, ActionError> {
        let interval = base.num_attr(mascot, env, "BornInterval", BREED_DEFAULT_BORN_INTERVAL)?;
        if interval < 1 {
            return Err(ActionError::Eval(EvalError {
                expr: "BornInterval".to_string(),
                message: "BornInterval must be positive".to_string(),
            }));
        }
        Ok(interval)
    }

    /// Java Delegate.validateBornCount L103-107 逐語（VariableException 相当）。
    pub(crate) fn validate_born_count(
        &self,
        base: &mut Base,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<(), ActionError> {
        let born_count = base.num_attr(mascot, env, "BornCount", BREED_DEFAULT_BORN_COUNT)?;
        if born_count < 1 {
            return Err(ActionError::Eval(EvalError {
                expr: "BornCount".to_string(),
                message: "BornCount must be positive".to_string(),
            }));
        }
        Ok(())
    }

    /// Java Delegate.validateBornInterval L109-113 逐語（Breed 以外が init で呼ぶ）。
    pub(crate) fn validate_born_interval(
        &self,
        base: &mut Base,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<(), ActionError> {
        self.born_interval(base, mascot, env).map(|_| ())
    }

    /// Java Delegate.breed L73-101 逐語。
    pub(crate) fn breed(
        &self,
        base: &mut Base,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<(), ActionError> {
        let born_x = base.num_attr(mascot, env, "BornX", BREED_DEFAULT_BORN_X)?;
        let born_y = base.num_attr(mascot, env, "BornY", BREED_DEFAULT_BORN_Y)?;
        let born_mascot = base
            .text_attr("BornMascot")
            .unwrap_or_else(|| BREED_DEFAULT_BORN_MASCOT.to_string());
        let born_behavior = base
            // Java 論理キー BornBehaviour は schema により生 XML 名 BornBehavior へ変換される
            .text_attr("BornBehavior")
            .unwrap_or_else(|| BREED_DEFAULT_BORN_BEHAVIOR.to_string());
        let born_count = base.num_attr(mascot, env, "BornCount", BREED_DEFAULT_BORN_COUNT)?;

        // Java L74: childType = getConfiguration(getBornMascot()) != null ? 親 imageSet。
        // BornMascot のロード済み判定は本層で不可能のため「非空ならその名・
        // 既定は親 imageSet 名」とした（意図的差異・Breed doc 参照）
        let child_type: String = if !born_mascot.is_empty() {
            born_mascot
        } else {
            mascot.image_set_name().to_string()
        };

        // Java L83-90 逐語: lookRight 分岐 + round(bornX * scaling)
        let (ax, ay) = mascot.anchor();
        let look_right = mascot.look_right();
        let born_x_scaled = java_round(f64::from(born_x) * self.scaling);
        let born_y_scaled = java_round(f64::from(born_y) * self.scaling);
        let anchor = if look_right {
            (ax - born_x_scaled, ay + born_y_scaled)
        } else {
            (ax + born_x_scaled, ay + born_y_scaled)
        };

        for _index in 0..born_count {
            // Java L94（manager.add）→ Rust は queue_spawn（次 tick 反映・意図的差異）。
            // BornBehaviour 名（L93 getBornBehavior()）は第 4 引数で queue へ伝播する（#8）
            env.queue_spawn(&child_type, anchor, look_right, &born_behavior);
        }
        Ok(())
    }
}

pub(crate) struct BreedAction {
    pub bordered: Bordered,
    delegate: BreedDelegate,
}

impl BreedAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> BreedAction {
        BreedAction {
            bordered: Bordered::new(attrs, animations),
            delegate: BreedDelegate::new(),
        }
    }

    /// Java Delegate.isPenultimateFrame L69-71 逐語。
    fn is_penultimate_frame(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<bool, ActionError> {
        Ok(self.bordered.base.get_time(mascot)
            == self.bordered.base.tolerant_anim_duration(mascot, env)? - 1)
    }
}

impl Action for BreedAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.init_common(mascot, env)?;
        // Java L154-155: initScaling + validateBornCount
        self.delegate.init_scaling(mascot);
        self.delegate
            .validate_born_count(&mut self.bordered.base, mascot, env)?;
        Ok(())
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java Breed は hasNext をオーバーライドしない → Animate の hasNext
        Ok(self.bordered.base.base_has_next(mascot, env)?
            && self.bordered.base.get_time(mascot)
                < self.bordered.base.tolerant_anim_duration(mascot, env)?)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.base.next_pre(mascot, env)?;
        // Java L159-165: super.tick()（Animate.tick 相当）
        animate_tick(&mut self.bordered, mascot, env)?;

        // Java L162-165: penultimate frame 且つ enabled → 生む
        if self.is_penultimate_frame(mascot, env)?
            && self
                .delegate
                .is_enabled(&mut self.bordered.base, mascot, env)?
        {
            self.delegate.breed(&mut self.bordered.base, mascot, env)?;
        }
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.bordered.base.draggable(mascot, env)
    }
}

// =====================================================================
// BreedMove（Java BreedMove.java L17-44 相当・Move 派生）
// =====================================================================

/// Java `BreedMove`（Move + Delegate）: Move の移動に加えて `BornInterval` ごとに
/// 増殖する。turning 中は増殖しない（Java L39）。
pub(crate) struct BreedMoveAction {
    pub move_action: MoveAction,
    delegate: BreedDelegate,
}

impl BreedMoveAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> BreedMoveAction {
        BreedMoveAction {
            move_action: MoveAction::new(attrs, animations),
            delegate: BreedDelegate::new(),
        }
    }
}

impl Action for BreedMoveAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        // Java L28-32: super.init（Move）→ initScaling + validateBornCount +
        // validateBornInterval
        self.move_action.init(mascot, env, rng)?;
        self.delegate.init_scaling(mascot);
        self.delegate
            .validate_born_count(&mut self.move_action.bordered.base, mascot, env)?;
        self.delegate
            .validate_born_interval(&mut self.move_action.bordered.base, mascot, env)?;
        Ok(())
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java BreedMove は hasNext をオーバーライドしない（Move のもの）
        self.move_action.has_next(mascot, env, rng)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.move_action.bordered.base.next_pre(mascot, env)?;
        // Java L37: super.tick()（Move.tick）
        self.move_action.move_tick(mascot, env)?;

        // Java L39-42: interval frame 且つ turning 中でない 且つ enabled → 生む
        if self
            .delegate
            .is_interval_frame(&mut self.move_action.bordered.base, mascot, env)?
            && !self.move_action.turning
            && self
                .delegate
                .is_enabled(&mut self.move_action.bordered.base, mascot, env)?
        {
            self.delegate
                .breed(&mut self.move_action.bordered.base, mascot, env)?;
        }
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.move_action.bordered.base.draggable(mascot, env)
    }
}

// =====================================================================
// Turn（Java Turn.java L16-50 相当・BorderedAction 派生）
// =====================================================================

/// Java `Turn`: 向きが `LookRight` 属性と食い違った瞬間に向き直るアニメを再生する。
/// `LookRight` の評価は hasNext と tick の両方で行われる（Java L30 / L36）。
pub(crate) struct TurnAction {
    pub bordered: Bordered,
    /// Java `turning`（L21）: LookRight と現在の向きが食い違って以降のラッチ。
    turning: bool,
}

impl TurnAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> TurnAction {
        TurnAction {
            bordered: Bordered::new(attrs, animations),
            turning: false,
        }
    }

    /// Java `isLookRight()` L47-49 逐語: `LookRight` 属性（既定 = 現在の向きの反転）。
    fn is_look_right(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<bool, ActionError> {
        let default = !mascot.look_right();
        self.bordered
            .base
            .bool_attr(mascot, env, "LookRight", default)
    }
}

impl Action for TurnAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.turning = false;
        self.bordered.init_common(mascot, env)
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L28-32: turning = turning || isLookRight() != mascot.isLookRight();
        // super.hasNext() && turning && getTime() < getAnimation().getDuration()
        let look_right = self.is_look_right(mascot, env)?;
        self.turning = self.turning || look_right != mascot.look_right();
        Ok(self.bordered.base.base_has_next(mascot, env)?
            && self.turning
            && self.bordered.base.get_time(mascot)
                < self.bordered.base.tolerant_anim_duration(mascot, env)?)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.base.next_pre(mascot, env)?;
        // Java L36: setLookRight(isLookRight())
        let look_right = self.is_look_right(mascot, env)?;
        mascot.set_look_right(look_right);
        // Java L38: super.tick()（border move）
        self.bordered.border_tick(mascot, env);
        // Java L40-42: 境界外 → LostGround
        self.bordered.check_on_border(mascot, env)?;
        // Java L44: getAnimation().apply(getMascot(), getTime())（基底 ActionBase の
        // getAnimation = 最初の有効アニメ）
        self.bordered.base.apply_effective_animation(mascot, env)
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.bordered.base.draggable(mascot, env)
    }
}

// =====================================================================
// MoveWithTurn（Java MoveWithTurn.java L18-57 相当・Move 派生・Deprecated）
// =====================================================================

/// Java `MoveWithTurn`（`@Deprecated since 1.0.21`・Move に turning が統合された
/// 旧クラス）: 転回中は常に最後のアニメ、それ以外は `0..len-1` の最初の有効アニメを
/// 使う。アニメ 2 本以上が前提（Java L24-27 の IllegalArgumentException。
/// Rust は warn して続行する・意図的差異）。
pub(crate) struct MoveWithTurnAction {
    pub move_action: MoveAction,
}

impl MoveWithTurnAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> MoveWithTurnAction {
        if animations.len() < 2 {
            log::warn!(
                "MoveWithTurn requires at least 2 animations; turning will not animate (got {})",
                animations.len()
            );
        }
        MoveWithTurnAction {
            move_action: MoveAction::new(attrs, animations),
        }
    }
}

impl Action for MoveWithTurnAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        // Java は init をオーバーライドしない（Move のもの）
        self.move_action.init(mascot, env, rng)
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java は hasNext をオーバーライドしない（Move のもの）
        self.move_action.has_next(mascot, env, rng)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.move_action.bordered.base.next_pre(mascot, env)?;
        // Java Move.tick を WithTurnLast の getAnimation / hasTurningAnimation で実行
        self.move_action
            .move_tick_with(mascot, env, TurnAnimMode::WithTurnLast)
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.move_action.bordered.base.draggable(mascot, env)
    }
}

// =====================================================================
// Interact（Java Interact.java L19-51 相当・Animate 派生）
// =====================================================================

/// Java `Interact`: 自分の anchor に他個体が重なっている間だけアニメを続け、
/// 最終フレームで自分を `Behaviour` 属性の行動へ差し替える（相手は変えない）。
pub(crate) struct InteractAction {
    animate: AnimateAction,
}

impl InteractAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> InteractAction {
        InteractAction {
            animate: AnimateAction::new(attrs, animations),
        }
    }

    /// Java `getBehaviour()` L49-51（既定 ""）。
    fn behavior(&mut self) -> String {
        self.animate
            .bordered
            .base
            .text_attr(PARAM_BEHAVIOR)
            .unwrap_or_default()
    }
}

impl Action for InteractAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.animate.init(mascot, env, rng)
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L30-32: super.hasNext() && manager.hasOverlappingMascotsAtPoint(anchor)
        Ok(self.animate.has_next(mascot, env, rng)? && env.overlapping_mascots_at(mascot.anchor()))
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        // Java L35-36: super.tick()（Animate.tick）
        self.animate.next(mascot, env, rng)?;

        // Java L38-46: 最終フレームで Behavior 差し替え（空名は何もしない）
        let time = self.animate.bordered.base.get_time(mascot);
        let duration = self
            .animate
            .bordered
            .base
            .tolerant_anim_duration(mascot, env)?;
        if (time == duration - 1 || duration == 1) && !self.behavior().trim().is_empty() {
            let behavior = self.behavior();
            mascot.request_affordance_arrival(AffordanceArrival {
                behavior,
                target_index: None,
                target_behavior: None,
                flip_look: false,
            });
        }
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.animate.is_draggable(mascot, env, rng)
    }
}

// =====================================================================
// SelfDestruct（Java SelfDestruct.java L16-32 相当・Animate 派生）
// =====================================================================

/// Java `SelfDestruct`: アニメ最終フレームで自分を消す。
pub(crate) struct SelfDestructAction {
    animate: AnimateAction,
}

impl SelfDestructAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> SelfDestructAction {
        SelfDestructAction {
            animate: AnimateAction::new(attrs, animations),
        }
    }
}

impl Action for SelfDestructAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.animate.init(mascot, env, rng)
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.animate.has_next(mascot, env, rng)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        // Java L25: super.tick()（Animate.tick）
        self.animate.next(mascot, env, rng)?;

        // Java L27-30: 最終フレームで dispose
        let time = self.animate.bordered.base.get_time(mascot);
        let duration = self
            .animate
            .bordered
            .base
            .tolerant_anim_duration(mascot, env)?;
        if time == duration - 1 || duration == 1 {
            mascot.dispose();
        }
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.animate.is_draggable(mascot, env, rng)
    }
}

// =====================================================================
// ScanInteract（Java ScanInteract.java L21-160 相当・BorderedAction 派生）
// =====================================================================

/// Java `ScanInteract`: その場に留まり、アフォーダンス保持者を毎 tick 追跡して
/// `TargetX`/`TargetY` を注入、転回アニメを挟みつつ最終フレームで自分（`Behaviour`）
/// と相手（`TargetBehaviour`・空なら触れない）の Behavior を差し替える。
pub(crate) struct ScanInteractAction {
    pub bordered: Bordered,
    has_turning: Option<bool>,
    turning: bool,
    affordance: String,
    target_index: Option<usize>,
}

impl ScanInteractAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> ScanInteractAction {
        ScanInteractAction {
            bordered: Bordered::new(attrs, animations),
            has_turning: None,
            turning: false,
            affordance: String::new(),
            target_index: None,
        }
    }

    fn behavior(&mut self) -> String {
        self.bordered
            .base
            .text_attr(PARAM_BEHAVIOR)
            .unwrap_or_default()
    }

    fn target_behavior(&mut self) -> String {
        self.bordered
            .base
            .text_attr(PARAM_TARGET_BEHAVIOR)
            .unwrap_or_default()
    }

    /// Java ScanInteract の turning-aware `getAnimation`（L118-136）で選ばれた
    /// アニメの duration。候補が無ければ基底の tolerant 値にフォールバックする。
    fn selected_anim_duration(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<i32, ActionError> {
        match select_turn_animation(
            TurnAnimMode::Automatic,
            &mut self.bordered,
            self.turning,
            mascot,
            env,
        )? {
            Some(index) => Ok(animation_duration(&self.bordered.base.animations[index])),
            None => self.bordered.base.tolerant_anim_duration(mascot, env),
        }
    }
}

impl Action for ScanInteractAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.has_turning = None;
        self.turning = false;
        self.bordered.init_common(mascot, env)?;
        // Java L47-51: cannot broadcast while scanning + TargetX/TargetY を null に。
        // Rust は変数を消せないため注入しない（不在時の hasNext/tick は相手を
        // 保持している間しか動かない・ScanMove と同じ意図的差異）。
        mascot.clear_affordances();
        self.affordance = self
            .bordered
            .base
            .text_attr("Affordance")
            .unwrap_or_default();
        self.target_index = None;
        Ok(())
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L54-57: super.hasNext() && (turning || getTime() < animDuration)
        if !self.bordered.base.base_has_next(mascot, env)? {
            return Ok(false);
        }
        Ok(self.turning
            || self.bordered.base.get_time(mascot) < self.selected_anim_duration(mascot, env)?)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.base.next_pre(mascot, env)?;
        // Java L61-64: super.tick()（border move）+ clear affordances
        self.bordered.border_tick(mascot, env);
        mascot.clear_affordances();
        // Java L66-68: 境界外 → LostGround
        self.bordered.check_on_border(mascot, env)?;

        // Java L70-77: 相手の再探索 + TargetX/TargetY 注入（保持中のみ）
        if super::affordance_target_anchor(env, self.target_index, &self.affordance).is_none() {
            self.target_index = super::find_affordance_target(env, &self.affordance);
        }
        let Some(target) =
            super::affordance_target_anchor(env, self.target_index, &self.affordance)
        else {
            return Ok(());
        };
        self.bordered
            .base
            .vars
            .inject("TargetX", f64::from(target.0));
        self.bordered
            .base
            .vars
            .inject("TargetY", f64::from(target.1));

        // Java L80-84: 向き更新 + 転回アニメ有効化
        if mascot.anchor().0 != target.0 {
            let look_right = mascot.look_right();
            self.turning = has_turning_animation(&mut self.has_turning, &self.bordered)
                && (self.turning || (mascot.anchor().0 < target.0) != look_right);
            mascot.set_look_right(mascot.anchor().0 < target.0);
        }

        // Java L86-92: 転回アニメ完了 → setTime 巻き戻し + 解除
        let mut anim_index = select_turn_animation(
            TurnAnimMode::Automatic,
            &mut self.bordered,
            self.turning,
            mascot,
            env,
        )?;
        if self.turning {
            let duration = anim_index
                .map(|i| animation_duration(&self.bordered.base.animations[i]))
                .unwrap_or(0);
            if self.bordered.base.get_time(mascot) >= duration {
                let rewound = self.bordered.base.get_time(mascot) - duration;
                self.bordered.base.set_time(mascot, rewound);
                self.turning = false;
                anim_index = select_turn_animation(
                    TurnAnimMode::Automatic,
                    &mut self.bordered,
                    self.turning,
                    mascot,
                    env,
                )?;
            }
        }

        // Java L94: animation.apply(mascot, getTime())
        if let Some(index) = anim_index {
            let rel = self.bordered.base.get_time(mascot);
            if let Some(pose) = animation_pose_at(&self.bordered.base.animations[index], rel) {
                apply_pose(pose, mascot);
            }
        }

        // Java L95-114: 最終フレームで Behavior 差し替え
        let duration = self.selected_anim_duration(mascot, env)?;
        let time = self.bordered.base.get_time(mascot);
        let behavior = self.behavior();
        if !self.turning && (time == duration - 1 || duration == 1) && !behavior.trim().is_empty() {
            let target_behavior = self.target_behavior();
            let target_behavior = (!target_behavior.trim().is_empty()).then_some(target_behavior);
            let flip_look = self
                .bordered
                .base
                .bool_attr(mascot, env, PARAM_TARGET_LOOK, false)?;
            mascot.request_affordance_arrival(AffordanceArrival {
                behavior,
                target_index: self.target_index,
                target_behavior,
                flip_look,
            });
        }
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.bordered.base.draggable(mascot, env)
    }
}

// =====================================================================
// ComplexMove（Java ComplexMove.java L21-265 相当・BorderedAction 派生）
// =====================================================================

/// Java `ComplexMove`: Move + Scan + Breed の複合。`Characteristics` 属性に
/// `Breed` / `Scan` を列挙すると機能有効化。非 Scan 時は `TargetX`/`TargetY`
/// （既定 `Integer.MAX_VALUE`）へ移動し、到達で自分（`Behaviour`）と相手
/// （`TargetBehaviour`）の Behavior を差し替える。
pub(crate) struct ComplexMoveAction {
    pub bordered: Bordered,
    delegate: BreedDelegate,
    has_turning: Option<bool>,
    turning: bool,
    affordance: String,
    target_index: Option<usize>,
    breed_enabled: bool,
    scan_enabled: bool,
}

impl ComplexMoveAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> ComplexMoveAction {
        ComplexMoveAction {
            bordered: Bordered::new(attrs, animations),
            delegate: BreedDelegate::new(),
            has_turning: None,
            turning: false,
            affordance: String::new(),
            target_index: None,
            breed_enabled: false,
            scan_enabled: false,
        }
    }

    fn behavior(&mut self) -> String {
        self.bordered
            .base
            .text_attr(PARAM_BEHAVIOR)
            .unwrap_or_default()
    }

    fn target_behavior(&mut self) -> String {
        self.bordered
            .base
            .text_attr(PARAM_TARGET_BEHAVIOR)
            .unwrap_or_default()
    }

    /// Java L62-79 逐語: `Characteristics` をカンマ区切りで読み Breed / Scan を立てる。
    fn parse_characteristics(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<(), ActionError> {
        self.breed_enabled = false;
        self.scan_enabled = false;
        let characteristics = self
            .bordered
            .base
            .text_attr("Characteristics")
            .unwrap_or_default();
        if !characteristics.is_empty() {
            for characteristic in characteristics.split(',') {
                if characteristic == super::CHARACTERISTIC_BREED {
                    self.breed_enabled = true;
                } else if characteristic == super::CHARACTERISTIC_SCAN {
                    self.scan_enabled = true;
                }
            }
        }
        if self.breed_enabled {
            self.delegate.init_scaling(mascot);
            self.delegate
                .validate_born_count(&mut self.bordered.base, mascot, env)?;
            self.delegate
                .validate_born_interval(&mut self.bordered.base, mascot, env)?;
        }
        Ok(())
    }
}

impl Action for ComplexMoveAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.has_turning = None;
        self.turning = false;
        self.bordered.init_common(mascot, env)?;
        self.parse_characteristics(mascot, env)?;
        if self.scan_enabled {
            // Java L77-87: cannot broadcast while scanning + 相手探索 + TargetX/Y 注入
            mascot.clear_affordances();
            self.affordance = self
                .bordered
                .base
                .text_attr("Affordance")
                .unwrap_or_default();
            self.target_index = super::find_affordance_target(env, &self.affordance);
            if let Some(anchor) =
                super::affordance_target_anchor(env, self.target_index, &self.affordance)
            {
                self.bordered
                    .base
                    .vars
                    .inject("TargetX", f64::from(anchor.0));
                self.bordered
                    .base
                    .vars
                    .inject("TargetY", f64::from(anchor.1));
            }
        }
        Ok(())
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L91-118
        if !self.bordered.base.base_has_next(mascot, env)? {
            return Ok(false);
        }
        if self.turning {
            return Ok(true);
        }
        if self.scan_enabled {
            return Ok(
                super::affordance_target_anchor(env, self.target_index, &self.affordance).is_some(),
            );
        }
        let target_x = self
            .bordered
            .base
            .num_attr(mascot, env, "TargetX", DEFAULT_TARGET_X)?;
        let target_y = self
            .bordered
            .base
            .num_attr(mascot, env, "TargetY", DEFAULT_TARGET_Y)?;
        let anchor = mascot.anchor();
        Ok((target_x != DEFAULT_TARGET_X && anchor.0 != target_x)
            || (target_y != DEFAULT_TARGET_Y && anchor.1 != target_y))
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.base.next_pre(mascot, env)?;
        // Java L122: super.tick()（border move）
        self.bordered.border_tick(mascot, env);
        // Java L124-127: cannot broadcast while scanning
        if self.scan_enabled {
            mascot.clear_affordances();
        }
        // Java L129-131: 境界外 → LostGround
        self.bordered.check_on_border(mascot, env)?;

        // Java L133-150: 目標決定（scan は相手 / 非 scan は属性）
        let target = if self.scan_enabled {
            let Some(anchor) =
                super::affordance_target_anchor(env, self.target_index, &self.affordance)
            else {
                return Ok(());
            };
            self.bordered
                .base
                .vars
                .inject("TargetX", f64::from(anchor.0));
            self.bordered
                .base
                .vars
                .inject("TargetY", f64::from(anchor.1));
            anchor
        } else {
            let target_x = self
                .bordered
                .base
                .num_attr(mascot, env, "TargetX", DEFAULT_TARGET_X)?;
            let target_y = self
                .bordered
                .base
                .num_attr(mascot, env, "TargetY", DEFAULT_TARGET_Y)?;
            (target_x, target_y)
        };

        // Java L152-157: turning / lookRight / down
        let down = mascot.anchor().1 < target.1;
        if mascot.anchor().0 != target.0 {
            let look_right = mascot.look_right();
            self.turning = has_turning_animation(&mut self.has_turning, &self.bordered)
                && (self.turning || (mascot.anchor().0 < target.0) != look_right);
            mascot.set_look_right(mascot.anchor().0 < target.0);
        }

        // Java L159-164: 転回アニメ完了
        let mut anim_index = select_turn_animation(
            TurnAnimMode::Automatic,
            &mut self.bordered,
            self.turning,
            mascot,
            env,
        )?;
        if self.turning {
            let duration = anim_index
                .map(|i| animation_duration(&self.bordered.base.animations[i]))
                .unwrap_or(0);
            if self.bordered.base.get_time(mascot) >= duration {
                self.turning = false;
                anim_index = select_turn_animation(
                    TurnAnimMode::Automatic,
                    &mut self.bordered,
                    self.turning,
                    mascot,
                    env,
                )?;
            }
        }

        // Java L167: animation.apply
        if let Some(index) = anim_index {
            let rel = self.bordered.base.get_time(mascot);
            if let Some(pose) = animation_pose_at(&self.bordered.base.animations[index], rel) {
                apply_pose(pose, mascot);
            }
        }

        // Java L169-182: overshoot クランプ（scan 中は既定値でも常にクランプ）
        if target.0 != DEFAULT_TARGET_X || self.scan_enabled {
            let (ax, ay) = mascot.anchor();
            let look_right = mascot.look_right();
            if (look_right && ax >= target.0) || (!look_right && ax <= target.0) {
                mascot.set_anchor((target.0, ay));
            }
        }
        if target.1 != DEFAULT_TARGET_Y || self.scan_enabled {
            let (ax, ay) = mascot.anchor();
            if (down && ay >= target.1) || (!down && ay <= target.1) {
                mascot.set_anchor((ax, target.1));
            }
        }

        // Java L184-187: interval frame 且つ turning 中でない 且つ enabled → 生む
        if self.breed_enabled
            && self
                .delegate
                .is_interval_frame(&mut self.bordered.base, mascot, env)?
            && !self.turning
            && self
                .delegate
                .is_enabled(&mut self.bordered.base, mascot, env)?
        {
            self.delegate.breed(&mut self.bordered.base, mascot, env)?;
        }

        // Java L189-206: 到達（両軸一致 && turning 中でない）→ Behavior 差し替え要求
        if !self.turning && mascot.anchor() == target {
            let behavior = self.behavior();
            let target_behavior = self.target_behavior();
            let flip_look = self
                .bordered
                .base
                .bool_attr(mascot, env, PARAM_TARGET_LOOK, false)?;
            mascot.request_affordance_arrival(AffordanceArrival {
                behavior,
                // 非 scan 時は相手が居ない（Java targetMascot == null）
                target_index: self.target_index.filter(|_| self.scan_enabled),
                target_behavior: Some(target_behavior),
                flip_look,
            });
        }
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.bordered.base.draggable(mascot, env)
    }
}
