//! タスク #31: 描画最適化リファクタの新契約テスト。
//!
//! 検証対象は実装予定の公開 API のみ:
//! - `Frame`（src/render/imageset.rs）: `pub width: u32` / `pub height: u32` /
//!   `pub argb: Vec<u32>`。`argb` はプレマルチプライド 0xAARRGGBB（行優先・非反転）。
//!   旧 `rgba: Vec<u8>` は廃止される。
//! - `Frame::from_rgba(width, height, rgba) -> Frame`:
//!   straight RGBA8 をプレマルチプライして `argb` に格納する。式は既存
//!   `premultiply_rgba_to_argb` と同一（切り捨て `(v * a) / 255`）。
//!   `width == 0` または `rgba` 空 → `argb` は空。整合入力なら長さ = width*height。
//! - `win::window::blit_argb(dst, src, width, flip) -> ()`:
//!   flip=false は完全コピー、flip=true は各行（幅 width）を水平反転。
//!   `src` 空は no-op。プレマルチプライは済んでいる前提で、flip は転送コピーに融合する。
//! - `ImageSet::load` は frames にプレマルチプライド `Frame` を格納する
//!   （実資産 1 枚で参照一致を確認）。
//!
//! TDD RED: 上記 `argb` フィールド / `from_rgba` / `blit_argb` は未実装のため
//! 「解決できない名前」「フィールドが存在しない」のコンパイルエラーになるのが正常。
//!
//! 網羅的な番号リストや見た目テストは行わない。契約（入力→振る舞い）の要点のみ。

use std::path::{Path, PathBuf};

use shimeji::render::imageset::{Frame, ImageSet};
use shimeji::win::window::{blit_argb, premultiply_rgba_to_argb};

// =====================================================================
// 共通ヘルパ
// =====================================================================

/// 実資産の img/ ディレクトリ（既存 imageset_test.rs の流儀を踏襲）。
fn assets_img() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("img")
}

/// straight RGBA8 の生バイトをプレマルチプライした参照値（公開 API をそのまま使用）。
fn reference_argb(rgba: &[u8]) -> Vec<u32> {
    premultiply_rgba_to_argb(rgba)
}

// =====================================================================
// 契約 1: Frame::from_rgba は straight RGBA をプレマルチプライド ARGB にする
// =====================================================================

/// 焼き込みプローブ + 参照関数一致:
/// `from_rgba` の `argb` は `premultiply_rgba_to_argb` の結果に一致する。
/// 半透明（α=128）・不透明（α=255）・完全透過（α=0）を含める。
#[test]
fn frame_from_rgba_premultiplies_straight_rgba() {
    let rgba: Vec<u8> = vec![
        10, 20, 30, 128, // 半透明 → 5,10,15
        255, 0, 0, 255, // 不透明
        0, 0, 0, 0, // 完全透過
        64, 128, 192, 64, // 半透明 → 16,32,48
    ];
    let frame = Frame::from_rgba(2, 2, rgba.clone());

    assert_eq!(
        frame.argb,
        reference_argb(&rgba),
        "argb == premultiply_rgba_to_argb(straight rgba)"
    );
    // 焼き込みで自己参照を回避（式 `(a<<24)|((r*a/255)<<16)|((g*a/255)<<8)|(b*a/255)`）
    let expected = vec![0x8005_0A0F, 0xFFFF_0000, 0x0000_0000, 0x4010_2030];
    assert_eq!(frame.argb, expected, "各行優先の焼き込み値");
}

/// 整合入力なら `argb` 長 = width*height。
#[test]
fn frame_from_rgba_length_matches_dimensions() {
    let (width, height) = (2u32, 3u32);
    let rgba = vec![7u8; (width * height * 4) as usize];
    let frame = Frame::from_rgba(width, height, rgba);
    assert_eq!(
        frame.argb.len(),
        (width * height) as usize,
        "argb 長 = width*height"
    );
    assert_eq!((frame.width, frame.height), (width, height));
}

/// `width == 0` または `rgba` 空 → `argb` は空（panic しない）。
#[test]
fn frame_from_rgba_empty_or_zero_width_yields_empty_argb() {
    let zero_width = Frame::from_rgba(0, 4, vec![1, 2, 3, 4]);
    assert!(zero_width.argb.is_empty(), "width == 0 → argb 空");

    let empty_rgba = Frame::from_rgba(2, 2, Vec::new());
    assert!(empty_rgba.argb.is_empty(), "rgba 空 → argb 空");
}

