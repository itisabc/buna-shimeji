//! Reload 素材ローダ（タスク #9d・design.md §1.10 (d) 9d + §2 Reload 方針）。
//!
//! Reload の差し替え素材（set 名・[`ImageSet`](crate::render::imageset::ImageSet)・
//! [`BehaviorTable`](crate::mascot::behavior::BehaviorTable)・
//! [`ActionsConfig`]）を conf/img ディレクトリから一括ロードする。エラー方針は
//! 「conf 側の失敗 = 素材全体の中止（Err 伝播）、個々の画像 set の失敗 = warn
//! ログ + スキップ（他の set は続行）」。
//!
//! #32 per-set 化: actions / behaviors は set ごとに解決する（探索順は Java
//! `Main.getActionsFilePath` / `getBehaviorsFilePath` と同一）。set 専用ファイルが
//! 無い set は `conf/` 直下の共通ファイルへフォールバックし、どちらも無ければ
//! warn + その set をスキップする（Java failedConfigurations 相当）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use crate::config::{
    parse_actions, parse_behaviors, validate_required_behaviors, ActionsConfig, ConfigError,
};
use crate::mascot::behavior::BehaviorTable;
use crate::render::imageset::{
    available_refs, check_references, enumerate_sets, DisabledAnimation, ImageSet, ImagesetError,
};

/// actions ファイル名の候補（Java `Main.ACTIONS_FILENAMES` L62-64 逐語）。
/// 大小文字を区別しない FS（Windows）では Java と同様に `Actions.xml` も一致する。
const ACTIONS_FILENAMES: [&str; 4] = ["actions.xml", "動作.xml", "one.xml", "1.xml"];

/// behaviors ファイル名の候補（Java `Main.BEHAVIORS_FILENAMES` L66-68 逐語）。
const BEHAVIORS_FILENAMES: [&str; 5] = [
    "behaviors.xml",
    "behavior.xml",
    "行動.xml",
    "two.xml",
    "2.xml",
];

