//! 画像セット（`img/<SetName>/*.png`）の一括展開と conf↔set 整合チェック。
//!
//! Java `ImagePairs` / `ImageUtils` / `AnimationBuilder` の画像読み込み部分相当:
//! - PNG を straight RGBA8（非プレマルチプル）でデコードし、ロード時に 1 回だけ
//!   プレマルチプライド 0xAARRGGBB（[`Frame::argb`]）へ変換して保持する。
//!   premultiply は描画時（win/window.rs）ではなくロード時（`ImageSet::load`）に行う
//! - 丸め規則は Java に一致させる（Java `Math.round` = floor(x + 0.5)。
//!   Rust `f64::round` は負の半端で 0 から遠ざかるため使用しない）
//! - scale のロード時プリスケール: 寸法は Java ImageUtils.scale の
//!   `(int) Math.round(width * effectiveScaling)`。フィルタは **Lanczos3**
//!   （straight RGBA8 のまま。プレマルチプライド形式への変換はプリスケール後）。
//!   アンカーは ImagePairs.java
//!   L81-82 の `(int) Math.round(anchorX * scaling)`（±1 補正なし）、速度は
//!   AnimationBuilder.java L206-211（非ゼロ→0 に丸まった成分を符号付き ±1 に補正）
//! - 逐語移植の例外: Java のフィルタ機構（nearest/bicubic/hqx）に lanczos は無く、
//!   Lanczos3 は本プロジェクト独自の意図的な逸脱（AGENTS.md §1 / design.md §1.6 の
//!   「明確なリスク時は Java と変えてよい」に基づく記録）。寸法・anchor・velocity の
//!   丸め規則は Java のまま不変
//!
//! Phase 1 の意図的な範囲外: opacity / hqx フィルタ / ImageRight 右向き反転
//! （反転は #5 描画側で検討）/ ログ出力 / 非 PNG の警告。
//! ロード時スケールのフィルタは Lanczos3 であり、この hqx 除外とは別軸。
//! サブディレクトリ内の PNG は対象外（資産はフラット構成・明記済みの制限）。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::config::{ActionDef, ActionsConfig, Pose};

/// PNG シグネチャ（8 バイト）。
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// フレーム寸法の 1 辺の上限（これを超えるフレームは欠落させる）。
const MAX_FRAME_DIMENSION: u32 = 8192;
/// フレームの最大ピクセル数（2048² = 4_194_304・最大 16MB/フレーム）。
const MAX_FRAME_PIXELS: u64 = 4_194_304;

/// フレーム寸法が有効範囲内か（0 超・1 辺 ≤ 8192・総ピクセル ≤ 4_194_304）。
/// 乗算は u64 で行いオーバーフローを避ける。
fn valid_frame_dimensions(width: u32, height: u32) -> bool {
    let w = u64::from(width);
    let h = u64::from(height);
    width > 0
        && height > 0
        && w <= u64::from(MAX_FRAME_DIMENSION)
        && h <= u64::from(MAX_FRAME_DIMENSION)
        && w * h <= MAX_FRAME_PIXELS
}

/// 画像セット読み込みエラー。
#[derive(Debug, Error)]
pub enum ImagesetError {
    /// ファイル / ディレクトリ I/O 失敗（set ディレクトリ不在を含む）。
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// PNG ヘッダとして解釈できないファイル（シグネチャ不正・IHDR 不在など）。
    #[error("cannot read PNG header: {0}")]
    NotPng(String),
}

/// 1 フレーム（1 ポーズ画像）。プレマルチプライド 0xAARRGGBB（行優先・非反転）。
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub argb: Vec<u32>,
}

impl Frame {
    /// straight RGBA8 をプレマルチプライド 0xAARRGGBB に変換して保持する。
    ///
    /// `width == 0 || height == 0 || rgba.is_empty()` のとき `argb` は空になる。
    /// それ以外は [`crate::win::window::premultiply_rgba_to_argb`] で変換する
    ///（切り捨て `(v * a) / 255`。整合入力なら `argb.len() == width * height`）。
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        let argb = if width == 0 || height == 0 || rgba.is_empty() {
            Vec::new()
        } else {
            crate::win::window::premultiply_rgba_to_argb(&rgba)
        };
        Frame {
            width,
            height,
            argb,
        }
    }
}

