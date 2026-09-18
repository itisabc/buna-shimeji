//! アクション実装 — design.md §1.8（タスク #7b・ユーザー承認 A 案）。
//!
//! Java の継承階層（ActionBase → InstantAction / BorderedAction → 各クラス）を
//! 「共通構造体 [`Base`] + 種別構造体 + [`ActionKind`] enum + match」で再現する
//! （継承階層を enum で再現・AGENTS.md §5-2）。ロジックは Java 正本
//! （.tmp/java-ref/action/*.java）を仕様として逐語移植する:
//!
//! - 共通契約 = [`Base`]（ActionBase.java L26-253）: 相対時刻
//!   `time = mascot.time − start_time`、next() 順序 = resetVariables→affordances
//!   更新→refreshHotspots→tick、属性既定値 Duration=i32::MAX / Condition=true /
//!   Draggable=true / Affordance=""
//! - BorderedAction 系（Animate/Stay/Move/ThrowIE/WalkWithIE/Breed）と FallWithIE
//!   は bordered.rs、ComplexAction 系は complex.rs
//! - stub 11 種: has_next=false で 1 tick も動かず即完了 + 警告ログ（§1.8(a)）
//! - 未知 Embedded FQN は fail-fast（Java ActionBuilder L194-206 踏襲）
//!
//! 構築 API は 2 経路（§1.8(j)）:
//! - [`create`]（Java ActionBuilder.buildAction switch L386-437 相当・直接構築）
//! - [`build_action`]（Java Configuration.buildAction(name, params) L419-427 相当・
//!   config 駆動 / Inline ネストと Ref マージ込み）
//!
//! 意図的差異（Java 一致検証時に差し引くこと）:
//! - Breed の出生は [`EnvironmentView::queue_spawn`] キュー + 次 tick 一括反映
//!   （Java は manager.add() 即時・design §1.8(f)）
//! - Dragged.tick の `refreshWorkArea()`（Java Dragged.java L69）は currentWorkArea
//!   をステートレス解決に置き換えるため no-op（design §1.7(e)-3・(V)）
//! - `${}` 文字列式（資産 0 件）は評価不能のため警告 + 既定値フォールバック
//!   （design §1.8(d)）。アニメ条件の評価エラーは refreshHotspots（クリア）
//!   と tick（伝播）で表面化させ、hasNext の duration 参照は最初のアニメの長さに
//!   フォールバックする（tests/action_test.rs 契約）

use super::behavior::{Action, ActionError, BehaviorError};
use super::{env, EnvironmentView, Mascot, MascotContext, Rng};
use crate::config::script::{EvalError, EvalValue, Variable};
use crate::config::{ActionDef, ActionsConfig, Animation, BorderType, VarMap};
use crate::render::imageset::{java_round, scale_pose};

pub mod bordered;
pub mod complex;
pub mod factory;

use bordered::{
    AnimateAction, BreedAction, FallWithIEAction, MoveAction, ScanMoveAction, StayAction,
    ThrowIEAction, TransformAction, WalkWithIEAction,
};
use complex::Complex;

// =====================================================================
// ActionKind / FQN 対応（design §1.8(a)）
// =====================================================================

/// アクション種別（Java クラス / Type 属性 1:1・基底クラスは variant 外の共通構造）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    // type 属性系（Java ActionBuilder TYPE_* 相当）
    Animate,
    Move,
    Stay,
    // Embedded 10（資産 actions.xml が参照する全クラスを 100% 補償）
    Fall,
    Jump,
    Dragged,
    Regist,
    Look,
    Offset,
    Breed,
    WalkWithIE,
    ThrowIE,
    FallWithIE,
    /// ScanMove（アフォーダンス探索移動・#32・デレマスしめじ v1.9 が使用）。
    /// Java は `Broadcast*` 4 種もクラスとして存在するが、いずれも
    /// Animate / Stay / Move / Jump を継承し override 0 個の空サブクラスのため
    /// variant を持たず [`fqn_to_kind`] で基底種別へ写す。
    ScanMove,
    // stub 11（資産外・has_next=false 即完了+警告）
    ScanJump,
    ScanInteract,
    ComplexMove,
    ComplexJump,
    BreedMove,
    BreedJump,
    Interact,
    SelfDestruct,
    Mute,
    MoveWithTurn,
    Turn,
    Transform,
    /// Sequence / Select（ComplexAction・build_action 経由で構築）
    Sequence,
    Select,
}

/// 境界種別（Java BORDERTYPE_CEILING/WALL/FLOOR 相当・getFloor/getWall/getCeiling
/// / getCeiling の呼び分け用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    // Fall / FallWithIE の床探索で Ceiling 判定は使用しない（株のスクリプト isOn
    // 経路は env.rs が直接処理するため下位の整理用）。
    #[allow(dead_code)]
    Ceiling,
    Wall,
    Floor,
}