// =====================================================================
// 契約 2: blit_argb はコピー / 行内水平反転
// =====================================================================

/// flip=false は dst が src と完全一致（単純コピー）。
#[test]
fn blit_argb_without_flip_copies_src() {
    let src: Vec<u32> = vec![0x8005_0A0F, 0xFFFF_0000, 0x0000_0000, 0x4010_2030];
    let mut dst = vec![0u32; src.len()];
    blit_argb(&mut dst, &src, 2, false);
    assert_eq!(dst, src, "flip=false は完全コピー");
}

/// 2×2: flip=true は各行を水平反転する（バッファ全体反転ではない）。
#[test]
fn blit_argb_flip_reverses_each_row_2x2() {
    // straight RGBA の 2×2 を先にプレマルチプライして src とする。
    let rgba: Vec<u8> = vec![
        10, 20, 30, 128, // A 行0左
        255, 0, 0, 255, // B 行0右
        64, 128, 192, 64, // C 行1左
        0, 0, 0, 0, // D 行1右
    ];
    let src = reference_argb(&rgba);
    let mut dst = vec![0u32; src.len()];
    blit_argb(&mut dst, &src, 2, true);

    // 行0: [B, A] / 行1: [D, C]
    let expected = vec![0xFFFF_0000, 0x8005_0A0F, 0x0000_0000, 0x4010_2030];
    assert_eq!(dst, expected, "2×2 の行内水平反転");
}

/// flip の正しさの基準: 元 RGBA をプレマルチプライして得た ARGB を、行ごとに
/// 水平反転したものと一致すること（プレマルチプライと反転は独立に交換可能）。
/// 参照側で行反転した straight RGBA を作り `premultiply_rgba_to_argb` と比較する。
#[test]
fn blit_argb_flip_matches_premultiply_of_flipped_rgba() {
    let (width, height) = (3usize, 2usize);
    let rgba: Vec<u8> = vec![
        1, 0, 0, 255, // P
        10, 20, 30, 128, // Q
        3, 0, 0, 255, // R
        4, 0, 0, 255, // S
        5, 0, 0, 255, // T
        6, 0, 0, 255, // U
    ];
    let src = reference_argb(&rgba);

    let mut dst = vec![0u32; src.len()];
    blit_argb(&mut dst, &src, width, true);

    // 参照: 各行を逆順に並べた straight RGBA をプレマルチプライ
    let mut flipped_rgba = Vec::with_capacity(rgba.len());
    for row in rgba.chunks_exact(width * 4) {
        for pixel in row.chunks_exact(4).rev() {
            flipped_rgba.extend_from_slice(pixel);
        }
    }
    let expected = reference_argb(&flipped_rgba);
    assert_eq!((width, height), (3, 2), "非正方・行単位の前提");
    assert_eq!(dst, expected, "flip はプレマルチプライと交換可能");
}

/// src が空 → 何もしない（no-op・panic しない）。
#[test]
fn blit_argb_empty_src_is_noop() {
    let mut dst: Vec<u32> = Vec::new();
    blit_argb(&mut dst, &[], 0, false);
    assert!(dst.is_empty(), "flip=false で no-op");
    blit_argb(&mut dst, &[], 0, true);
    assert!(dst.is_empty(), "flip=true で no-op");
}

// =====================================================================
// 契約 3: ImageSet::load は frames にプレマルチプライド Frame を格納する
// =====================================================================

/// 実資産 Shimeji の代表 1 枚（shime1.png）をロードし、`argb` が
/// straight RGBA をプレマルチプライしたものに一致することを確認する。
#[test]
fn image_set_load_stores_premultiplied_argb() {
    let set =
        ImageSet::load(&assets_img(), "Shimeji", None).expect("実資産 img/Shimeji をロードできる");
    let frame = set
        .frame("shime1.png")
        .expect("実資産に shime1.png が存在する");
    assert_eq!(
        (frame.width, frame.height),
        (128, 128),
        "shime1.png は 128×128"
    );

    // 元 PNG をテスト側で straight RGBA8 にデコードし、参照プレマルチプライ値と比較
    let decoded = image::open(assets_img().join("Shimeji").join("shime1.png"))
        .expect("shime1.png をデコードできる")
        .to_rgba8();
    let expected = reference_argb(&decoded.into_raw());
    assert_eq!(frame.argb.len(), 128 * 128, "argb 長 = width*height");
    assert_eq!(
        frame.argb, expected,
        "load はプレマルチプライド argb を格納する"
    );
}
