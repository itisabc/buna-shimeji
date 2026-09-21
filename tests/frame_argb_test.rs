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
//!   あわせてグロー用の α ブラー層 `glow`（1 画素 1 バイト・`argb` と同じ並び）を
//!   生成する。空入力なら `glow` も空。
//! - `win::window::blit_argb_at(dst, dst_w, dst_h, src, src_w, src_h, at, flip, tint) -> ()`:
//!   dst 全体を 0 クリアし、src を `at` に flip 付きで配置する（はみ出しはクリップ）。
//!   `src` 空は no-op。プレマルチプライは済んでいる前提で、flip は転送コピーに融合する。
//! - `win::window::blit_add_argb_at(dst, dst_size, glow, at, flip)`:
//!   `GlowLayer`（α ブラー層 + 寸法 + `color` + α 倍率 `strength`）を加算合成する。
//!   dst はクリアしない（blit の後に呼ぶ）。チャンネル毎の飽和加算で `r,g,b <= a` を保つ。
//! - `ImageSet::load` は frames にプレマルチプライド `Frame` を格納する
//!   （実資産 1 枚で参照一致を確認）。
//!
//! TDD RED: 上記 `argb` フィールド / `from_rgba` / `blit_argb_at` は未実装のため
//! 「解決できない名前」「フィールドが存在しない」のコンパイルエラーになるのが正常。
//!
//! 網羅的な番号リストや見た目テストは行わない。契約（入力→振る舞い）の要点のみ。

use std::path::{Path, PathBuf};

use shimeji::render::imageset::{Frame, ImageSet};
use shimeji::win::window::{blit_add_argb_at, blit_argb_at, premultiply_rgba_to_argb, GlowLayer};

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
// 契約 2: blit_argb_at は「全クリア → at に配置」/ クリップ / 行内水平反転
// =====================================================================

/// 同寸法・at=(0,0)・flip=false は dst が src と完全一致。
#[test]
fn blit_argb_at_same_size_copies_src() {
    let src: Vec<u32> = vec![0x8005_0A0F, 0xFFFF_0000, 0x0000_0000, 0x4010_2030];
    let mut dst = vec![0u32; src.len()];
    blit_argb_at(&mut dst, (2, 2), &src, (2, 2), (0, 0), false, None);
    assert_eq!(dst, src, "同寸法 at=(0,0) は完全コピー");
}

/// 2×2: at=(0,0)・flip=true は各行を水平反転する（バッファ全体反転ではない）。
#[test]
fn blit_argb_at_flip_reverses_each_row_2x2() {
    // straight RGBA の 2×2 を先にプレマルチプライして src とする。
    let rgba: Vec<u8> = vec![
        10, 20, 30, 128, // A 行0左
        255, 0, 0, 255, // B 行0右
        64, 128, 192, 64, // C 行1左
        0, 0, 0, 0, // D 行1右
    ];
    let src = reference_argb(&rgba);
    let mut dst = vec![0u32; src.len()];
    blit_argb_at(&mut dst, (2, 2), &src, (2, 2), (0, 0), true, None);

    // 行0: [B, A] / 行1: [D, C]
    let expected = vec![0xFFFF_0000, 0x8005_0A0F, 0x0000_0000, 0x4010_2030];
    assert_eq!(dst, expected, "2×2 の行内水平反転");
}

/// flip の正しさの基準: 元 RGBA をプレマルチプライして得た ARGB を、行ごとに
/// 水平反転したものと一致すること（プレマルチプライと反転は独立に交換可能）。
#[test]
fn blit_argb_at_flip_matches_premultiply_of_flipped_rgba() {
    let (width, height) = (3u32, 2u32);
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
    blit_argb_at(
        &mut dst,
        (width, height),
        &src,
        (width, height),
        (0, 0),
        true,
        None,
    );

    // 参照: 各行を逆順に並べた straight RGBA をプレマルチプライ
    let mut flipped_rgba = Vec::with_capacity(rgba.len());
    for row in rgba.chunks_exact((width * 4) as usize) {
        for pixel in row.chunks_exact(4).rev() {
            flipped_rgba.extend_from_slice(pixel);
        }
    }
    let expected = reference_argb(&flipped_rgba);
    assert_eq!(dst, expected, "flip はプレマルチプライと交換可能");
}