/// Embedded の FQN → [`ActionKind`]（Java Class.forName 相当・未知は None）。
pub fn fqn_to_kind(fqn: &str) -> Option<ActionKind> {
    use ActionKind::*;
    Some(match fqn {
        "com.group_finity.mascot.action.Fall" => Fall,
        "com.group_finity.mascot.action.Jump" => Jump,
        "com.group_finity.mascot.action.Dragged" => Dragged,
        "com.group_finity.mascot.action.Regist" => Regist,
        "com.group_finity.mascot.action.Look" => Look,
        "com.group_finity.mascot.action.Offset" => Offset,
        "com.group_finity.mascot.action.Breed" => Breed,
        "com.group_finity.mascot.action.WalkWithIE" => WalkWithIE,
        "com.group_finity.mascot.action.ThrowIE" => ThrowIE,
        "com.group_finity.mascot.action.FallWithIE" => FallWithIE,
        "com.group_finity.mascot.action.ScanMove" => ScanMove,
        "com.group_finity.mascot.action.ScanJump" => ScanJump,
        "com.group_finity.mascot.action.ScanInteract" => ScanInteract,
        // Java の Broadcast 4 種は Animate / Stay / Move / Jump を継承し
        // メソッドを 1 つも override しない空サブクラス（放送実体は ActionBase の
        // Affordance 属性）。variant を持たず基底種別へ写す（デレマスしめじ v1.9 が
        // Broadcast を 6 箇所で使う）。
        "com.group_finity.mascot.action.Broadcast" => Animate,
        "com.group_finity.mascot.action.BroadcastStay" => Stay,
        "com.group_finity.mascot.action.BroadcastMove" => Move,
        "com.group_finity.mascot.action.BroadcastJump" => Jump,
        "com.group_finity.mascot.action.ComplexMove" => ComplexMove,
        "com.group_finity.mascot.action.ComplexJump" => ComplexJump,
        "com.group_finity.mascot.action.BreedMove" => BreedMove,
        "com.group_finity.mascot.action.BreedJump" => BreedJump,
        "com.group_finity.mascot.action.Interact" => Interact,
        "com.group_finity.mascot.action.SelfDestruct" => SelfDestruct,
        "com.group_finity.mascot.action.Mute" => Mute,
        "com.group_finity.mascot.action.MoveWithTurn" => MoveWithTurn,
        "com.group_finity.mascot.action.Turn" => Turn,
        "com.group_finity.mascot.action.Transform" => Transform,
        _ => return None,
    })
}

// =====================================================================
// 共通ロジック（ActionBase.java L26-253 相当）
// =====================================================================

/// 共通状態（Java `ActionBase` のフィールド相当）。
pub(crate) struct Base {
    /// 属性マップ（Java schema / params 相当）。
    pub attrs: VarMap,
    /// 構築時に scale 適用済み（scale_pose）のアニメ群。
    pub animations: Vec<Animation>,
    /// アクションローカル変数（Java VariableMap 相当・注入変数 + 式キャッシュ）。
    pub vars: crate::config::script::Variables,
    /// 相対時刻の基点（Java ActionBase.startTime）。
    pub start_time: i32,
}

impl Base {
    pub(crate) fn new(attrs: VarMap, animations: Vec<Animation>) -> Base {
        let mut vars = crate::config::script::Variables::new();
        // ActionReference の全属性を子アクションの識別子空間へ載せる
        // （Java ActionRef.java L66 / ActionBuilder.createVariables L486-507）。
        vars.set_attrs(attrs.clone());
        Base {
            attrs,
            animations,
            vars,
            start_time: 0,
        }
    }

    /// 相対時刻（Java L211-213: `mascot.getTime() - startTime` 逐語）。
    pub(crate) fn get_time(&self, mascot: &Mascot) -> i32 {
        mascot.time() - self.start_time
    }

    /// setTime（Java L215-217: `startTime = mascot.getTime() - time` 逐語）。
    pub(crate) fn set_time(&mut self, mascot: &Mascot, time: i32) {
        self.start_time = mascot.time() - time;
    }

    /// init 共通部分（Java L66-92 該当部）: setTime(0) + Variables 初期化（
    /// ${} キャッシュ全クリア）。アニメ条件は同一 `Variables` を共有するため
    /// vars.init() が animation.init() の一括相当。
    pub(crate) fn init(&mut self, mascot: &Mascot) {
        self.set_time(mascot, 0);
        self.vars.init();
    }

    /// Text 定数を VarMap から直接取り出す（design §1.8(d)。
    /// Java eval(name, String.class, null) 相当・スクリプト / 数値 / ブールは
    /// String キャスト不可 → 警告 + 既定値（None）フォールバック）。
    pub(crate) fn text_attr(&mut self, key: &str) -> Option<String> {
        use crate::config::script::ConstantValue;
        match self.attrs.get(key) {
            Some(Variable::Constant(ConstantValue::Text(text))) => Some(text.clone()),
            Some(_) => {
                log::warn!("attribute `{key}` cannot be evaluated as a string (using the default)");
                None
            }
            None => None,
        }
    }

    /// 数値属性（Java `eval(name, Number.class, default).intValue()` 相当）。
    pub(crate) fn num_attr(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
        key: &str,
        default: i32,
    ) -> Result<i32, ActionError> {
        if !self.attrs.contains_key(key) {
            return Ok(default);
        }
        let var = self.attrs.get(key).cloned().expect("checked above");
        self.eval_number(var, mascot, env, key)
            .map(crate::config::script::to_java_int)
    }

    /// 数値属性（f64 既定値バージョン）。
    pub(crate) fn f64_attr(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
        key: &str,
        default: f64,
    ) -> Result<f64, ActionError> {
        if !self.attrs.contains_key(key) {
            return Ok(default);
        }
        let var = self.attrs.get(key).cloned().expect("checked above");
        self.eval_number(var, mascot, env, key)
    }

    fn eval_number(
        &mut self,
        var: Variable,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
        key: &str,
    ) -> Result<f64, ActionError> {
        let snapshot = mascot.eval_snapshot();
        let ctx = MascotContext {
            snapshot: &snapshot,
            env,
        };
        match self.vars.eval(&var, &ctx)? {
            EvalValue::Number(n) => Ok(n),
            // Java (Number.class) キャスト失敗相当
            EvalValue::Bool(_) => Err(ActionError::Eval(EvalError {
                expr: key.to_string(),
                message: format!("attribute `{key}` cannot be evaluated as a number"),
            })),
        }
    }

    /// ブール属性（Java `eval(name, Boolean.class, default)` 相当）。
    pub(crate) fn bool_attr(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
        key: &str,
        default: bool,
    ) -> Result<bool, ActionError> {
        if !self.attrs.contains_key(key) {
            return Ok(default);
        }
        let var = self.attrs.get(key).cloned().expect("checked above");
        let snapshot = mascot.eval_snapshot();
        let ctx = MascotContext {
            snapshot: &snapshot,
            env,
        };
        match self.vars.eval(&var, &ctx)? {
            EvalValue::Bool(b) => Ok(b),
            // Java (Boolean.class) キャスト失敗相当
            EvalValue::Number(_) => Err(ActionError::Eval(EvalError {
                expr: key.to_string(),
                message: format!("attribute `{key}` cannot be evaluated as a boolean"),
            })),
        }
    }