/// マスコット 1 set 分の conf ファイルを Java と同じ探索順で解決する
/// （Java `Main.getActionsFilePath` / `getBehaviorsFilePath` L391-427 逐語）:
/// `img/<set>/conf/` → `conf/<set>/` → `conf/` の順に、候補ファイル名を順に試し、
/// 最初に見つかったものを返す。見つからなければ `None`。
fn resolve_config_file(
    conf_dir: &Path,
    img_dir: &Path,
    set: &str,
    names: &[&str],
) -> Option<PathBuf> {
    let dirs = [
        img_dir.join(set).join("conf"),
        conf_dir.join(set),
        conf_dir.to_path_buf(),
    ];
    for dir in dirs {
        for name in names {
            let path = dir.join(name);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

/// Reload の差し替え素材 1 set 分（[`crate::app::manager::Manager::reload`] の入力）。
/// `Debug` はテストの `expect_err`（`Result<Vec<_>, _>` の Ok 側表示）要件。
#[derive(Debug)]
pub struct ReloadMaterial {
    /// 画像 set 名（img_dir 配下のディレクトリ名）。
    pub name: String,
    /// 当該 set のロード済み画像セット。
    pub image_set: Arc<ImageSet>,
    /// 当該 set 用の行動表（set の behaviors ファイル全行の set 毎所有 copy）。
    pub table: BehaviorTable,
    /// 当該 set の action 定義集合（欠落参照アニメを除去済み・#32）。
    /// [`crate::mascot::action::factory::XmlBehaviorFactory::from_sets`] が構築に使う。
    pub actions: Arc<ActionsConfig>,
    /// 当該 set で欠落参照により無効化されたアニメ（check_references 結果・
    /// #10b-1。消費先は構築経由の
    /// [`XmlBehaviorFactory`](crate::mascot::action::factory::XmlBehaviorFactory)）。
    pub disabled_animations: Vec<DisabledAnimation>,
}

/// 素材ロードエラー（conf 側 / set 列挙の失敗 = 素材全体の中止）。
#[derive(Debug, Error)]
pub enum MaterialError {
    /// conf（actions.xml / behaviors.xml / 必須 4 種 Behavior）の読み込み失敗。
    #[error("failed to load configuration: {0}")]
    Config(#[from] ConfigError),
    /// 画像 set の列挙失敗（img_dir 不在等）。
    #[error("failed to enumerate image sets: {0}")]
    Imageset(#[from] ImagesetError),
}

/// conf/img ディレクトリから Reload 素材を一括ロードする。
///
/// 手順（#9d 契約 + #32 per-set 化）:
/// 1. [`enumerate_sets`] で set を辞書順列挙 → Err 伝播。辞書順がそのまま
///    [`ReloadMaterial`] の順 =「既定 set は先頭」を決める
/// 2. set 毎に actions / behaviors ファイルを [`resolve_config_file`]
///    （Java `Main.getActionsFilePath` / `getBehaviorsFilePath` の探索順）で解決する。
///    どちらか見つからない set は warn ログ + スキップ（Java の failedConfigurations
///    相当・画像だけ置かれた set を許容する）。見つかったファイルのパース失敗は
///    Err 伝播（素材全体の中止）
/// 3. 当該 set の behaviors に [`validate_required_behaviors`]（必須 4 種）→ Err 伝播
/// 4. set 毎に [`ImageSet::load`]（`scales` に無い set は `None` = 等倍）→
///    失敗 set は warn ログ + スキップ（他は続行）
/// 5. set 毎に [`available_refs`] + [`check_references`] で conf↔set 整合を検査し
///    `report.warnings` を warn ログ。`report.disabled`（欠落参照アニメ）は
///    [`ReloadMaterial::disabled_animations`] に記録し、同時に当該 set の
///    [`ActionsConfig`] から除去する（実無効化の適用・#10b-1）。
///    refs 列挙の I/O 失敗 set も画像読み込み失敗に準じてスキップする
/// 6. 行動表は set 毎に [`BehaviorTable::new`] で所有 copy する
/// 7. 列挙順の [`Vec<ReloadMaterial>`] を返す（set 0 件 = Ok(空 Vec)）
pub fn load_materials(
    conf_dir: &Path,
    img_dir: &Path,
    scales: &HashMap<String, f64>,
) -> Result<Vec<ReloadMaterial>, MaterialError> {
    // 1. set 列挙（辞書順 = ReloadMaterial の順 = 既定 set は先頭）
    let sets = enumerate_sets(img_dir)?;

    let mut materials = Vec::new();
    for set in sets {
        // 2. set 別 conf 解決（見つからない set はスキップ = Java failedConfigurations）
        let (Some(actions_path), Some(behaviors_path)) = (
            resolve_config_file(conf_dir, img_dir, &set, &ACTIONS_FILENAMES),
            resolve_config_file(conf_dir, img_dir, &set, &BEHAVIORS_FILENAMES),
        ) else {
            log::warn!("skipping image set `{set}`: no actions/behaviors file found for it");
            continue;
        };

        // 2-3. パース + 必須 4 種（失敗 = 素材全体の中止）
        let actions = parse_actions(&actions_path)?;
        let behaviors = parse_behaviors(&behaviors_path)?;
        validate_required_behaviors(&behaviors)?;

        // 4. 画像セット読み込み（失敗 set はスキップ・他は続行）
        let image_set = match ImageSet::load(img_dir, &set, scales.get(&set).copied()) {
            Ok(image_set) => Arc::new(image_set),
            Err(err) => {
                log::warn!("skipping image set `{set}`: failed to load it: {err}");
                continue;
            }
        };

        // 5. conf↔set 参照整合の警告出力 + 欠落参照アニメの実無効化
        let refs = match available_refs(img_dir, &set) {
            Ok(refs) => refs,
            Err(err) => {
                log::warn!("skipping image set `{set}`: failed to enumerate its images: {err}");
                continue;
            }
        };
        let report = check_references(&actions, &refs);
        for warning in &report.warnings {
            log::warn!("{warning}");
        }
        let disabled: Vec<(String, usize)> = report
            .disabled
            .iter()
            .map(|entry| (entry.action.clone(), entry.animation_index))
            .collect();
        let mut actions = actions;
        actions.strip_animations(&disabled);

        materials.push(ReloadMaterial {
            name: set,
            image_set,
            // 6. 行動表は set 毎の所有 copy
            table: BehaviorTable::new(&behaviors),
            actions: Arc::new(actions),
            disabled_animations: report.disabled,
        });
    }
    Ok(materials)
}
