//! 資産ディレクトリ解決（タスク #10b-2b・design.md L89-96 + AGENTS §9）。
//!
//! 起動は **exe と同じ場所の conf/ と img/** を使う（design.md L89-96・
//! 「cargo run --release で exe と同じ場所に conf/ と img/ が必要」= AGENTS §9）。
//! img/ 配下の各ディレクトリ = 1 画像 set。XML パースや画像読み込み等の
//! 素材検証は [`load_materials`](crate::app::reload::load_materials) 側の責務のため、
//! 本モジュールはパス解決 + ディレクトリ存在検証のみを行う純関数群である
//! （[`crate::main`]（#10b-2c）からの利用を想定）。

use std::path::{Path, PathBuf};

use thiserror::Error;

/// 解決済みの資産ディレクトリ（[`resolve_assets`] の成功値）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetDirs {
    /// exe 同場所の `conf` ディレクトリ（actions.xml / behaviors.xml 等）。
    pub conf_dir: PathBuf,
    /// exe 同場所の `img` ディレクトリ（配下の各ディレクトリ = 1 画像 set）。
    pub img_dir: PathBuf,
}

/// 資産ディレクトリの検出エラー（欠落種別で区別する）。
#[derive(Debug, Error)]
pub enum AssetError {
    /// `conf` ディレクトリが見つからない（`exe_dir.join("conf")` を報告）。
    #[error("conf directory not found: {0}")]
    ConfDirNotFound(PathBuf),
    /// `img` ディレクトリが見つからない（`exe_dir.join("img")` を報告）。
    #[error("img directory not found: {0}")]
    ImgDirNotFound(PathBuf),
}

/// exe 起動ディレクトリから資産ディレクトリを解決する（design.md L89-96）。
///
/// - `exe_dir/conf`・`exe_dir/img` がともにディレクトリとして存在 →
///   [`Ok(AssetDirs)`](AssetDirs)（`conf_dir = exe_dir.join("conf")` /
///   `img_dir = exe_dir.join("img")`）
/// - conf 欠落 → [`AssetError::ConfDirNotFound`]
/// - img 欠落 → [`AssetError::ImgDirNotFound`]
/// - 両方欠落 → Err（検査順は conf 先）
pub fn resolve_assets(exe_dir: &Path) -> Result<AssetDirs, AssetError> {
    let conf_dir = exe_dir.join("conf");
    if !conf_dir.is_dir() {
        return Err(AssetError::ConfDirNotFound(conf_dir));
    }
    let img_dir = exe_dir.join("img");
    if !img_dir.is_dir() {
        return Err(AssetError::ImgDirNotFound(img_dir));
    }
    Ok(AssetDirs { conf_dir, img_dir })
}