    /// Duration 属性（既定 i32::MAX = Integer.MAX_VALUE・Java L223-225）。
    pub(crate) fn get_duration(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<i32, ActionError> {
        self.num_attr(mascot, env, "Duration", i32::MAX)
    }

    /// Condition 属性（既定 true・Java L227-229）。
    pub(crate) fn is_effective(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<bool, ActionError> {
        self.bool_attr(mascot, env, "Condition", true)
    }

    /// Draggable 属性（既定 true・Java L231-233）。
    pub(crate) fn draggable(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<bool, ActionError> {
        self.bool_attr(mascot, env, "Draggable", true)
    }

    /// Java ActionBase.hasNext L95-97 逐語: `mascot != null &&
    /// getTime() < getDuration() && isEffective()`（mascot は常に非 null 相当のため
    /// 1 項目省略）。
    pub(crate) fn base_has_next(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<bool, ActionError> {
        let within = self.get_time(mascot) < self.get_duration(mascot, env)?;
        Ok(within && self.is_effective(mascot, env)?)
    }

    /// 条件一致する最初のアニメ（Java getAnimation L165-180 相当・
    /// 条件評価例外 = Err 伝播）。
    pub(crate) fn get_animation(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<Option<&Animation>, ActionError> {
        for animation in &self.animations {
            let snapshot = mascot.eval_snapshot();
            let ctx = MascotContext {
                snapshot: &snapshot,
                env,
            };
            if crate::mascot::animation::animation_is_effective(animation, &mut self.vars, &ctx)? {
                return Ok(Some(animation));
            }
        }
        Ok(None)
    }

    /// Java hasNext のアニメ長参照。評価エラー時は最初のアニメの長さに
    /// フォールバック（意図的差異・refreshHotspots（クリア）/ tick（伝播）で
    /// 表面化させる）。
    pub(crate) fn tolerant_anim_duration(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<i32, ActionError> {
        // 評価エラーは最初のアニメの長さにフォールバック（Java は hasNext 内で
        // 例外をスローするが、本実装は refreshHotspots（クリア）/ tick（伝播）で
        // 表面化させる）
        let effective = match self.get_animation(mascot, env) {
            Ok(Some(anim)) => Some(crate::mascot::animation::animation_duration(anim)),
            Ok(None) | Err(_) => None,
        };
        if let Some(d) = effective {
            return Ok(d);
        }
        if let Some(first) = self.animations.first() {
            return Ok(crate::mascot::animation::animation_duration(first));
        }
        Err(ActionError::Eval(EvalError {
            expr: "(Animation)".to_string(),
            message: "no effective animation exists (Java stops as if by a null reference)"
                .to_string(),
        }))
    }

    /// Java next() 共通部分 L100-119 逐語。
    ///
    /// 1) resetVariables: #{} / アニメ条件キャッシュクリア（本実装ではアニメ条件も
    ///    vars を共有するため resetValues = #{..} のみ再評価相当）
    /// 2) affordances: clear（非空時）+ Affordance 属性追加（trim 非空時）
    /// 3) refreshHotspots: Java ActionBase.refreshHotspots（L140-155）逐語
    ///    （[`Base::refresh_hotspots`]・アニメ条件評価 / 失敗時は catch 準拠の握り）
    ///
    /// tick 本体は各実装がこの直後に続ける（Java abstract tick L138）。
    /// refreshHotspots をオーバーライドする Dragged は
    /// [`Base::next_pre_clear_hotspots`] を使う。
    pub(crate) fn next_pre(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<(), ActionError> {
        self.next_pre_common(mascot);
        self.refresh_hotspots(mascot, env);
        Ok(())
    }

    /// Java next() 共通部分 + `Dragged.refreshHotspots` オーバーライド相当
    /// （Dragged.java L112-122 逐語: clearHotspots のみ・アニメ条件評価しない。
    /// 評価しないため環境は不要）。
    pub(crate) fn next_pre_clear_hotspots(
        &mut self,
        mascot: &mut Mascot,
    ) -> Result<(), ActionError> {
        self.next_pre_common(mascot);
        mascot.set_hotspots(Vec::new());
        Ok(())
    }

    /// next() 共通の resetVariables + affordances 部分（ActionBase L105-113 逐語）。
    fn next_pre_common(&mut self, mascot: &mut Mascot) {
        self.vars.reset_values();

        // Clear affordances（L108-113）
        if !mascot.affordances().is_empty() {
            mascot.clear_affordances();
        }
        let affordance = self.text_attr("Affordance").unwrap_or_default(); // 既定 ""
        if !affordance.trim().is_empty() {
            mascot.add_affordance(affordance);
        }
    }

    /// Java ActionBase.refreshHotspots L140-155 逐語。
    ///
    /// ```java
    /// try {
    ///   Animation animation = getAnimation();   // 自身の variables（注入値含む）で条件評価
    ///   if (animation != null) {
    ///     getMascot().setHotspots(animation.getHotspots());
    ///   }
    /// } catch (VariableException e) {
    ///   getMascot().clearHotspots();            // log 無し
    /// }
    /// ```
    ///
    /// アニメ条件の評価は自身の variables（[`Base::vars`]. 注入値含む）+
    /// [`MascotContext`] で行う（getAnimation L165-180 と同一経路・getVariables()
    /// 逐語）。評価は warn 無しの `Variables::eval_quiet` を使う（Java catch は
    /// 評価失敗を log 無しで握るため。tick 内のアニメ選択は warn ありの eval を
    /// 維持する）。hotspot 供給値は資産 hotspot 0 件のため空 Vec（既存設計維持）。
    fn refresh_hotspots(&mut self, mascot: &mut Mascot, env: &dyn EnvironmentView) {
        let snapshot = mascot.eval_snapshot();
        let ctx = MascotContext {
            snapshot: &snapshot,
            env,
        };
        let outcome = (|| -> Result<bool, EvalError> {
            for animation in &self.animations {
                match &animation.condition {
                    Some(cond) => match self.vars.eval_quiet(cond, &ctx)? {
                        EvalValue::Bool(true) => return Ok(true),
                        EvalValue::Bool(false) => continue,
                        // Java の (Boolean) キャスト失敗相当
                        EvalValue::Number(_) => {
                            return Err(EvalError {
                                expr: "Animation".to_string(),
                                message: "animation condition must be a boolean".to_string(),
                            })
                        }
                    },
                    None => return Ok(true),
                }
            }
            Ok(false)
        })();
        match outcome {
            // 有効アニメ有り → setHotspots（資産 hotspot 0 件のため空供給）
            Ok(true) => mascot.set_hotspots(Vec::new()),
            // Java catch 相当: 条件評価例外時 clearHotspots（log 無し）
            Err(_) => mascot.set_hotspots(Vec::new()),
            // Java L146-149: animation == null → hotspots 保持
            Ok(false) => {}
        }
    }

    /// 条件一致する最初のアニメの rel 時刻フレームを適用する
    /// （Java `getAnimation().apply(mascot, getTime())` 相当）。
    pub(crate) fn apply_effective_animation(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
    ) -> Result<(), ActionError> {
        let rel = self.get_time(mascot);
        let anim = self.get_animation(mascot, env)?.ok_or_else(|| {
            ActionError::Eval(EvalError {
                expr: "(Animation)".to_string(),
                message: "no applicable animation exists (Java's equivalent of a null reference)"
                    .to_string(),
            })
        })?;
        if let Some(pose) = crate::mascot::animation::animation_pose_at(anim, rel) {
            crate::mascot::animation::apply_pose(pose, mascot);
        }
        Ok(())
    }
}

// =====================================================================
// stub 17 種（資産外・design §1.8(a)）
// =====================================================================

/// stub: has_next=false で 1 tick も動かず即完了 + 警告ログ。
/// Java 版も資産 XML 未参照のクラスはロードされず実行パスに乗らないため
/// （§1.8(a)）、実行系としての挙動は一致する。
pub(crate) struct StubAction {
    kind: ActionKind,
    base: Base,
}

impl Action for StubAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.base.init(mascot);
        log::warn!(
            "action kind `{:?}` is not implemented in this version; completing immediately",
            self.kind
        );
        Ok(())
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // 即完了（1 tick も動かない）
        let _ = self.base.base_has_next(mascot, env)?;
        Ok(false)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.base.next_pre(mascot, env)
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.base.draggable(mascot, env)
    }
}

impl StubAction {
    pub(crate) fn new(kind: ActionKind, attrs: VarMap) -> StubAction {
        StubAction {
            kind,
            base: Base::new(attrs, Vec::new()),
        }
    }
}

// =====================================================================
// Jump（Java Jump.java L20-110 相当）
// =====================================================================

pub(crate) struct JumpAction {
    base: Base,
    scaling: f64,
}

impl JumpAction {
    pub(crate) fn new(attrs: VarMap, animations: Vec<Animation>) -> JumpAction {
        JumpAction {
            base: Base::new(attrs, animations),
            scaling: 1.0,
        }
    }
}

impl Action for JumpAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.base.init(mascot);
        self.scaling = mascot.scale(); // Java L47
        Ok(())
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L51-64 逐語: super.hasNext() && distance != 0
        if !self.base.base_has_next(mascot, env)? {
            return Ok(false);
        }
        let target_x = self.base.num_attr(mascot, env, "TargetX", 0)?;
        let target_y = self.base.num_attr(mascot, env, "TargetY", 0)?;
        let (ax, ay) = mascot.anchor();
        let distance_x = f64::from(target_x - ax);
        // Java L60 放物線距離式
        let distance_y = f64::from(target_y - ay) - distance_x.abs() / 2.0;
        let distance = (distance_x * distance_x + distance_y * distance_y).sqrt();
        Ok(distance != 0.0)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.base.next_pre(mascot, env)?;

        // Java L69-74: setLookRight(anchor.x < targetX)（anchor.x != targetX のとき）
        let target_x = self.base.num_attr(mascot, env, "TargetX", 0)?;
        let target_y = self.base.num_attr(mascot, env, "TargetY", 0)?;
        let velocity = self.base.f64_attr(mascot, env, "VelocityParam", 20.0)? * self.scaling; // L81
        if mascot.anchor().0 != target_x {
            mascot.set_look_right(mascot.anchor().0 < target_x);
        }

        let (ax, ay) = mascot.anchor();
        let distance_x = f64::from(target_x - ax);
        // Java L77 逐語: distanceY = targetY − anchor.y − |distanceX| / 2
        let distance_y = f64::from(target_y - ay) - distance_x.abs() / 2.0;
        let distance = (distance_x * distance_x + distance_y * distance_y).sqrt();

        if distance != 0.0 {
            // Java L84-88: velocityX / velocityY 注入
            let velocity_x = velocity * distance_x / distance;
            let velocity_y = velocity * distance_y / distance;
            self.base.vars.inject("VelocityX", velocity_x);
            self.base.vars.inject("VelocityY", velocity_y);
            // Java L90 逐語: translate((int) Math.round(velocityX), ...)
            let (ax, ay) = mascot.anchor();
            mascot.set_anchor((ax + java_round(velocity_x), ay + java_round(velocity_y)));
            // Java L91 逐語: アニメ適用は distance != 0 の内側
            self.base.apply_effective_animation(mascot, env)?;
        }

        // Java L94-96: distance <= velocity → target にスナップ
        if distance <= velocity {
            mascot.set_anchor((target_x, target_y));
        }
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.base.draggable(mascot, env)
    }
}

// =====================================================================
// Fall（Java Fall.java L22-174 相当）
// =====================================================================

/// Fall: 重力式 + stepwise 衝突ループ。getFloor(true) / getWall(true) は
/// 毎呼出し fresh 解決（ignoreSeparator = true）。
pub(crate) struct FallAction {
    pub base: Base,
    velocity_x: f64,
    velocity_y: f64,
    /// 余剰積算（Java modX / modY）。
    mod_x: f64,
    mod_y: f64,
    scaling: f64,
}

impl FallAction {
    pub(crate) fn new(attrs: VarMap, animations: Vec<Animation>) -> FallAction {
        FallAction {
            base: Base::new(attrs, animations),
            velocity_x: 0.0,
            velocity_y: 0.0,
            mod_x: 0.0,
            mod_y: 0.0,
            scaling: 1.0,
        }
    }
}

/// 境界上判定ヘルパ（getFloor / getWall / getCeiling 相当・
/// ignoreSeparator 引数付き・fresh resolve）。
fn border_on(
    env: &dyn EnvironmentView,
    kind: Kind,
    ignore_separator: bool,
    look_right: bool,
    anchor: (i32, i32),
) -> bool {
    use crate::mascot::env::BorderKind;
    let java_kind = match kind {
        Kind::Floor => BorderKind::Floor,
        Kind::Wall => BorderKind::Wall,
        Kind::Ceiling => BorderKind::Ceiling,
    };
    env::resolve_border(env, java_kind, anchor, look_right, ignore_separator).is_some()
}

/// Fall 本体の tick（Java Fall.tick L100-153 逐語）。FallWithIE から super.tick()
/// 相当で使用。
pub(crate) fn fall_tick(
    a: &mut FallAction,
    mascot: &mut Mascot,
    env: &dyn EnvironmentView,
) -> Result<(), ActionError> {
    // Java L102-104: velocityX != 0 → setLookRight(velocityX > 0)
    if a.velocity_x != 0.0 {
        mascot.set_look_right(a.velocity_x > 0.0);
    }

    // Java L107-108 逐語（velocity は init で scale 済みのため gravity のみ scale）
    let rx = a.base.f64_attr(mascot, env, "RegistanceX", 0.05)?;
    let ry = a.base.f64_attr(mascot, env, "RegistanceY", 0.1)?;
    let gravity = a.base.f64_attr(mascot, env, "Gravity", 2.0)?;
    a.velocity_x -= a.velocity_x * rx; // L107
    a.velocity_y = a.velocity_y - a.velocity_y * ry + gravity * a.scaling; // L108
    let vx = a.velocity_x;
    let vy = a.velocity_y;

    // Java L110-111: VelocityX / VelocityY 注入
    a.base.vars.inject("VelocityX", vx);
    a.base.vars.inject("VelocityY", vy);

    // Java L113-121 逐語: modX / modY 余剰累積 + dx/dy 丸め
    a.mod_x += vx % 1.0;
    a.mod_y += vy % 1.0;
    let dx = java_round(vx + a.mod_x);
    let dy = java_round(vy + a.mod_y);
    a.mod_x %= 1.0;
    a.mod_y %= 1.0;

    // Java L123: dev = max(1, max(|dx|, |dy|))
    let dev = 1.max(dx.abs()).max(dy.abs());

    // Java L129-149: stepwise 衝突ループ
    let (anchor_x, anchor_y) = mascot.anchor();
    let look_right = mascot.look_right();
    'outer: for i in 0..=dev {
        let x = anchor_x + dx * i / dev; // Java int 除算（0 向け切り捨て）= Rust i32 同値
        let y = anchor_y + dy * i / dev;

        if dy > 0 {
            // HACK: Windows は窓がよく動くため高頻度に床を確認（Java コメント踏襲）
            for j in -80..=0 {
                let point = (x, y + j);
                // getFloor(true).isOn(anchor) 相当（fresh resolve・Java は
                // setLocation → isOn の順で毎回 anchor を更新する）
                mascot.set_anchor(point);
                if border_on(env, Kind::Floor, true, look_right, point) {
                    break 'outer;
                }
            }
        } else {
            mascot.set_anchor((x, y));
        }
        // getWall(true).isOn(anchor) 相当
        let current = mascot.anchor();
        if border_on(env, Kind::Wall, true, look_right, current) {
            break;
        }
    }

    // Java L152: getAnimation().apply(mascot, getTime())
    a.base.apply_effective_animation(mascot, env)
}

impl Action for FallAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.base.init(mascot);
        // Java L62: scaling（FIXME コメント付きの無条件 scale 適用・L82-83 逐語）
        self.scaling = mascot.scale();
        // Java L82-83: getInitialVx()/getInitialVy() は intValue() で切り捨ててから scaling 乗算
        self.velocity_x = f64::from(crate::config::script::to_java_int(self.base.f64_attr(
            mascot,
            env,
            "InitialVX",
            0.0,
        )?)) * self.scaling;
        self.velocity_y = f64::from(crate::config::script::to_java_int(self.base.f64_attr(
            mascot,
            env,
            "InitialVY",
            0.0,
        )?)) * self.scaling;
        Ok(())
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L87-97 逐語: super.hasNext() &&
        //   (velocityY < 0 || !getFloor().isOn(anchor)) && !getWall().isOn(anchor)
        if !self.base.base_has_next(mascot, env)? {
            return Ok(false);
        }
        let anchor = mascot.anchor();
        let look_right = mascot.look_right();
        // 「上向き速度があれば床の有無を無視する」（Java コメント踏襲）。
        // 无引数オーバーロード相当（ignoreSeparator = false）
        let up_or_not_floor =
            self.velocity_y < 0.0 || !border_on(env, Kind::Floor, false, look_right, anchor);
        Ok(up_or_not_floor && !border_on(env, Kind::Wall, false, look_right, anchor))
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.base.next_pre(mascot, env)?;
        fall_tick(self, mascot, env)
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.base.draggable(mascot, env)
    }
}

