//! タスク #5: src/render/mod.rs の描画 compose 純粋関数の契約テスト。
//!
//! 検証対象は coder が実装する公開 API（シグネチャ固定）のみ:
//! - `compose_argb(frame: &Frame, flip: bool) -> Vec<u32>`:
//!   straight RGBA8 をプレマルチプライした 0xAARRGGBB（u32・上位バイト α）を返す。
//!   式は `win::window::premultiply_rgba_to_argb` と同一（整数切り捨て `(v * a) / 255`）。
//!   出力長 = width * height・行優先
//! - flip=true は各行内での水平反転（行単位。非正方 3×2 でも行ごと）。
//!   flip とプレマルチプライはピクセル独立演算のため順序不変
//! - `flipped_offset_x(frame_width: u32, pose_dx: i32) -> i32`:
//!   ImagePairs.java L85 の右画像アンカー相当 `width - dx`（dx は負でもそのまま演算）
//! - 統合: 実資産 img/Shimeji を ImageSet::load(None) し、代表 1 枚
//!   （shime1.png）の compose_argb(frame, false) の長さが 128*128
//!
//! 実資産の扱いは集計レベル（件数・長さ・代表 1 枚の集計 assert）のみ。
//! 全枚ループ・全ピクセル走査・ハッシュ照合は行わない。ピクセル内容の検査は
//! 合成 Frame（1×1 / 2×2 / 2×3 / 3×2）の契約プローブに限定。
//! MascotView（Win32 ウィンドウ制御）と DrawAction 等の enum 判定はテストしない。
//!
//! TDD RED: compose_argb / flipped_offset_x は未実装のため
//! 「解決できない名前」のコンパイルエラーになることが正常。

use std::path::Path;

use shimeji::render::imageset::{Frame, ImageSet};
use shimeji::render::{compose_argb, flipped_offset_x};
use shimeji::win::window::premultiply_rgba_to_argb;

// =====================================================================
// 共通ヘルパ
// =====================================================================

/// 1×1 のプローブ用 Frame（合成 Frame はテスト内で直接構築。
/// Frame は Clone+Debug を持つが、clone は不要）。
fn probe_frame(px: [u8; 4]) -> Frame {
    frame_of(1, 1, px.to_vec())
}

/// 幅 width・高さ height・ピクセル列 rgba から Frame を直接構築する。
/// rgba 長 = width*height*4 を assert してテスト側 latent bug を早期検出。
fn frame_of(width: u32, height: u32, rgba: Vec<u8>) -> Frame {
    assert_eq!(
        rgba.len(),
        width as usize * height as usize * 4,
        "テスト側の rgba 長が寸法と不整合"
    );
    Frame {
        width,
        height,
        rgba,
    }
}

// =====================================================================
// 契約 1: compose_argb は straight RGBA をプレマルチプライした ARGB を返す
// =====================================================================

/// 焼き込みプローブ: 式 `(a<<24) | ((r*a/255)<<16) | ((g*a/255)<<8) | (b*a/255)`
/// （整数切り捨て）で算出済みの 4 色。flip=false。
#[test]
fn compose_argb_premultiplies_straight_rgba() {
    let probes: [([u8; 4], u32); 4] = [
        // r=1280/255=5, g=2560/255=10, b=3840/255=15, a=128
        ([10, 20, 30, 128], 0x80050A0F),
        // 不透過はそのまま（r=255, g=0, b=0, a=255）
        ([255, 0, 0, 255], 0xFFFF0000),
        // 全 0 は完全透過（値 0）
        ([0, 0, 0, 0], 0x0000_0000),
        // r=4096/255=16, g=8192/255=32, b=12288/255=48, a=64
        ([64, 128, 192, 64], 0x40102030),
    ];
    for (rgba, expected) in probes {
        let out = compose_argb(&probe_frame(rgba), false);
        assert_eq!(out.len(), 1, "1×1 の出力長は 1: rgba={rgba:?}");
        assert_eq!(out[0], expected, "rgba={rgba:?}");
    }
}

/// 出力長 = width*height、行優先（index = y*width + x）。
/// 2×3 の合成 Frame で 6 ピクセルすべての配置を確認（実資産走査ではない）。
#[test]
fn compose_argb_output_length_and_row_major_layout() {
    let (width, height) = (2u32, 3u32);
    // ピクセル (x, y) = [x, y, 0, 255]。プレマルチプライ後は
    // 0xFF000000 | (x << 16) | (y << 8)（不透過なので成分は不変）。
    let mut rgba = Vec::new();
    for y in 0..height {
        for x in 0..width {
            rgba.extend_from_slice(&[x as u8, y as u8, 0, 255]);
        }
    }
    let out = compose_argb(&frame_of(width, height, rgba), false);
    assert_eq!(
        out.len(),
        (width * height) as usize,
        "出力長 = width*height"
    );
    for y in 0..height {
        for x in 0..width {
            let expected = 0xFF00_0000u32 | (x << 16) | (y << 8);
            assert_eq!(out[(y * width + x) as usize], expected, "(x={x}, y={y})");
        }
    }
}

// =====================================================================
// 契約 2: flip=true は各行内で水平反転（行単位・順序不変）
// =====================================================================

