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
use super::super::{EnvironmentView, Mascot, MascotContext, Rng};
use super::{Base, FallAction};
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

    /// Java hasTurningAnimation L128-133 逐語。
    pub(crate) fn has_turning_animation(&mut self) -> bool {
        if self.has_turning.is_none() {
            self.has_turning = Some(self.bordered.base.animations.iter().any(|a| a.is_turn));
        }
        self.has_turning.expect("just set")
    }

    /// Java Move.getAnimation L107-126 逐語（turning == animation.isTurn() &&
    /// isEffective のみ一致・（index, animation）を返す）。
    pub(crate) fn get_turning_animation(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<Option<usize>, ActionError> {
        for (index, animation) in self.bordered.base.animations.iter().enumerate() {
            if self.turning == animation.is_turn {
                let snapshot = mascot.eval_snapshot();
                let ctx = MascotContext {
                    snapshot: &snapshot,
                    env,
                };
                if animation_is_effective(animation, &mut self.bordered.base.vars, &ctx)? {
                    return Ok(Some(index));
                }
            }
        }
        Ok(None)
    }

    /// Java Move.tick L57-104 逐語。
    pub(crate) fn move_tick(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<(), ActionError> {
        self.bordered.border_tick(mascot, env); // super.tick()（L58）
        self.bordered.check_on_border(mascot, env)?; // L60-62

        let target_x = self
            .bordered
            .base
            .num_attr(mascot, env, "TargetX", DEFAULT_TARGET_X)?;
        let target_y = self
            .bordered
            .base
            .num_attr(mascot, env, "TargetY", DEFAULT_TARGET_Y)?;

        // ComplexMove.java L145-146 相当（design §1.10(f)・Java 逐語原則からの
        // 意図的差異・ユーザー決定 方針 (b)）: TargetX/TargetY を変数へ注入し
        // アニメ条件から参照可能にする。Java 本家の Move は putVariable しないため
        // TargetY 条件付き Move（資産 ClimbWall）は評価エラーになるが、ComplexMove
        // 相当の注入を MoveAction にも適用する。注入値 = 上記属性評価値
        //（属性無し時は DEFAULT_TARGET_X/Y）。get_turning_animation（アニメ条件評価）
        // より前に注入する。
        self.bordered
            .base
            .vars
            .inject("TargetX", f64::from(target_x));
        self.bordered
            .base
            .vars
            .inject("TargetY", f64::from(target_y));

        let mut down = false;

        // Java L69-75: 方向転換アニメ有効化 + 向き更新
        if target_x != DEFAULT_TARGET_X && mascot.anchor().0 != target_x {
            let look_right = mascot.look_right();
            self.turning = self.has_turning_animation()
                && (self.turning || (mascot.anchor().0 < target_x) != look_right);
            mascot.set_look_right(mascot.anchor().0 < target_x); // L73
        }
        if target_y != DEFAULT_TARGET_Y {
            down = mascot.anchor().1 < target_y; // L77
        }

        // Java L81-85: turning アニメ完了チェック（getTime >= duration → turning 終了）
        let mut anim_index = self.get_turning_animation(mascot, env)?;
        if self.turning {
            let dur = anim_index
                .map(|i| animation_duration(&self.bordered.base.animations[i]))
                .unwrap_or(0);
            if self.bordered.base.get_time(mascot) >= dur {
                self.turning = false;
                anim_index = self.get_turning_animation(mascot, env)?;
            }
        }

        // Java L88: getAnimation().apply(getMascot(), getTime())
        if let Some(index) = anim_index {
            let rel = self.bordered.base.get_time(mascot);
            if let Some(pose) = animation_pose_at(&self.bordered.base.animations[index], rel) {
                apply_pose(pose, mascot);
            }
        }

        // Java L90-103: overshoot クランプ
        if target_x != DEFAULT_TARGET_X {
            let (ax, ay) = mascot.anchor();
            let look_right = mascot.look_right();
            if (look_right && ax >= target_x) || (!look_right && ax <= target_x) {
                mascot.set_anchor((target_x, ay));
            }
        }
        if target_y != DEFAULT_TARGET_Y {
            let (ax, ay) = mascot.anchor();
            if (down && ay >= target_y) || (!down && ay <= target_y) {
                mascot.set_anchor((ax, target_y));
            }
        }
        Ok(())
    }
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
        // Java L36-54 逐語
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
        self.scaling = env.scaling(); // Java L45
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

pub(crate) struct BreedAction {
    pub bordered: Bordered,
    /// Java Delegate.scaling（Delegate L49）。
    scaling: f64,
}

impl BreedAction {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        animations: Vec<super::Animation>,
    ) -> BreedAction {
        BreedAction {
            bordered: Bordered::new(attrs, animations),
            scaling: 1.0,
        }
    }

    /// Java Delegate.isEnabled L59-63 逐語: BornTransient が true → transients 設定、
    /// それ以外は breeding 設定。
    fn is_enabled(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<bool, ActionError> {
        let born_transient = self.bordered.base.bool_attr(
            mascot,
            env,
            "BornTransient",
            BREED_DEFAULT_BORN_TRANSIENT,
        )?;
        Ok(if born_transient {
            env.transients_enabled()
        } else {
            env.breeding_allowed()
        })
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

    /// Java Delegate.breed L73-101 逐語。
    fn breed(&mut self, mascot: &mut Mascot, env: &dyn EnvironmentView) -> Result<(), ActionError> {
        let born_x = self
            .bordered
            .base
            .num_attr(mascot, env, "BornX", BREED_DEFAULT_BORN_X)?;
        let born_y = self
            .bordered
            .base
            .num_attr(mascot, env, "BornY", BREED_DEFAULT_BORN_Y)?;
        let born_mascot = self
            .bordered
            .base
            .text_attr("BornMascot")
            .unwrap_or_else(|| BREED_DEFAULT_BORN_MASCOT.to_string());
        let born_behavior = self
            .bordered
            .base
            .text_attr("BornBehaviour")
            .unwrap_or_else(|| BREED_DEFAULT_BORN_BEHAVIOR.to_string());
        let born_interval = self.bordered.base.num_attr(
            mascot,
            env,
            "BornInterval",
            BREED_DEFAULT_BORN_INTERVAL,
        )?;
        let born_count =
            self.bordered
                .base
                .num_attr(mascot, env, "BornCount", BREED_DEFAULT_BORN_COUNT)?;

        // Java Delegate.validateBornInterval L109-113 は本版では呼ばれない（dead）。
        // BornInterval は資産使用 0 件のため値の保持のみ。
        let _ = born_interval;

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

impl Action for BreedAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.bordered.init_common(mascot, env)?;
        // Java L154-155: initScaling + validateBornCount
        self.scaling = env.scaling();
        let born_count =
            self.bordered
                .base
                .num_attr(mascot, env, "BornCount", BREED_DEFAULT_BORN_COUNT)?;
        if born_count < 1 {
            // Java Delegate L103-107: VariableException 相当
            return Err(ActionError::Eval(
                super::super::super::config::script::EvalError {
                    expr: "BornCount".to_string(),
                    message: "BornCount must be positive".to_string(),
                },
            ));
        }
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
        if self.is_penultimate_frame(mascot, env)? && self.is_enabled(mascot, env)? {
            self.breed(mascot, env)?;
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
