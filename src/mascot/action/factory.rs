//! XML 資産駆動の [`BehaviorFactory`] 実装（design §1.10(f)・handoff glue ⑦）。
//!
//! Java 対応: `Configuration.buildAction` は actions.xml から `ActionBuilder` を
//! 選びアクションを組み立てる。本プロジェクトではその役割を [`build_action`] /
//! [`build_def`] が担っており、[`XmlBehaviorFactory`] は [`BehaviorFactory`] の
//! その消費先実装である（`BehaviorTable::build_behavior_direct` など app 側の
//! 構築経由を全てこの 1 実装へ集約する）。
//!
//! check_references 実無効化（design §1.10(f)・handoff ⑦）: imageset.rs の
//! `check_references` が検出した欠落参照アニメ
//! ([`DisabledAnimation`](crate::render::imageset::DisabledAnimation)) を、
//! 構築時に **1 回だけ** [`ActionsConfig`] から除去して保持する（= Reload 素材
//! [`ReloadMaterial`](crate::app::reload::ReloadMaterial) の
//! `disabled_animations` の消費先）。以後の全構築はそのフィルタ済み定義経由で
//! 行われるため、Sequence 内 Ref 子（actions 内参照）にも自然に伝播する。
//! build 毎のフィルタリングは行わない（構築時 1 回のみ・オブジェクト毎に
//! フィルタコストを払わないため）。
//!
//! 構築契約（Java 逐語踏襲・action/mod.rs 参照）:
//! - Ref 子: 既存 `build_action` 経由 =「参照先 attrs + Ref 側 attrs」の
//!   Ref 側優先マージ（Java ActionRef.buildAction L113-126）
//! - Inline 子: 既存 `build_def` 経由 = 空パラメータ（Java createActions
//!   L445-455 `buildAction(Map.of())`）

use crate::config::{ActionDef, ActionsConfig, SequenceChild, VarMap};
use crate::mascot::behavior::{Action, BehaviorError, BehaviorFactory};

use super::{build_action, build_def};

/// XML 資産駆動の BehaviorFactory。
/// 構築時に disabled 該当アニメを除去済みの定義集合を 1 回だけ保持し、
/// 以後の構築は全てそれを経由する（[`BehaviorFactory`] 実装）。
pub struct XmlBehaviorFactory {
    /// disabled 除去済みの定義集合（new 以後は不変）。
    actions: ActionsConfig,
    /// 構築時 scale（[`scale_pose`](crate::render::imageset::scale_pose) 適用用・
    /// Java AnimationBuilder L206-211 相当）。[`BehaviorFactory::set_scale`] で
    /// 更新され、`build_action` 呼び出し時に全ポーズへ適用される。初期値は 1.0。
    scale: f64,
}

impl XmlBehaviorFactory {
    /// `disabled` の (action 名, animation_index) に該当するアニメを
    /// `actions` から除去済みのファクトリを構築する。フィルタはここで 1 回だけ
    /// 行い、`build_action` 呼び出し毎には行わない。
    /// scale は [`BehaviorFactory::set_scale`] で構築前に注入される（初期値 1.0）。
    pub fn new(actions: ActionsConfig, disabled: &[(String, usize)]) -> XmlBehaviorFactory {
        let mut actions = actions;
        strip_disabled(&mut actions, disabled);
        XmlBehaviorFactory {
            actions,
            scale: 1.0,
        }
    }
}

/// disabled の (action 名, animation_index) に該当するアニメを定義から除去する。
/// 除去は index 降順で行い、先に除去した index が後続 index をずらさないようにする
///（check_references の index は除去前の animations 内位置を指すため）。
fn strip_disabled(actions: &mut ActionsConfig, disabled: &[(String, usize)]) {
    for (name, def) in actions.actions.iter_mut() {
        let animations = match def {
            ActionDef::Embedded { animations, .. }
            | ActionDef::Stay { animations, .. }
            | ActionDef::Move { animations, .. }
            | ActionDef::Animate { animations, .. }
            | ActionDef::Sequence { animations, .. }
            | ActionDef::Select { animations, .. } => animations,
        };
        // 該当 action 名の index を集め、重複除去のうえ降順で除去する
        let mut indices: Vec<usize> = disabled
            .iter()
            .filter(|(action, _)| action == name)
            .map(|(_, index)| *index)
            .collect();
        if indices.is_empty() {
            continue;
        }
        indices.sort_unstable();
        indices.dedup();
        for index in indices.into_iter().rev() {
            if index < animations.len() {
                animations.remove(index);
            }
        }
    }
}

impl BehaviorFactory for XmlBehaviorFactory {
    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
    }

    fn build_action(&mut self, child: &SequenceChild) -> Result<Box<dyn Action>, BehaviorError> {
        match child {
            // Ref 子: 既存 build_action 経由（Ref 側 attrs 優先マージ込み）
            SequenceChild::Ref { name, attrs } => {
                build_action(&self.actions, name, attrs, self.scale)
            }
            // Inline 子: 既存 build_def 直経由（空パラメータ・Java Map.of() 逐語）
            SequenceChild::Inline(def) => build_def(&self.actions, def, &VarMap::new(), self.scale),
        }
    }
}