/// 2×2: 行内 p0p1 → p1p0。flip=false との差分も確認（flip が no-op でないこと）。
/// A=[10,20,30,128], B=[255,0,0,255], C=[64,128,192,64], D=[0,0,0,0]。
#[test]
fn compose_argb_flip_reverses_each_row_2x2() {
    let frame = frame_of(
        2,
        2,
        vec![
            10, 20, 30, 128, // A 行目左
            255, 0, 0, 255, // B 行目右
            64, 128, 192, 64, // C 2 行目左
            0, 0, 0, 0, // D 2 行目右
        ],
    );
    let flipped = compose_argb(&frame, true);
    // 行 0: [B, A] / 行 1: [D, C]
    assert_eq!(
        flipped,
        vec![0xFFFF0000, 0x80050A0F, 0x0000_0000, 0x40102030],
        "flip=true は行内反転"
    );
    let not_flipped = compose_argb(&frame, false);
    assert_eq!(
        not_flipped,
        vec![0x80050A0F, 0xFFFF0000, 0x40102030, 0x0000_0000],
        "flip=false は元の順序"
    );
}

/// 3×2 非正方でも反転は行単位（幅 3 ごと。バッファ全体反転や列単位でないこと）。
/// 半透明 Q を含め、flip とプレマルチプライの同時適用も確認する。
#[test]
fn compose_argb_flip_reverses_each_row_3x2_non_square() {
    let frame = frame_of(
        3,
        2,
        vec![
            1, 0, 0, 255, // P → 0xFF010000
            10, 20, 30, 128, // Q → 0x80050A0F
            3, 0, 0, 255, // R → 0xFF030000
            4, 0, 0, 255, // S → 0xFF040000
            5, 0, 0, 255, // T → 0xFF050000
            6, 0, 0, 255, // U → 0xFF060000
        ],
    );
    let flipped = compose_argb(&frame, true);
    // 行 0: [R, Q, P] / 行 1: [U, T, S]（バッファ全体反転 [U,T,S,R,Q,P] とは異なる）
    assert_eq!(
        flipped,
        vec![0xFF030000, 0x80050A0F, 0xFF010000, 0xFF060000, 0xFF050000, 0xFF040000,],
        "非正方 3×2 でも行内反転"
    );
}

/// flip とプレマルチプライはピクセル独立のため順序不変:
/// compose_argb(frame, true) == premultiply(行内反転した rgba)。
/// 既存公開 API premultiply_rgba_to_argb を期待値計算に使用し、
/// 少なくとも 1 要素は焼き込み値で固定して自己参照を回避する。
#[test]
fn compose_argb_flip_equals_premultiply_of_flipped_rgba() {
    let frame = frame_of(
        2,
        2,
        vec![
            10, 20, 30, 128, // A
            255, 0, 0, 255, // B
            64, 128, 192, 64, // C
            0, 0, 0, 0, // D
        ],
    );
    let flipped_rgba = vec![
        255, 0, 0, 255, // B
        10, 20, 30, 128, // A
        0, 0, 0, 0, // D
        64, 128, 192, 64, // C
    ];
    let expected = premultiply_rgba_to_argb(&flipped_rgba);
    assert_eq!(
        expected[0], 0xFFFF0000,
        "期待値の固定（式の一致を焼き込みで担保）"
    );
    assert_eq!(
        compose_argb(&frame, true),
        expected,
        "flip とプレマルチプライの適用順序に依存しない"
    );
}

// =====================================================================
// 契約 3: flipped_offset_x は ImagePairs.java L85 相当 `width - dx`
// =====================================================================

/// 焼き込み値（ImagePairs.java L85: `rightImage.getWidth() - scaledAnchorX`）。
/// dx は負でもそのまま演算する（Java int 演算相当）。
#[test]
fn flipped_offset_x_is_width_minus_dx() {
    assert_eq!(flipped_offset_x(128, 64), 64);
    assert_eq!(flipped_offset_x(128, 96), 32);
    assert_eq!(flipped_offset_x(128, 0), 128);
    assert_eq!(flipped_offset_x(128, 128), 0);
    // 負の dx もそのまま width - dx
    assert_eq!(flipped_offset_x(100, -20), 120);
}

// =====================================================================
// 契約 4: 統合（実資産・代表 1 枚の集計 assert のみ）
// =====================================================================

/// 実資産 img/Shimeji を ImageSet::load(None) し、代表 1 枚 shime1.png の
/// compose_argb(frame, false) の出力長が 128*128。
/// 全枚ループ・全ピクセル走査・ハッシュ照合は行わない。
#[test]
fn compose_argb_real_asset_shime1_output_length() {
    let set = ImageSet::load(Path::new("img"), "Shimeji", None)
        .expect("実資産 img/Shimeji をロードできる");
    let frame = set
        .frame("shime1.png")
        .expect("実資産に shime1.png が存在する");
    // 事前条件（代表 1 枚の寸法）
    assert_eq!(
        (frame.width, frame.height),
        (128, 128),
        "shime1.png は 128×128"
    );
    let out = compose_argb(frame, false);
    assert_eq!(out.len(), 128 * 128, "compose 出力長 = width*height");
}