// =====================================================================
// Dragged（Java Dragged.java L22-135 相当）
// =====================================================================

pub(crate) struct DraggedAction {
    base: Base,
    foot_x: f64,
    foot_dx: f64,
    time_to_resist: i32,
    scaling: f64,
}

impl DraggedAction {
    pub(crate) fn new(attrs: VarMap, animations: Vec<Animation>) -> DraggedAction {
        DraggedAction {
            base: Base::new(attrs, animations),
            foot_x: 0.0,
            foot_dx: 0.0,
            time_to_resist: 250,
            scaling: 1.0,
        }
    }
}

impl Action for DraggedAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.base.init(mascot);
        // Java L54: scaling
        self.scaling = mascot.scale();
        // Java L56 逐語: footX = cursor.x + (int) Math.round(offsetX * scaling)
        let offset_x = self.base.num_attr(mascot, env, "OffsetX", 0)?;
        self.foot_x = f64::from(env.cursor().x + java_round(f64::from(offset_x) * self.scaling));
        self.time_to_resist = 250; // Java L57
        Ok(())
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L62 逐語: super.hasNext() && getTime() < timeToResist
        Ok(self.base.base_has_next(mascot, env)? && self.get_time(mascot) < self.time_to_resist)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        // Java L100-119 + L112-122 逐語: refreshHotspots をオーバーライドするため
        // アニメ条件評価は行わず clearHotspots のみ（"action does not support
        // hotspots"・Dragged.java L112-122 逐語）。
        self.base.next_pre_clear_hotspots(mascot)?;

        // Java L67-68 逐語
        mascot.set_look_right(false);
        mascot.set_dragging(true);
        // Java L69: refreshWorkArea() の唯一の呼び出し元と確定（(V)）。
        // currentWorkArea はステートレス fresh 解決のため no-op（design §1.7(e)-3）
        no_op_refresh_work_area();

        let cursor = env.cursor(); // Java L71

        // Java L73-74 逐語: offset = round(OffsetX/Y * scaling)
        let mut offset_x =
            java_round(f64::from(self.num_attr(mascot, env, "OffsetX", 0)?) * self.scaling);
        let mut offset_y =
            java_round(f64::from(self.num_attr(mascot, env, "OffsetY", 120)?) * self.scaling);
        if self
            .base
            .text_attr("OffsetType")
            .unwrap_or_else(|| "ImageAnchor".to_string())
            == "Origin"
        {
            // Java L75-78 逐語: image.center 基準に差分を取る。
            // この時点では当該 tick の apply（L99）未実行 → 直前 tick の画像
            // center が基準になる。image None なら補正をスキップし raw
            // offset×scaling を使用（Java は null 画像で NPE・防御的契約）。
            if let Some(img) = mascot.image() {
                let center = img.center;
                offset_x = center.0 - offset_x;
                offset_y = center.1 - offset_y;
            }
        }

        // Java L80-82 逐語: anchor 未到達のときは setTime(0) 抵抗リセット
        if (cursor.x + offset_x - mascot.anchor().0).abs() >= 5 {
            self.base.set_time(mascot, 0);
        }

        // Java L91-96: footDx 減衰式 + FootDX / FootX 注入（アニメ選択に先立つ）
        let new_x = cursor.x; // Java L84
        self.foot_dx = (self.foot_dx + (f64::from(new_x) - self.foot_x) * 0.1) * 0.8;
        self.foot_x += self.foot_dx;
        self.base.vars.inject("FootDX", self.foot_dx);
        self.base.vars.inject("FootX", self.foot_x);

        // Java L99: アニメ適用（FootDX 注入済みの変数で選択される）
        self.base.apply_effective_animation(mascot, env)?;

        // Java L102 逐語: anchor = cursor + offset
        mascot.set_anchor((cursor.x + offset_x, cursor.y + offset_y));

        // Java L107-109 逐語: getTime() == timeToResist - 1 && Math.random() >= 0.1 →
        // timeToResist++。短絡のため条件成立時のみ rng を消費（design §1.8(e)）
        if self.get_time(mascot) == self.time_to_resist - 1 && rng.unit() >= 0.1 {
            self.time_to_resist += 1;
        }
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.base.draggable(mascot, env)
    }
}

