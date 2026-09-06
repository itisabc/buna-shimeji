//! 画像セット（`img/<SetName>/*.png`）の一括展開と conf↔set 整合チェック。
//!
//! Java `ImagePairs` / `ImageUtils` / `AnimationBuilder` の画像読み込み部分相当:
//! - PNG を straight RGBA8（非プレマルチプル）で保持する。premultiply は
//!   win/window.rs 側の描画時処理（UpdateLayeredWindow の要件）なのでここでは行わない
//! - 丸め規則は Java に一致させる（Java `Math.round` = floor(x + 0.5)。
//!   Rust `f64::round` は負の半端で 0 から遠ざかるため使用しない）
//! - scale のロード時プリスケール: 寸法は Java ImageUtils.scale の
//!   `(int) Math.round(width * effectiveScaling)`、フィルタは Java 既定の
//!   NEAREST_NEIGHBOUR。アンカーは ImagePairs.java L81-82 の
//!   `(int) Math.round(anchorX * scaling)`（±1 補正なし）、速度は
//!   AnimationBuilder.java L206-211（非ゼロ→0 に丸まった成分を符号付き ±1 に補正）
//!
//! Phase 1 の意図的な範囲外: opacity / hqx フィルタ / ImageRight 右向き反転
//! （反転は #5 描画側で検討）/ ログ出力 / 非 PNG の警告。
//! サブディレクトリ内の PNG は対象外（資産はフラット構成・明記済みの制限）。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::config::{ActionDef, ActionsConfig, Pose};