/// オフセット配置: src は `at` に置かれ、それ以外（および行末）は 0 にクリアされる。
#[test]
fn blit_argb_at_offset_places_sprite_and_clears_rest() {
    // src = 2×1 [A, B]、dst = 4×2 を非 0 で初期化
    let src = reference_argb(&[10, 20, 30, 128, 255, 0, 0, 255]);
    let mut dst = vec![0xDEAD_BEEFu32; 4 * 2];
    blit_argb_at(&mut dst, (4, 2), &src, (2, 1), (1, 0), false, None);

    assert_eq!(dst[0], 0, "x=0 はクリア");
    assert_eq!(dst[1], src[0], "src 左が x=1 に配置");
    assert_eq!(dst[2], src[1], "src 右が x=2 に配置");
    assert_eq!(dst[3], 0, "x=3 はクリア");
    assert_eq!(&dst[4..8], &[0, 0, 0, 0], "y=1 は全クリア");
}

/// はみ出しはクリップする（負の at / dst 外への張り出し・panic しない）。
#[test]
fn blit_argb_at_clips_out_of_bounds() {
    // src = 2×2 [1,2; 3,4]
    let src: Vec<u32> = vec![1, 2, 3, 4];

    // at=(-1,-1): src(1,1)=4 だけが dst(0,0) に載る
    let mut dst = vec![0u32; 4];
    blit_argb_at(&mut dst, (2, 2), &src, (2, 2), (-1, -1), false, None);
    assert_eq!(dst, vec![4, 0, 0, 0], "左上はみ出しのクリップ");

    // at=(1,1): src(0,0)=1 だけが dst(1,1) に載る
    let mut dst2 = vec![0u32; 4];
    blit_argb_at(&mut dst2, (2, 2), &src, (2, 2), (1, 1), false, None);
    assert_eq!(dst2, vec![0, 0, 0, 1], "右下はみ出しのクリップ");

    // 完全に外: 全て 0
    let mut dst3 = vec![9u32; 4];
    blit_argb_at(&mut dst3, (2, 2), &src, (2, 2), (10, 10), false, None);
    assert_eq!(dst3, vec![0, 0, 0, 0], "全はみ出しでもクリアはされる");
}

/// src が空 → 何もしない（no-op・クリアもしない）。
#[test]
fn blit_argb_at_empty_src_is_noop() {
    let mut dst = vec![7u32; 4];
    blit_argb_at(&mut dst, (2, 2), &[], (0, 0), (0, 0), false, None);
    assert_eq!(dst, vec![7u32; 4], "空 src は no-op（クリアしない）");
}

/// tint が None なら、従来（tint なし）と 1 バイトも変わらない。
#[test]
fn blit_argb_at_tint_none_is_identity() {
    let rgba = [10, 20, 30, 128, 255, 0, 0, 255, 0, 255, 0, 0, 1, 2, 3, 64];
    let src = reference_argb(&rgba);
    let mut dst = vec![0u32; src.len()];
    blit_argb_at(&mut dst, (4, 1), &src, (4, 1), (0, 0), false, None);
    assert_eq!(dst, src, "tint なしは src のコピー（従来と同一）");
}

/// tint はチャンネル毎の乗算（切り捨て `v * t / 255`）。α は変えない。
#[test]
fn blit_argb_at_tint_multiplies_channels() {
    // プレマルチプライド 0xAARRGGBB を直接与える（A=255, R=200, G=100, B=50）
    let src = vec![0xFF_C8_64_32u32];
    let mut dst = vec![0u32; 1];
    blit_argb_at(
        &mut dst,
        (1, 1),
        &src,
        (1, 1),
        (0, 0),
        false,
        Some([128, 255, 0]),
    );
    let out = dst[0];
    assert_eq!(out & 0xFF00_0000, 0xFF00_0000, "α は変えない");
    assert_eq!(
        (out >> 16) & 0xFF,
        200 * 128 / 255,
        "R は 128/255 倍（切り捨て）"
    );
    assert_eq!((out >> 8) & 0xFF, 100 * 255 / 255, "G は不変");
    assert_eq!(out & 0xFF, 0, "B は 0 倍");
}