impl DraggedAction {
    fn get_time(&self, mascot: &Mascot) -> i32 {
        self.base.get_time(mascot)
    }

    fn num_attr(
        &mut self,
        mascot: &Mascot,
        env: &dyn EnvironmentView,
        key: &str,
        default: i32,
    ) -> Result<i32, ActionError> {
        self.base.num_attr(mascot, env, key, default)
    }
}

/// Dragged.tick の refreshWorkArea() 呼び出し点保持用 no-op。
fn no_op_refresh_work_area() {}

// =====================================================================
// Regist（Java Regist.java L22-99 相当）
// =====================================================================

pub(crate) struct RegistAction {
    base: Base,
    scaling: f64,
}

impl RegistAction {
    pub(crate) fn new(attrs: VarMap, animations: Vec<Animation>) -> RegistAction {
        RegistAction {
            base: Base::new(attrs, animations),
            scaling: 1.0,
        }
    }

    /// Java Regist L50-53 / L92-94 逐語（hasNext / 共通の offsetX 算出）。
    /// Origin 時は image.center.x 基準（Java L52）。
    /// image None なら補正をスキップし raw offset×scaling を使用
    ///（Java は null 画像で NPE・Dragged と同一の防御的契約）。
    fn offset_x(&mut self, mascot: &Mascot, env: &dyn EnvironmentView) -> Result<i32, ActionError> {
        let offset_x = self.base.num_attr(mascot, env, "OffsetX", 0)?;
        let scaled = java_round(f64::from(offset_x) * self.scaling);
        if self
            .base
            .text_attr("OffsetType")
            .unwrap_or_else(|| "ImageAnchor".to_string())
            == "Origin"
        {
            if let Some(img) = mascot.image() {
                return Ok(img.center.0 - scaled);
            }
        }
        Ok(scaled)
    }
}