/// PNG シグネチャ（8 バイト）。
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// 画像セット読み込みエラー。
#[derive(Debug, Error)]
pub enum ImagesetError {
    /// ファイル / ディレクトリ I/O 失敗（set ディレクトリ不在を含む）。
    #[error("I/O エラー: {0}")]
    Io(#[from] std::io::Error),
    /// PNG ヘッダとして解釈できないファイル（シグネチャ不正・IHDR 不在など）。
    #[error("PNG ヘッダを読めません: {0}")]
    NotPng(String),
}

/// 1 フレーム（1 ポーズ画像）。straight RGBA8（行優先・非プレマルチプル）。
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// 画像セット。frames のキーは正規化済み PNG 名（例: "shime1.png"）。
#[derive(Debug, Clone)]
pub struct ImageSet {
    pub name: String,
    pub frames: BTreeMap<String, Frame>,
    /// ロード時に検出した問題（日本語・1 行）。ログ出力は呼び出し側の責務。
    pub warnings: Vec<String>,
}

/// conf↔set 整合チェックの結果。
#[derive(Debug, Clone)]
pub struct ConsistencyReport {
    pub warnings: Vec<String>,
    pub disabled: Vec<DisabledAnimation>,
}

/// 欠落参照により丸ごと無効化されたアニメーション。
#[derive(Debug, Clone)]
pub struct DisabledAnimation {
    pub action: String,
    /// animations 内の位置（0 始まり）。
    pub animation_index: usize,
    /// 欠落参照の正規化名（重複除去・アニメ内出現順）。
    pub missing: Vec<String>,
}

/// 先頭 '/' を除去した正規化 image 参照名を返す。
pub fn normalize_image_ref(image_ref: &str) -> &str {
    image_ref.strip_prefix('/').unwrap_or(image_ref)
}

/// Java `Math.round(double)` 相当（floor(x + 0.5)。負の半端は正方向へ丸める）。
/// Rust `f64::round` は 0 から遠ざかる方向へ丸めるため使用しない。
/// NaN は 0 になる（Java `(int) Double.NaN` と同じ）。
pub fn java_round(x: f64) -> i32 {
    (x + 0.5).floor() as i32
}

/// アンカーを scale する（Java ImagePairs.java L81-82:
/// `(int) Math.round(anchor * scaling)`。非ゼロ→0 に丸まっても ±1 補正はしない）。
pub fn scale_anchor(anchor: (i32, i32), scale: f64) -> (i32, i32) {
    (
        java_round(f64::from(anchor.0) * scale),
        java_round(f64::from(anchor.1) * scale),
    )
}

/// 速度を scale する（Java AnimationBuilder.java L206-211。
/// 非ゼロが 0 に丸まった成分は符号付き ±1 に補正して移動不能を防ぐ）。
pub fn scale_velocity(velocity: (i32, i32), scale: f64) -> (i32, i32) {
    let (dx, dy) = velocity;
    let mut sx = java_round(f64::from(dx) * scale);
    let mut sy = java_round(f64::from(dy) * scale);
    if dx != 0 && sx == 0 {
        sx = if dx < 0 { -1 } else { 1 };
    }
    if dy != 0 && sy == 0 {
        sy = if dy < 0 { -1 } else { 1 };
    }
    (sx, sy)
}

/// ポーズを scale する。anchor / velocity は scale、image / duration は不変
///（画像寸法のプリスケールは [`ImageSet::load`] が担当）。
pub fn scale_pose(pose: &Pose, scale: f64) -> Pose {
    Pose {
        image: pose.image.clone(),
        anchor: scale_anchor(pose.anchor, scale),
        velocity: scale_velocity(pose.velocity, scale),
        duration: pose.duration,
    }
}

/// PNG シグネチャ + IHDR のみで寸法を読む（全体デコード不要）。
/// width は先頭から 16 バイト目、height は 20 バイト目（u32 ビッグエンディアン）。
/// IDAT 以降が壊れていても寸法は返す。シグネチャ不正 / 短すぎ / IHDR 不在は Err。
pub fn read_png_size(path: &Path) -> Result<(u32, u32), ImagesetError> {
    let bytes = fs::read(path)?;
    read_png_size_from(&bytes).ok_or_else(|| ImagesetError::NotPng(path.display().to_string()))
}

/// バイト列から PNG 寸法を解釈する（シグネチャ 8B + チャンク length 4B + type 4B
/// + width 4B + height 4B = 24 バイト目までを要求）。
fn read_png_size_from(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || bytes[..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((width, height))
}

/// ファイル名の拡張子が png（大小文字を問わない）か。
fn is_png_file_name(file_name: &str) -> bool {
    Path::new(file_name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
}

/// set ディレクトリのパス。
fn set_dir(img_dir: &Path, set_name: &str) -> PathBuf {
    img_dir.join(set_name)
}

/// img_dir 配下のサブディレクトリ名（= 画像セット名）を列挙する。不在は Err。
pub fn enumerate_sets(img_dir: &Path) -> Result<Vec<String>, ImagesetError> {
    let mut sets = Vec::new();
    for entry in fs::read_dir(img_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            sets.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    sets.sort();
    Ok(sets)
}

/// set ディレクトリ直下の PNG ファイル名（正規化済み・トップレベルのみ）を列挙する。
pub fn available_refs(img_dir: &Path, set_name: &str) -> Result<Vec<String>, ImagesetError> {
    let mut refs = Vec::new();
    for entry in fs::read_dir(set_dir(img_dir, set_name))? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if is_png_file_name(&file_name) {
            refs.push(file_name);
        }
    }
    refs.sort();
    Ok(refs)
}

/// Java ImageUtils.scale の `(int) Math.round(dimension * effectiveScaling)` 相当。
/// 寸法 0 は image クレートのリサイズが panic するため 1 にクランプする
///（scale > 0 の実用範囲では発生しない）。
fn scaled_dimension(dimension: u32, scale: f64) -> u32 {
    java_round(f64::from(dimension) * scale).max(1) as u32
}

impl ImageSet {
    /// set ディレクトリ直下の PNG をすべて straight RGBA8 に展開する
    ///（Java `ImagePairs.load` + `ImageUtils.scale` 相当）。
    ///
    /// - 非 PNG（拡張子判定）は無言スキップ（banner.bmp 等）
    /// - デコード失敗 PNG は IHDR 寸法の全透明フレームで代替 + 警告、
    ///   ヘッダ自体が読めない PNG はフレーム欠落 + 警告
    /// - サイズ混在 set は警告 1 件を追加して続行（フレームは各自の寸法を保持）
    /// - `scale` が Some(s) かつ s != 1.0 のとき全フレームを java_round 寸法・
    ///   Nearest でプリスケール
    /// - set ディレクトリ不在 / 列挙 I/O エラーは Err（set 単位のスキップ判断は呼び出し側）
    pub fn load(
        img_dir: &Path,
        set_name: &str,
        scale: Option<f64>,
    ) -> Result<ImageSet, ImagesetError> {
        let mut frames = BTreeMap::new();
        let mut warnings = Vec::new();

        for entry in fs::read_dir(set_dir(img_dir, set_name))? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().into_owned();
            if !is_png_file_name(&file_name) {
                continue;
            }
            let path = entry.path();
            match image::open(&path) {
                Ok(decoded) => {
                    let rgba = decoded.to_rgba8();
                    let (width, height) = rgba.dimensions();
                    frames.insert(
                        file_name,
                        Frame {
                            width,
                            height,
                            rgba: rgba.into_raw(),
                        },
                    );
                }
                Err(_) => match read_png_size(&path) {
                    // ヘッダが読める: IHDR 寸法の全透明フレームで代替
                    Ok((width, height)) => {
                        warnings.push(format!(
                            "{file_name}: デコードに失敗したため {width}x{height} の全透明フレームで代替しました"
                        ));
                        frames.insert(
                            file_name,
                            Frame {
                                width,
                                height,
                                rgba: vec![0; (width * height * 4) as usize],
                            },
                        );
                    }
                    // ヘッダも読めない: フレーム欠落 + 警告
                    Err(_) => {
                        warnings.push(format!(
                            "{file_name}: PNG ヘッダが読めないため読み飛ばしました"
                        ));
                    }
                },
            }
        }

        // サイズ混在は 1 件の警告で続行（Frame は各自の寸法を保持）
        let sizes: BTreeSet<(u32, u32)> = frames.values().map(|f| (f.width, f.height)).collect();
        if sizes.len() > 1 {
            let dims: Vec<String> = sizes.iter().map(|(w, h)| format!("{w}x{h}")).collect();
            warnings.push(format!(
                "{set_name}: フレーム寸法が混在しています（{}）。各フレームは自身の寸法のまま読み込みました",
                dims.join(", ")
            ));
        }

        // scale 指定（1.0 以外）なら全フレームをプリスケール
        if let Some(s) = scale.filter(|&s| s != 1.0) {
            for frame in frames.values_mut() {
                let new_width = scaled_dimension(frame.width, s);
                let new_height = scaled_dimension(frame.height, s);
                let src = image::RgbaImage::from_raw(
                    frame.width,
                    frame.height,
                    std::mem::take(&mut frame.rgba),
                )
                .expect("フレームの rgba 長は寸法と整合する");
                let dst = image::imageops::resize(
                    &src,
                    new_width,
                    new_height,
                    image::imageops::FilterType::Nearest,
                );
                frame.width = dst.width();
                frame.height = dst.height();
                frame.rgba = dst.into_raw();
            }
        }

        Ok(ImageSet {
            name: set_name.to_string(),
            frames,
            warnings,
        })
    }

    /// image 参照でフレームを引く（先頭 '/' は正規化して許容）。
    pub fn frame(&self, image_ref: &str) -> Option<&Frame> {
        self.frames.get(normalize_image_ref(image_ref))
    }
}

/// conf の全アクション（トップレベルの animations）について、各ポーズの参照画像が
/// available に存在するかを検査する。1 個でも欠落があればそのアニメーションを
/// 丸ごと disabled に登録する（ネスト Inline 定義は Phase 1 対象外）。
pub fn check_references(config: &ActionsConfig, available: &[String]) -> ConsistencyReport {
    let mut warnings = Vec::new();
    let mut disabled = Vec::new();

    for (action_name, def) in &config.actions {
        let animations = match def {
            ActionDef::Embedded { animations, .. }
            | ActionDef::Stay { animations, .. }
            | ActionDef::Move { animations, .. }
            | ActionDef::Animate { animations, .. }
            | ActionDef::Sequence { animations, .. }
            | ActionDef::Select { animations, .. } => animations,
        };
        for (index, animation) in animations.iter().enumerate() {
            let mut missing: Vec<String> = Vec::new();
            for pose in &animation.poses {
                let normalized = normalize_image_ref(&pose.image);
                if !available.iter().any(|a| a == normalized)
                    && !missing.iter().any(|m| m == normalized)
                {
                    missing.push(normalized.to_string());
                }
            }
            if missing.is_empty() {
                continue;
            }
            warnings.push(format!(
                "アクション '{action_name}' のアニメーション #{index}: 画像 {} が見つからないため無効化しました",
                missing.join(", ")
            ));
            disabled.push(DisabledAnimation {
                action: action_name.clone(),
                animation_index: index,
                missing,
            });
        }
    }

    ConsistencyReport { warnings, disabled }
}