/// 画像セット。frames のキーは正規化済み PNG 名（例: "shime1.png"）。
#[derive(Debug, Clone)]
pub struct ImageSet {
    pub name: String,
    pub frames: BTreeMap<String, Frame>,
    /// ロード時に検出した問題（日本語・1 行）。ログ出力は呼び出し側の責務。
    pub warnings: Vec<String>,
    /// 解決済みの per-set scale（`None` → 1.0）。ポーズの anchor/velocity 変換
    /// （action 構築）がこの値を参照する。
    pub scale: f64,
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

/// ポーズを scale する。anchor / velocity は scale、image / duration / sound /
/// volume は不変（画像寸法のプリスケールは [`ImageSet::load`] が担当）。
pub fn scale_pose(pose: &Pose, scale: f64) -> Pose {
    Pose {
        image: pose.image.clone(),
        anchor: scale_anchor(pose.anchor, scale),
        velocity: scale_velocity(pose.velocity, scale),
        duration: pose.duration,
        sound: pose.sound.clone(),
        volume: pose.volume,
    }
}

/// PNG シグネチャ + IHDR のみで寸法を読む（全体デコード不要）。
/// width は先頭から 16 バイト目、height は 20 バイト目（u32 ビッグエンディアン）。
/// IDAT 以降が壊れていても寸法は返す。シグネチャ不正 / 短すぎ / IHDR 不在は Err。
/// ファイル全体は読まず先頭 24 バイトのみ読む。
pub fn read_png_size(path: &Path) -> Result<(u32, u32), ImagesetError> {
    let mut file = fs::File::open(path)?;
    let mut header = [0u8; 24];
    file.read_exact(&mut header)?;
    read_png_size_from(&header).ok_or_else(|| ImagesetError::NotPng(path.display().to_string()))
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
    /// set ディレクトリ直下の PNG をすべてプレマルチプライド ARGB の [`Frame`] に
    /// 展開する（Java `ImagePairs.load` + `ImageUtils.scale` 相当）。
    ///
    /// - 非 PNG（拡張子判定）は無言スキップ（banner.bmp 等）
    /// - デコード失敗 PNG は IHDR 寸法の全透明フレームで代替 + 警告、
    ///   ヘッダ自体が読めない PNG はフレーム欠落 + 警告
    /// - サイズ混在 set は警告 1 件を追加して続行（フレームは各自の寸法を保持）
    /// - `scale` が Some(s) かつ s != 1.0 のとき全フレームを java_round 寸法・
    ///   Lanczos3（straight RGBA8 のまま）でプリスケールし、その後
    ///   [`Frame::from_rgba`] でプレマルチプライド ARGB に変換する
    /// - set ディレクトリ不在 / 列挙 I/O エラーは Err（set 単位のスキップ判断は呼び出し側）
    pub fn load(
        img_dir: &Path,
        set_name: &str,
        scale: Option<f64>,
    ) -> Result<ImageSet, ImagesetError> {
        // straight RGBA8 の中間表現（プリスケール後に Frame::from_rgba で変換する。
        // プレマルチプライド形式を常駐させないため、スケールは変換前に適用する）。
        let mut rgba_frames: BTreeMap<String, image::RgbaImage> = BTreeMap::new();
        let mut warnings = Vec::new();
        // 解決済み scale（None → 1.0）。フレームのプリスケール判定と保持に共用する。
        let resolved_scale = scale.unwrap_or(1.0);

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
                    if !valid_frame_dimensions(width, height) {
                        warnings.push(format!(
                            "{file_name}: skipped because dimensions {width}x{height} exceed the limit"
                        ));
                        continue;
                    }
                    rgba_frames.insert(file_name, rgba);
                }
                Err(_) => match read_png_size(&path) {
                    // ヘッダが読める: IHDR 寸法の全透明フレームで代替
                    Ok((width, height)) => {
                        if !valid_frame_dimensions(width, height) {
                            warnings.push(format!(
                                "{file_name}: skipped because dimensions {width}x{height} are invalid"
                            ));
                            continue;
                        }
                        warnings.push(format!(
                            "{file_name}: decode failed; substituted a fully transparent {width}x{height} frame"
                        ));
                        rgba_frames.insert(
                            file_name,
                            image::RgbaImage::from_pixel(width, height, image::Rgba([0, 0, 0, 0])),
                        );
                    }
                    // ヘッダも読めない: フレーム欠落 + 警告
                    Err(_) => {
                        warnings.push(format!(
                            "{file_name}: skipped because the PNG header could not be read"
                        ));
                    }
                },
            }
        }

        // サイズ混在は 1 件の警告で続行（Frame は各自の寸法を保持）
        let sizes: BTreeSet<(u32, u32)> = rgba_frames.values().map(|f| f.dimensions()).collect();
        if sizes.len() > 1 {
            let dims: Vec<String> = sizes.iter().map(|(w, h)| format!("{w}x{h}")).collect();
            warnings.push(format!(
                "{set_name}: mixed frame dimensions ({}). Each frame was loaded at its own dimensions",
                dims.join(", ")
            ));
        }

        // scale 指定（1.0 以外）なら全フレームを straight RGBA8 のままプリスケール
        if let Some(s) = scale.filter(|&s| s != 1.0) {
            let mut oversized: Vec<String> = Vec::new();
            for (file_name, src) in rgba_frames.iter_mut() {
                let new_width = scaled_dimension(src.width(), s);
                let new_height = scaled_dimension(src.height(), s);
                // resize 実行前に上限検査（巨大 alloc / u32 オーバーフロー回避）
                if !valid_frame_dimensions(new_width, new_height) {
                    oversized.push(file_name.clone());
                    continue;
                }
                *src = image::imageops::resize(
                    src,
                    new_width,
                    new_height,
                    image::imageops::FilterType::Lanczos3,
                );
            }
            for file_name in oversized {
                rgba_frames.remove(&file_name);
                warnings.push(format!(
                    "{file_name}: skipped because dimensions after applying scale {s} exceed the limit"
                ));
            }
        }

        // straight RGBA8 → プレマルチプライド ARGB（ロード時 1 回）
        let frames: BTreeMap<String, Frame> = rgba_frames
            .into_iter()
            .map(|(file_name, img)| {
                let (width, height) = img.dimensions();
                (file_name, Frame::from_rgba(width, height, img.into_raw()))
            })
            .collect();

        Ok(ImageSet {
            name: set_name.to_string(),
            frames,
            warnings,
            scale: resolved_scale,
        })
    }

    /// image 参照でフレームを引く（先頭 '/' は正規化して許容）。
    pub fn frame(&self, image_ref: &str) -> Option<&Frame> {
        self.frames.get(normalize_image_ref(image_ref))
    }

    /// セル寸法の基礎: scale 適用後の最大フレーム寸法。
    /// frames が空（全欠落で警告のみの経路）のときは (0, 0)。呼び出し側で 1 に clamp する。
    pub fn max_frame_size(&self) -> (u32, u32) {
        self.frames
            .values()
            .fold((0, 0), |acc, f| (acc.0.max(f.width), acc.1.max(f.height)))
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
                "action '{action_name}' animation #{index}: disabled because image {} was not found",
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