impl Action for RegistAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.base.init(mascot);
        self.scaling = mascot.scale(); // Java L41
        Ok(())
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java L45-62 逐語: super.hasNext() && |cursor.x - anchor.x + offsetX| < 5
        //（Java FIXME: y 座標は無視の quirk コメント踏襲）
        if !self.base.base_has_next(mascot, env)? {
            return Ok(false);
        }
        let offset_x = self.offset_x(mascot, env)?;
        let cursor_x = env.cursor().x;
        Ok((cursor_x - mascot.anchor().0 + offset_x).abs() < 5)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        // Java Regist.java L81-92 逐語: refreshHotspots をオーバーライドし
        // clearHotspots のみ（"action does not support hotspots"・アニメ条件評価しない）。
        // Java で refreshHotspots をオーバーライドするのは Dragged と Regist の 2 種のみ。
        self.base.next_pre_clear_hotspots(mascot)?;

        // Java L66 逐語
        mascot.set_dragging(true);

        // Java L69: アニメ適用
        self.base.apply_effective_animation(mascot, env)?;

        // Java L71-77 逐語: 期間経過で rng で向き択一 → LostGround
        let rel = self.base.get_time(mascot);
        let dur = self.base.tolerant_anim_duration(mascot, env)?;
        if rel + 1 >= dur {
            // Java L74 逐語: setLookRight(Math.random() < 0.5)・rng 1 回消費（§1.8(e)）
            let look_right = rng.unit() < 0.5;
            mascot.set_look_right(look_right);
            // Java L76 逐語: LostGroundException
            return Err(ActionError::LostGround);
        }
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.base.draggable(mascot, env)
    }
}

