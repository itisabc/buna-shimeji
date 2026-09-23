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
//! per-set 定義集合（#32）: set ごとに `Actions.xml` を持つ資産（デレマスしめじ
//! v1.9 等）では **同名 Action でも内容が異なる**（例: `Stand` の参照画像が set
//! ごとに違う）ため、1 個の [`ActionsConfig`] では全 set を賄えない。
//! [`XmlBehaviorFactory::from_sets`] が set 名 → 定義集合を登録し、
//! [`BehaviorFactory::set_image_set`]（[`BehaviorTable::build_behavior_direct`] が
//! 構築直前にマスコットの image set 名を注入する）で選択する。
//! スコープ未設定 / 未知 set は既定集合へフォールバックする。
//!
//! 構築契約（Java 逐語踏襲・action/mod.rs 参照）:
//! - Ref 子: 既存 `build_action` 経由 =「参照先 attrs + Ref 側 attrs」の
//!   Ref 側優先マージ（Java ActionRef.buildAction L113-126）
//! - Inline 子: 既存 `build_def` 経由 = 空パラメータ（Java createActions
//!   L445-455 `buildAction(Map.of())`）

use std::collections::HashMap;
use std::sync::Arc;

use crate::config::{ActionsConfig, SequenceChild, VarMap};
use crate::mascot::behavior::{Action, BehaviorError, BehaviorFactory};

use super::{build_action, build_def};

/// XML 資産駆動の BehaviorFactory。
/// 構築時に disabled 該当アニメを除去済みの定義集合（set 名 → 集合）を保持し、
/// 以後の構築は [`BehaviorFactory::set_image_set`] で選ばれた集合を経由する。
pub struct XmlBehaviorFactory {
    /// set 名 → disabled 除去済み定義集合（`new` / `from_sets` 以後は不変・
    /// 構築時の clone を避けるため `Arc` 共有）。
    sets: HashMap<String, Arc<ActionsConfig>>,
    /// スコープ未設定 / 未知 set 用の既定集合（[`XmlBehaviorFactory::new`] は
    /// 単一集合、[`XmlBehaviorFactory::from_sets`] は先頭 set の集合）。
    default: Arc<ActionsConfig>,
    /// 現在の構築スコープ（[`BehaviorFactory::set_image_set`] が更新する）。
    current: Option<String>,
    /// 構築時 scale（[`scale_pose`](crate::render::imageset::scale_pose) 適用用・
    /// Java AnimationBuilder L206-211 相当）。[`BehaviorFactory::set_scale`] で
    /// 更新され、`build_action` 呼び出し時に全ポーズへ適用される。初期値は 1.0。
    scale: f64,
}

impl XmlBehaviorFactory {
    /// `disabled` の (action 名, animation_index) に該当するアニメを
    /// `actions` から除去済みのファクトリを構築する（単一集合版 = 全 set が
    /// 同じ定義集合を使う既存互換のコンストラクタ）。フィルタはここで 1 回だけ
    /// 行い、`build_action` 呼び出し毎には行わない。
    /// scale は [`BehaviorFactory::set_scale`] で構築前に注入される（初期値 1.0）。
    pub fn new(actions: ActionsConfig, disabled: &[(String, usize)]) -> XmlBehaviorFactory {
        let mut actions = actions;
        actions.strip_animations(disabled);
        XmlBehaviorFactory {
            sets: HashMap::new(),
            default: Arc::new(actions),
            current: None,
            scale: 1.0,
        }
    }

    /// set 名 → disabled 除去済み定義集合から per-set ファクトリを構築する。
    /// 先頭要素が既定集合（= Reload 素材の先頭 set = 辞書順先頭 set）になる。
    pub fn from_sets(
        sets: impl IntoIterator<Item = (String, Arc<ActionsConfig>)>,
    ) -> XmlBehaviorFactory {
        let mut map = HashMap::new();
        let mut default: Option<Arc<ActionsConfig>> = None;
        for (name, actions) in sets {
            if default.is_none() {
                default = Some(Arc::clone(&actions));
            }
            map.insert(name, actions);
        }
        XmlBehaviorFactory {
            sets: map,
            default: default.unwrap_or_else(|| Arc::new(ActionsConfig::default())),
            current: None,
            scale: 1.0,
        }
    }

    /// 現在のスコープの定義集合（未設定 / 未知 set は既定集合）。
    fn scoped_actions(&self) -> &ActionsConfig {
        self.current
            .as_ref()
            .and_then(|name| self.sets.get(name))
            .unwrap_or(&self.default)
    }
}

impl BehaviorFactory for XmlBehaviorFactory {
    fn set_scale(&mut self, scale: f64) {
        self.scale = scale;
    }

    fn set_image_set(&mut self, image_set: &str) {
        // set 名は構築の都度マスコットから注入される（未知 set は既定集合へ落ちる）。
        // 同一 set の連続構築では String を作り直さない。
        if self.current.as_deref() != Some(image_set) {
            self.current = Some(image_set.to_string());
        }
    }

    fn build_action(&mut self, child: &SequenceChild) -> Result<Box<dyn Action>, BehaviorError> {
        let actions = self.scoped_actions();
        match child {
            // Ref 子: 既存 build_action 経由（Ref 側 attrs 優先マージ込み）
            SequenceChild::Ref { name, attrs } => build_action(actions, name, attrs, self.scale),
            // Inline 子: 既存 build_def 直経由（空パラメータ・Java Map.of() 逐語）
            SequenceChild::Inline(def) => build_def(actions, def, &VarMap::new(), self.scale),
        }
    }
}
