//! Reload 素材ローダ（タスク #9d・design.md §1.10 (d) 9d + §2 Reload 方針）。
//!
//! Reload の差し替え素材（set 名・[`ImageSet`](crate::render::imageset::ImageSet)・
//! [`BehaviorTable`](crate::mascot::behavior::BehaviorTable)）を conf/img ディレクトリ
//! から一括ロードする。エラー方針は「conf 側の失敗 = 素材全体の中止（Err 伝播）、
//! 個々の画像 set の失敗 = warn ログ + スキップ（他の set は続行）」。
//! conf↔set の参照整合は警告出力 + 欠落参照アニメの記録（`disabled_animations`・
//! #10b-1。実無効化の適用は構築経由の XmlBehaviorFactory）を行う。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use thiserror::Error;

use crate::config::{parse_actions, parse_behaviors, validate_required_behaviors, ConfigError};
use crate::mascot::behavior::BehaviorTable;
use crate::render::imageset::{
    available_refs, check_references, enumerate_sets, DisabledAnimation, ImageSet, ImagesetError,
};

/// Reload の差し替え素材 1 set 分（[`crate::app::manager::Manager::reload`] の入力）。
/// `Debug` はテストの `expect_err`（`Result<Vec<_>, _>` の Ok 側表示）要件。
#[derive(Debug)]
pub struct ReloadMaterial {
    /// 画像 set 名（img_dir 配下のディレクトリ名）。
    pub name: String,
    /// 当該 set のロード済み画像セット。
    pub image_set: Arc<ImageSet>,
    /// 当該 set 用の行動表（behaviors.xml 全行の set 毎所有 copy）。
    pub table: BehaviorTable,
    /// 当該 set で欠落参照により無効化されたアニメ（check_references 結果・
    /// #10b-1。消費先は構築経由の
    /// [`XmlBehaviorFactory`](crate::mascot::action::factory::XmlBehaviorFactory)）。
    pub disabled_animations: Vec<DisabledAnimation>,
}

/// 素材ロードエラー（conf 側 / set 列挙の失敗 = 素材全体の中止）。
#[derive(Debug, Error)]
pub enum MaterialError {
    /// conf（actions.xml / behaviors.xml / 必須 4 種 Behavior）の読み込み失敗。
    #[error("設定の読み込みに失敗しました: {0}")]
    Config(#[from] ConfigError),
    /// 画像 set の列挙失敗（img_dir 不在等）。
    #[error("画像セットの列挙に失敗しました: {0}")]
    Imageset(#[from] ImagesetError),
}

/// conf/img ディレクトリから Reload 素材を一括ロードする。
///
/// 手順（#9d 契約）:
/// 1. actions.xml / behaviors.xml をパース → Err はそのまま伝播（素材全体の中止）
/// 2. [`validate_required_behaviors`]（必須 4 種）→ Err 伝播
/// 3. [`enumerate_sets`] で set を辞書順列挙 → Err 伝播。辞書順がそのまま
///    [`ReloadMaterial`] の順 =「既定 set は先頭」を決める
/// 4. set 毎に [`ImageSet::load`]（`scales` に無い set は `None` = 等倍）→
///    失敗 set は warn ログ + スキップ（他は続行）
/// 5. set 毎に [`available_refs`] + [`check_references`] で conf↔set 整合を検査し
///    `report.warnings` を warn ログ。`report.disabled`（欠落参照アニメ）は
///    [`ReloadMaterial::disabled_animations`] に記録する（#10b-1・構築経由の
///    [`XmlBehaviorFactory`](crate::mascot::action::factory::XmlBehaviorFactory) が消費）。
///    refs 列挙の I/O 失敗 set も画像読み込み失敗に準じてスキップする
/// 6. 行動表は set 毎に [`BehaviorTable::new`] で所有 copy する
/// 7. 列挙順の [`Vec<ReloadMaterial>`] を返す（set 0 件 = Ok(空 Vec)）
pub fn load_materials(
    conf_dir: &Path,
    img_dir: &Path,
    scales: &HashMap<String, f64>,
) -> Result<Vec<ReloadMaterial>, MaterialError> {
    // 1. conf パース（失敗 = 素材全体の中止）
    let actions = parse_actions(&conf_dir.join("actions.xml"))?;
    let behaviors = parse_behaviors(&conf_dir.join("behaviors.xml"))?;

    // 2. 必須 4 種 Behavior の検証
    validate_required_behaviors(&behaviors)?;

    // 3. set 列挙（辞書順 = ReloadMaterial の順 = 既定 set は先頭）
    let sets = enumerate_sets(img_dir)?;

    let mut materials = Vec::new();
    for set in sets {
        // 4. 画像セット読み込み（失敗 set はスキップ・他は続行）
        let image_set = match ImageSet::load(img_dir, &set, scales.get(&set).copied()) {
            Ok(image_set) => Arc::new(image_set),
            Err(err) => {
                log::warn!("画像セット `{set}` の読み込みに失敗したためスキップします: {err}");
                continue;
            }
        };

        // 5. conf↔set 参照整合の警告出力（アニメ実無効化の適用は #10）
        let refs = match available_refs(img_dir, &set) {
            Ok(refs) => refs,
            Err(err) => {
                log::warn!("画像セット `{set}` の画像列挙に失敗したためスキップします: {err}");
                continue;
            }
        };
        let report = check_references(&actions, &refs);
        for warning in &report.warnings {
            log::warn!("{warning}");
        }

        // 6. 行動表は set 毎の所有 copy
        let table = BehaviorTable::new(&behaviors);

        materials.push(ReloadMaterial {
            name: set,
            image_set,
            table,
            disabled_animations: report.disabled,
        });
    }
    Ok(materials)
}