// =====================================================================
// Look（Java Look.java L25-33・InstantAction 相当）
// =====================================================================

pub(crate) struct LookAction {
    base: Base,
}

impl LookAction {
    pub(crate) fn new(attrs: VarMap) -> LookAction {
        LookAction {
            base: Base::new(attrs, Vec::new()),
        }
    }
}

impl Action for LookAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.base.init(mascot);
        // Java InstantAction.init L26-32: base hasNext の間 apply() する
        if self.base.base_has_next(mascot, env)? {
            // Java Look.java L27/L30-32: setLookRight(isLookRight())・
            // LookRight 属性が無い場合は現在値を反転（Java 既定 !mascot.isLookRight()）
            let look_right = self
                .base
                .bool_attr(mascot, env, "LookRight", !mascot.look_right())?;
            mascot.set_look_right(look_right);
        }
        Ok(())
    }

    fn has_next(
        &mut self,
        _: &mut Mascot,
        _: &dyn EnvironmentView,
        _: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        // Java InstantAction.hasNext は final false（L37-39）
        Ok(false)
    }

    fn next(
        &mut self,
        _: &mut Mascot,
        _: &dyn EnvironmentView,
        _: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        // InstantAction.tick = no-op（Java L42-43）
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.base.draggable(mascot, env)
    }
}

// =====================================================================
// Offset（Java Offset.java L39-58・InstantAction 相当）
// =====================================================================

pub(crate) struct OffsetAction {
    base: Base,
}

impl OffsetAction {
    pub(crate) fn new(attrs: VarMap) -> OffsetAction {
        OffsetAction {
            base: Base::new(attrs, Vec::new()),
        }
    }
}

impl Action for OffsetAction {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.base.init(mascot);
        if self.base.base_has_next(mascot, env)? {
            let x = self.base.num_attr(mascot, env, "X", 0)?;
            let y = self.base.num_attr(mascot, env, "Y", 0)?;
            let (ax, ay) = mascot.anchor();
            // Java L55-57: scaling を使わない quirk（Java コメント L41-54 踏襲）
            mascot.set_anchor((ax + x, ay + y));
        }
        Ok(())
    }

    fn has_next(
        &mut self,
        _: &mut Mascot,
        _: &dyn EnvironmentView,
        _: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        Ok(false)
    }

    fn next(
        &mut self,
        _: &mut Mascot,
        _: &dyn EnvironmentView,
        _: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        Ok(())
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.base.draggable(mascot, env)
    }
}

// =====================================================================
// 構築 API 経路 1: create（Java ActionBuilder.buildAction switch L386-437 相当）
// =====================================================================

/// 直接構築（tester pin）。アニメの Pose に構築時 scale 変換（Java
/// AnimationBuilder L206-211 相当・[`scale_pose`]）を適用してから種別を組み立てる。
pub fn create(
    kind: ActionKind,
    attrs: &VarMap,
    animations: Vec<Animation>,
    scale: f64,
) -> Result<Box<dyn Action>, BehaviorError> {
    let scaled = animations
        .into_iter()
        .map(|anim| Animation {
            condition: anim.condition.clone(),
            is_turn: anim.is_turn,
            poses: anim
                .poses
                .iter()
                .map(|pose| scale_pose(pose, scale))
                .collect(),
        })
        .collect();

    let action: Box<dyn Action> = match kind {
        ActionKind::Animate => Box::new(AnimateAction::new(attrs.clone(), scaled)),
        ActionKind::Move => Box::new(MoveAction::new(attrs.clone(), scaled)),
        ActionKind::Stay => Box::new(StayAction::new(attrs.clone(), scaled)),
        ActionKind::Fall => Box::new(FallAction::new(attrs.clone(), scaled)),
        ActionKind::Jump => Box::new(JumpAction::new(attrs.clone(), scaled)),
        ActionKind::Dragged => Box::new(DraggedAction::new(attrs.clone(), scaled)),
        ActionKind::Regist => Box::new(RegistAction::new(attrs.clone(), scaled)),
        ActionKind::Look => Box::new(LookAction::new(attrs.clone())),
        ActionKind::Offset => Box::new(OffsetAction::new(attrs.clone())),
        ActionKind::Breed => Box::new(BreedAction::new(attrs.clone(), scaled)),
        ActionKind::WalkWithIE => Box::new(WalkWithIEAction::new(attrs.clone(), scaled)),
        ActionKind::ThrowIE => Box::new(ThrowIEAction::new(attrs.clone(), scaled)),
        ActionKind::FallWithIE => Box::new(FallWithIEAction::new(attrs.clone(), scaled)),
        ActionKind::ScanMove => Box::new(ScanMoveAction::new(attrs.clone(), scaled)),
        // Transform（Java Transform.java・Animate 派生・#33）
        ActionKind::Transform => Box::new(TransformAction::new(attrs.clone(), scaled)),
        // stub 11 種
        ActionKind::ScanJump
        | ActionKind::ScanInteract
        | ActionKind::ComplexMove
        | ActionKind::ComplexJump
        | ActionKind::BreedMove
        | ActionKind::BreedJump
        | ActionKind::Interact
        | ActionKind::SelfDestruct
        | ActionKind::Mute
        | ActionKind::MoveWithTurn
        | ActionKind::Turn => Box::new(StubAction::new(kind, attrs.clone())),
        // ComplexAction 系は子アクションが必要なため直接構築不可
        ActionKind::Sequence | ActionKind::Select => {
            return Err(BehaviorError::UnknownBehavior(
                "Sequence/Select have child actions; build them via build_action".to_string(),
            ));
        }
    };
    Ok(action)
}

