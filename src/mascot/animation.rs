//! Animation / Pose 再生ヘルパ — Java `animation.Animation` / `animation.Pose` 相当。
//!
//! 逐語移植の正本:
//! - Animation.java: getPoseAt L139-150 / getDuration L158-160 / isEffective L114-120 /
//!   init L84-89 / resetCondition L99-104
//! - Pose.java: apply L26-31
//!
//! Java の Animation は 1 アニメ 1 インスタンスだが、本実装は config の
//! [`Animation`]（強型データ・不変）に対する状態無しの関数群として提供する。
//! 条件のキャッシュポリシーは script.rs の [`Variables`] 契約どおり（呼び出し側が
//! `init` / `reset_values` を制御する）。
//!
//! Phase 1 の意図的な範囲外（doc 開示）: 効果音（Java Pose.apply の setSound）は
//! 効果音実体ごと #9 の範囲外のため適用しない。資産の画像は ImagePairs の
//! rightImage 相当（leftWidth - anchorX）を flip 調整済み center として再現する。
//!
//! scale 契約: 本モジュールは config の Pose（生値）をそのまま解釈する。set 単位
//! scale が指定された場合、アンカー / velocity の変換（`imageset::scale_pose` 等・
//! Java AnimationBuilder L206-211 相当）はアクション構築時（タスク #7）に行う
//! （plan.md (H)・(Q) 契約。apply_pose での追加スケールは二重適用になるため禁止）。

use super::{ImageState, Mascot};
use crate::config::script::{EvalContext, EvalError, EvalValue, Variables};
use crate::config::{Animation, Pose};
use crate::render::imageset::normalize_image_ref;

/// アニメーション全体の長さ（poses の duration 合計。Java Animation.getDuration 相当）。
pub fn animation_duration(animation: &Animation) -> i32 {
    animation.poses.iter().map(|pose| pose.duration).sum()
}

/// 時刻 time に対応するポーズ（Java Animation.getPoseAt L139-150 逐語）。
/// time を duration で剰余した後、poses を順に減算して time < 0 になった pose を返す。
/// duration 0（Java では 0 除算の ArithmeticException）や空 poses は None で安全化。
pub fn animation_pose_at(animation: &Animation, time: i32) -> Option<&Pose> {
    let duration = animation_duration(animation);
    if duration == 0 {
        return None;
    }
    let mut time = time % duration;
    for pose in &animation.poses {
        time -= pose.duration;
        if time < 0 {
            return Some(pose);
        }
    }
    None
}

/// アニメの条件式が有効か（Java Animation.isEffective L114-120 相当）。
/// condition 無しは常時 true。評価結果はブールである必要があり、数値は
/// Java の (Boolean) キャスト失敗相当として Err にする。
pub fn animation_is_effective(
    animation: &Animation,
    vars: &mut Variables,
    ctx: &dyn EvalContext,
) -> Result<bool, EvalError> {
    let Some(condition) = animation.condition.as_ref() else {
        return Ok(true);
    };
    match vars.eval(condition, ctx)? {
        EvalValue::Bool(effective) => Ok(effective),
        EvalValue::Number(value) => Err(EvalError {
            expr: format!("{condition:?}"),
            message: format!("animation condition must be a boolean (got number {value})"),
        }),
    }
}

/// アニメ条件の初期化（Java Animation.init L84-89 相当）。
/// 全式のキャッシュをクリアして再評価させる（アクション開始時の呼び出し想定）。
pub fn animation_init_condition(
    animation: &Animation,
    vars: &mut Variables,
) -> Result<(), EvalError> {
    if animation.condition.is_some() {
        vars.init();
    }
    Ok(())
}

/// アニメ条件のフレーム開始時のキャッシュクリア（Java Animation.resetCondition
/// L99-104 相当）。`#{}` のみ再評価される（`${}` はキャッシュ保持）。
pub fn animation_reset_condition(
    animation: &Animation,
    vars: &mut Variables,
) -> Result<(), EvalError> {
    if animation.condition.is_some() {
        vars.reset_values();
    }
    Ok(())
}

/// ポーズを 1 フレーム適用する（Java Pose.apply L26-31 逐語）。
///
/// - anchor += (look_right ? -dx : dx, dy)（Java getAnchor().translate 逐語）
/// - 画像は image_set のフレームが存在するときのみ Some。center は flip 調整済み
///   （look_right のとき width - anchor.x。Java ImagePairs.getImage(right) 相当）
/// - 欠落フレームは set_image(None)（prev 保持・needs_repaint 立ち。Java の
///   setImage(null) 相当）
pub fn apply_pose(pose: &Pose, mascot: &mut Mascot) {
    let dx = if mascot.look_right() {
        -pose.velocity.0
    } else {
        pose.velocity.0
    };
    let (anchor_x, anchor_y) = mascot.anchor();
    mascot.set_anchor((anchor_x + dx, anchor_y + pose.velocity.1));

    let look_right = mascot.look_right();
    match mascot.image_set.frame(&pose.image) {
        Some(frame) => {
            let width = frame.width as i32;
            let center = if look_right {
                (width - pose.anchor.0, pose.anchor.1)
            } else {
                pose.anchor
            };
            mascot.set_image(Some(ImageState {
                image_ref: normalize_image_ref(&pose.image).to_string(),
                center,
                width: frame.width,
                height: frame.height,
            }));
        }
        None => mascot.set_image(None),
    }
}