/// tint 後もプレマルチプライドの不変条件（r, g, b <= a）が保たれる。
#[test]
fn blit_argb_at_tint_keeps_premultiplied_invariant() {
    let src: Vec<u32> = (0..=255u32)
        .map(|a| {
            let r = 200 * a / 255;
            let g = 100 * a / 255;
            let b = 50 * a / 255;
            (a << 24) | (r << 16) | (g << 8) | b
        })
        .collect();
    let width = src.len() as u32;
    let mut dst = vec![0u32; src.len()];
    blit_argb_at(
        &mut dst,
        (width, 1),
        &src,
        (width, 1),
        (0, 0),
        false,
        Some([255, 255, 255]),
    );
    assert_eq!(dst, src, "全 255 の tint は恒等");
    for (i, &p) in dst.iter().enumerate() {
        let a = (p >> 24) & 0xFF;
        assert!(((p >> 16) & 0xFF) <= a, "R <= A (i={i})");
        assert!(((p >> 8) & 0xFF) <= a, "G <= A (i={i})");
        assert!((p & 0xFF) <= a, "B <= A (i={i})");
    }
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

/// 実資産のフレームにもグロー層が付く（1 画素 1 バイト・塗りの内側は 255 に達し、
/// 輪郭の外側はぼけて中間値になる）。
#[test]
fn image_set_load_generates_glow_layer() {
    let set =
        ImageSet::load(&assets_img(), "Shimeji", None).expect("実資産 img/Shimeji をロードできる");
    let frame = set
        .frame("shime1.png")
        .expect("実資産に shime1.png が存在する");

    assert_eq!(frame.glow.len(), 128 * 128, "glow 長 = width*height");
    assert_eq!(
        frame.glow.iter().copied().max(),
        Some(255),
        "不透明な塗りの内側はぼけても 255 のまま"
    );
    assert!(
        frame.glow.iter().any(|&g| g > 0 && g < 255),
        "輪郭の外側に中間値（ぼけ）が出る"
    );
}

// =====================================================================
// 契約 4: Frame::from_rgba はグロー用の α ブラー層も作る
// =====================================================================

/// グローは α のぼかし。完全透過はどこにも広がらず、全不透明は端まで 255 のまま
///（端は clamp なので境界でも薄くならない）。
#[test]
fn frame_from_rgba_glow_is_blurred_alpha() {
    let transparent = Frame::from_rgba(4, 4, vec![0u8; 4 * 4 * 4]);
    assert_eq!(transparent.glow.len(), 16, "glow 長 = width*height");
    assert!(transparent.glow.iter().all(|&g| g == 0), "透過は広がらない");

    let opaque = Frame::from_rgba(4, 4, vec![255u8; 4 * 4 * 4]);
    assert_eq!(opaque.glow, vec![255u8; 16], "一色不透明は端まで 255");
}

/// 1 画素だけ不透明にすると周囲へ広がり、元画素が最も明るく、遠いほど薄くなる。
#[test]
fn frame_from_rgba_glow_spreads_from_a_single_pixel() {
    let mut rgba = vec![0u8; 8 * 8 * 4];
    rgba[3] = 255; // (0,0) だけ不透明
    let frame = Frame::from_rgba(8, 8, rgba);
    let at = |x: usize, y: usize| frame.glow[y * 8 + x];

    assert!(at(0, 0) > 0, "元画素は光る");
    assert!(at(0, 0) < 255, "1 画素はぼけて広がる（元画素も薄まる）");
    assert!(at(1, 0) > 0 && at(0, 1) > 0, "隣へ広がる");
    assert!(
        (i32::from(at(1, 0)) - i32::from(at(0, 1))).abs() <= 1,
        "縦横の広がりは同じ（分離型ブラー）: {} vs {}",
        at(1, 0),
        at(0, 1)
    );
    assert!(at(7, 7) < at(0, 0), "遠くは薄い");
}

/// グローは α だけの関数（RGB を変えても同じ）。色は `argb` 側にだけ出る。
#[test]
fn frame_from_rgba_glow_depends_only_on_alpha() {
    let mut alpha_only = vec![0u8; 4 * 4 * 4];
    let mut colored = alpha_only.clone();
    for i in 0..16 {
        alpha_only[i * 4 + 3] = 200;
        colored[i * 4 + 3] = 200;
        colored[i * 4] = 255;
        colored[i * 4 + 1] = 128;
    }
    let a = Frame::from_rgba(4, 4, alpha_only);
    let b = Frame::from_rgba(4, 4, colored);

    assert_eq!(a.glow, b.glow, "glow は α のみに依存する");
    assert_ne!(a.argb, b.argb, "色は argb 側にだけ出る");
}

/// 画素が無い入力（寸法 0 / rgba 空 / 長さ不足）は glow も空（panic しない）。
#[test]
fn frame_from_rgba_glow_is_empty_without_pixels() {
    assert!(Frame::from_rgba(0, 4, vec![0u8; 3 * 4]).glow.is_empty());
    assert!(Frame::from_rgba(2, 2, Vec::new()).glow.is_empty());
    assert!(
        Frame::from_rgba(2, 2, vec![0u8; 8]).glow.is_empty(),
        "rgba が短すぎる場合は空"
    );
}

// =====================================================================
// 契約 5: blit_add_argb_at は α ブラー層を加算合成する（blit の後・dst はクリアしない）
// =====================================================================

/// 空 glow / α 倍率 0 は no-op（dst をクリアもしない）。
#[test]
fn blit_add_argb_at_noop_cases() {
    let mut dst = vec![0xDEAD_BEEFu32; 4];
    blit_add_argb_at(
        &mut dst,
        (2, 2),
        GlowLayer {
            blur: &[],
            size: (0, 0),
            color: [255, 255, 255],
            strength: 255,
        },
        (0, 0),
        false,
    );
    assert_eq!(dst, vec![0xDEAD_BEEFu32; 4], "空 glow は no-op");

    blit_add_argb_at(
        &mut dst,
        (2, 2),
        GlowLayer {
            blur: &[255u8; 4],
            size: (2, 2),
            color: [255, 255, 255],
            strength: 0,
        },
        (0, 0),
        false,
    );
    assert_eq!(dst, vec![0xDEAD_BEEFu32; 4], "α 倍率 0 は no-op");
}

/// 加算: 不透明青の上に α=255 の赤グローを足すと赤が立ち上がり、青は残る（255 で飽和）。
#[test]
fn blit_add_argb_at_adds_color_premultiplied_by_alpha() {
    let glow = |color, strength| GlowLayer {
        blur: &[255u8],
        size: (1, 1),
        color,
        strength,
    };

    let mut dst = vec![0xFF00_00FFu32]; // A=255, B=255
    blit_add_argb_at(&mut dst, (1, 1), glow([255, 0, 0], 255), (0, 0), false);
    assert_eq!(dst[0], 0xFFFF_00FF, "A も R も飽和し、B は残る");

    // α 倍率 128 は「その画素の α」を 1/2 にし、色も同じ比率で薄く足す
    let mut dst = vec![0u32];
    blit_add_argb_at(&mut dst, (1, 1), glow([255, 255, 255], 128), (0, 0), false);
    assert_eq!(dst[0], 0x8080_8080, "α = 255*128/255, RGB = 色 × その α");
}

/// flip は glow 層を行内で水平反転して足す（blit の flip と一致）。
#[test]
fn blit_add_argb_at_flips_each_row() {
    let glow = GlowLayer {
        blur: &[255u8, 0, 0, 0], // 2×2: 左上だけ光る
        size: (2, 2),
        color: [255, 255, 255],
        strength: 255,
    };
    let mut dst = vec![0u32; 4];
    blit_add_argb_at(&mut dst, (2, 2), glow, (0, 0), true);
    assert_eq!(
        dst,
        vec![0, 0xFFFF_FFFF, 0, 0],
        "反転して右上（行内で反転した位置）が光る"
    );
}

/// はみ出しはクリップする（負の at・panic しない）。
#[test]
fn blit_add_argb_at_clips_out_of_bounds() {
    let glow = GlowLayer {
        blur: &[255u8; 4],
        size: (2, 2),
        color: [255, 255, 255],
        strength: 255,
    };
    let mut dst = vec![0u32; 4];
    blit_add_argb_at(&mut dst, (2, 2), glow, (-1, -1), false);
    assert_eq!(
        dst,
        vec![0xFFFF_FFFF, 0, 0, 0],
        "重なる (0,0) だけが足される"
    );
}

/// 加算後もプレマルチプライドの不変条件（r, g, b <= a）が保たれる。
#[test]
fn blit_add_argb_at_keeps_premultiplied_invariant() {
    let src: Vec<u32> = (0..=255u32)
        .map(|a| {
            let r = 200 * a / 255;
            let g = 100 * a / 255;
            let b = 50 * a / 255;
            (a << 24) | (r << 16) | (g << 8) | b
        })
        .collect();
    let blur: Vec<u8> = (0..=255u32).map(|i| i as u8).collect();
    let width = src.len() as u32;
    let mut dst = src.clone();
    blit_add_argb_at(
        &mut dst,
        (width, 1),
        GlowLayer {
            blur: &blur,
            size: (width, 1),
            color: [255, 200, 100],
            strength: 200,
        },
        (0, 0),
        false,
    );

    for (i, &p) in dst.iter().enumerate() {
        let a = (p >> 24) & 0xFF;
        assert!(((p >> 16) & 0xFF) <= a, "R <= A (i={i})");
        assert!(((p >> 8) & 0xFF) <= a, "G <= A (i={i})");
        assert!((p & 0xFF) <= a, "B <= A (i={i})");
    }
}