// =====================================================================
// 構築 API 経路 2: build_action（Java Configuration.buildAction L419-427 相当）
// =====================================================================

/// config 駆動構築（tester pin）。
///
/// - caller 側パラメータ `extra` は定義自身の attrs を上書きする
///   （Java ActionBuilder.createVariables L486-507: 既存を先に put、引数を後に put）
/// - Inline 子は Java 同様に空パラメータで構築（ActionBuilder.createActions
///   L445-455 `buildAction(Map.of())` 逐語）
/// - Ref 子は「参照先 ActionDef の attrs + Ref 自身の attrs」でマージし
///   **Ref 側優先**（Java ActionRef.buildAction L113-126 逐語）
pub fn build_action(
    actions: &ActionsConfig,
    name: &str,
    extra: &VarMap,
    scale: f64,
) -> Result<Box<dyn Action>, BehaviorError> {
    let Some(def) = actions.actions.get(name) else {
        // Java: NoCorrespondingActionFoundErrorMessage 相当
        return Err(BehaviorError::UnknownBehavior(name.to_string()));
    };
    build_def(actions, def, extra, scale)
}

/// 上書き merge: `extra`（caller 側 / Ref 側のパラメータ）が既存 attrs に勝つ
///（Java createVariables L488-505 / ActionRef.buildAction L113-126 逐語）。
pub(crate) fn merge_caller_over(attrs: &mut VarMap, extra: &VarMap) {
    for (key, value) in extra {
        attrs.insert(key.clone(), value.clone());
    }
}

/// Java は BorderType も params の 1 個（BorderedAction.init で eval キー）なので、
/// 構造体フィールドを attrs へ文字属性として注入する。
pub(crate) fn inject_border_type(attrs: &mut VarMap, border: Option<BorderType>) {
    if let Some(border) = border {
        let text = match border {
            BorderType::Floor => "Floor",
            BorderType::Wall => "Wall",
            BorderType::Ceiling => "Ceiling",
        };
        attrs.insert("BorderType".to_string(), Variable::parse(text));
    }
}

/// 定義の attrs 抜き出し（全 variant 共通）。
pub(crate) fn def_attrs(def: &ActionDef) -> &VarMap {
    match def {
        ActionDef::Embedded { attrs, .. }
        | ActionDef::Stay { attrs, .. }
        | ActionDef::Move { attrs, .. }
        | ActionDef::Animate { attrs, .. }
        | ActionDef::Sequence { attrs, .. }
        | ActionDef::Select { attrs, .. } => attrs,
    }
}

/// ActionDef 1 定義からアクションを組み立てる（Java ActionBuilder.buildAction 相当）。
pub(crate) fn build_def(
    actions: &ActionsConfig,
    def: &ActionDef,
    extra: &VarMap,
    scale: f64,
) -> Result<Box<dyn Action>, BehaviorError> {
    match def {
        ActionDef::Embedded {
            class,
            border,
            attrs: _,
            animations,
        } => {
            let mut attrs = def_attrs(def).clone();
            inject_border_type(&mut attrs, *border);
            merge_caller_over(&mut attrs, extra);
            let kind =
                fqn_to_kind(class).ok_or_else(|| BehaviorError::UnknownBehavior(class.clone()))?; // fail-fast
            create(kind, &attrs, animations.clone(), scale)
        }
        ActionDef::Stay {
            border,
            attrs: _,
            animations,
        }
        | ActionDef::Move {
            border,
            attrs: _,
            animations,
        }
        | ActionDef::Animate {
            border,
            attrs: _,
            animations,
        } => {
            let kind = match def {
                ActionDef::Stay { .. } => ActionKind::Stay,
                ActionDef::Move { .. } => ActionKind::Move,
                _ => ActionKind::Animate,
            };
            let mut attrs = def_attrs(def).clone();
            inject_border_type(&mut attrs, *border);
            merge_caller_over(&mut attrs, extra);
            create(kind, &attrs, animations.clone(), scale)
        }
        ActionDef::Sequence {
            border,
            attrs: _,
            is_loop,
            animations: _,
            children,
        }
        | ActionDef::Select {
            border,
            attrs: _,
            is_loop,
            animations: _,
            children,
        } => {
            let mut attrs = def_attrs(def).clone();
            inject_border_type(&mut attrs, *border);
            merge_caller_over(&mut attrs, extra);
            // Java createActions L445-455: 子は caller パラメータ Map.of() で構築
            let mut built_actions: Vec<Box<dyn Action>> = Vec::new();
            for child in children {
                match child {
                    crate::config::SequenceChild::Ref {
                        name,
                        attrs: ref_attrs,
                    } => {
                        let child_def = actions
                            .actions
                            .get(name)
                            .ok_or_else(|| BehaviorError::UnknownBehavior(name.clone()))?;
                        // Java ActionRef.buildAction L113-126 逐語:
                        // newParams = params(caller = Map.of()); newParams.putAll(this.params)
                        // → 参照先構築の引数 = Ref 自身の attrs（Ref 側優先）
                        built_actions.push(build_def(actions, child_def, ref_attrs, scale)?);
                    }
                    crate::config::SequenceChild::Inline(inline) => {
                        built_actions.push(build_def(actions, inline, &VarMap::new(), scale)?);
                    }
                }
            }
            let is_sequence = matches!(def, ActionDef::Sequence { .. });
            Ok(Box::new(Complex::new(
                attrs,
                built_actions,
                is_sequence,
                *is_loop,
            )))
        }
    }
}
