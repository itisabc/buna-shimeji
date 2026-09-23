//! ルート `<Mascot>` の色づけ宣言（`Tint` / `TintSpeed` / `TintSat` / `TintLum` /
//! `TintGlow` / `TintSweep`）のパースと、色の変換のテスト。
//!
//! 設計: `docs/plans/design-gaming-color.md`（ローカル専用）

use std::path::PathBuf;

use shimeji::config::parse_actions;
use shimeji::tint::{
    hex_to_hue, hsl_to_rgb, hue_to_palette_index, palette_hue, ColorSet, Sweep, TintMode,
    TintStyle, PALETTE_HUES, PALETTE_LEN,
};

fn temp_actions(tag: &str, root_attrs: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("shimeji_tint_{}_{tag}.xml", std::process::id()));
    let content = format!(
        "<Mascot xmlns=\"http://www.group-finity.com/Mascot\"{root_attrs}>\n\
         <ActionList>\n\
         <Action Name=\"Stand\" Type=\"Stay\" BorderType=\"Floor\">\n\
         <Animation><Pose Image=\"/shime1.png\" ImageAnchor=\"64,128\" Velocity=\"0,0\" Duration=\"10\"/></Animation>\n\
         </Action>\n\
         </ActionList>\n\
         </Mascot>\n"
    );
    std::fs::write(&path, content).expect("一時 actions.xml を書ける");
    path
}

fn tint_of(tag: &str, root_attrs: &str) -> TintStyle {
    let path = temp_actions(tag, root_attrs);
    parse_actions(&path)
        .unwrap_or_else(|e| panic!("parse failed: {e}"))
        .tint
}

// =====================================================================
// 宣言のパース
// =====================================================================

/// 属性が無ければ色づけなし（既存 set が影響を受けない）。
#[test]
fn absent_attributes_mean_off() {
    let tint = tint_of("absent", "");
    assert_eq!(tint, TintStyle::default(), "未指定は既定");
    assert_eq!(tint.mode, TintMode::Off);
    assert_eq!(tint.rotate, 0.0, "off では回さない");
}

/// 全部書いたときの値がそのまま入る。
#[test]
fn full_declaration_is_parsed() {
    let tint = tint_of(
        "full",
        " Tint=\"rainbow\" TintSpeed=\"200\" TintSat=\"35\" TintLum=\"78\" TintGlow=\"0.5\" TintSweep=\"steps\"",
    );
    assert_eq!(tint.mode, TintMode::Cycle);
    assert_eq!(tint.rotate, 200.0);
    assert_eq!(tint.sat, 35.0);
    assert_eq!(tint.lum, 78.0);
    assert_eq!(tint.glow, 0.5);
    assert_eq!(tint.sweep, Sweep::Steps);
}

/// `Tint="rainbow"` で速度未指定なら既定 150°/s。
#[test]
fn cycle_defaults_to_150_per_second() {
    let tint = tint_of("cycle-default", " Tint=\"rainbow\"");
    assert_eq!(tint.mode, TintMode::Cycle);
    assert_eq!(tint.rotate, 150.0);
    assert_eq!(tint.sat, 100.0);
    assert_eq!(tint.lum, 62.0);
    assert_eq!(tint.glow, 1.0);
    assert_eq!(tint.sweep, Sweep::Within, "sweep の既定は within");
}

/// `Tint="#RRGGBB"` は固定色の色相になる。`random` は回さない。
#[test]
fn fixed_and_random_modes_do_not_rotate() {
    let fixed = tint_of("fixed", " Tint=\"#00ff00\"");
    assert_eq!(fixed.mode, TintMode::Fixed(120.0));
    assert_eq!(fixed.rotate, 0.0);

    let random = tint_of("random", " Tint=\"random\"");
    assert_eq!(random.mode, TintMode::Random);
    assert_eq!(random.rotate, 0.0);
}

/// 未知の値は off へフォールバック（起動は止めない）。
#[test]
fn unknown_tint_value_falls_back_to_off() {
    let tint = tint_of("unknown", " Tint=\"glitter\"");
    assert_eq!(tint.mode, TintMode::Off);
}

/// 数値のパース失敗・非有限も既定へフォールバック。
#[test]
fn invalid_numbers_fall_back_to_defaults() {
    let tint = tint_of(
        "bad-numbers",
        " Tint=\"rainbow\" TintSpeed=\"fast\" TintSat=\"NaN\"",
    );
    assert_eq!(tint.rotate, 150.0, "不正な速度は既定");
    assert_eq!(tint.sat, 100.0, "NaN は既定");
}

/// `TintSweep` の値と既定。
#[test]
fn sweep_values() {
    assert_eq!(
        tint_of("sweep-full", " Tint=\"rainbow\" TintSweep=\"full\"").sweep,
        Sweep::Full
    );
    assert_eq!(
        tint_of("sweep-steps", " Tint=\"rainbow\" TintSweep=\"steps\"").sweep,
        Sweep::Steps
    );
    assert_eq!(
        tint_of("sweep-bad", " Tint=\"rainbow\" TintSweep=\"zigzag\"").sweep,
        Sweep::Within
    );
}

// =====================================================================
// 色の変換
// =====================================================================

/// 16 進 → 色相。赤 0 / 緑 120 / 青 240。不正入力は None。
#[test]
fn hex_to_hue_known_values() {
    assert_eq!(hex_to_hue("#ff0000"), Some(0.0));
    assert_eq!(hex_to_hue("#00ff00"), Some(120.0));
    assert_eq!(hex_to_hue("#0000ff"), Some(240.0));
    assert_eq!(hex_to_hue("ff0000"), Some(0.0), "# なしも許す");
    assert_eq!(hex_to_hue("#808080"), Some(0.0), "彩度 0 は 0");
    for bad in ["", "#12345", "#1234567", "#gggggg", "blue", "#"] {
        assert_eq!(hex_to_hue(bad), None, "不正: {bad}");
    }
}

/// `hsl_to_rgb` は既知の値を返し、`hsl(0, 100%, 62%)` がプロトタイプの
/// パレット先頭 `#ff3d3d` と一致する（canvas の hsl と同じ結果）。
#[test]
fn hsl_to_rgb_known_values() {
    assert_eq!(hsl_to_rgb(0.0, 100.0, 50.0), [255, 0, 0]);
    assert_eq!(hsl_to_rgb(120.0, 100.0, 50.0), [0, 255, 0]);
    assert_eq!(hsl_to_rgb(240.0, 100.0, 50.0), [0, 0, 255]);
    assert_eq!(hsl_to_rgb(0.0, 100.0, 62.0), [255, 61, 61], "#ff3d3d");
    assert_eq!(hsl_to_rgb(0.0, 0.0, 50.0), [128, 128, 128], "彩度 0 は灰");
    assert_eq!(
        hsl_to_rgb(360.0, 100.0, 50.0),
        [255, 0, 0],
        "360 は 0 と同じ"
    );
    assert_eq!(
        hsl_to_rgb(-120.0, 100.0, 50.0),
        [0, 0, 255],
        "負の色相は wrap する"
    );
}

// =====================================================================
// グロー強度（描画時に加算合成する α 倍率）
// =====================================================================

/// `glow_alpha` は gain（シルエットの白飛び防止）を掛けた α 倍率（0..=255）。
/// ムード表の宣言値がそのまま入る: ゲーミング 1.4 / パステル 0.5。
#[test]
fn glow_alpha_scales_with_gain_and_clamps() {
    let cycle = |glow: f32| TintStyle {
        mode: TintMode::Cycle,
        glow,
        ..TintStyle::default()
    };
    assert_eq!(cycle(0.0).glow_alpha(), 0, "0 は光らない");
    assert_eq!(cycle(1.0).glow_alpha(), 102, "既定 1.0 × gain 0.4");
    assert_eq!(cycle(1.4).glow_alpha(), 143, "ゲーミングの宣言値");
    assert_eq!(cycle(0.5).glow_alpha(), 51, "パステルの宣言値");
    assert_eq!(cycle(-1.0).glow_alpha(), 0, "負値は 0");
    assert_eq!(cycle(10.0).glow_alpha(), 255, "上限で飽和する");
    assert_eq!(cycle(f32::NAN).glow_alpha(), 0, "NaN は 0");
}

/// 色づけなし（`Off`）は色を持たないため光らせない（従来の set は影響を受けない）。
#[test]
fn glow_alpha_is_zero_without_color() {
    let off = TintStyle {
        glow: 1.4,
        ..TintStyle::default()
    };
    assert_eq!(off.mode, TintMode::Off);
    assert_eq!(off.glow_alpha(), 0);
}

// =====================================================================
// パレット（出現を許可する色の単位）
// =====================================================================

/// パレットは 12 色・30° 刻み・色相順（色相 = 添字 × 30）。表示名は辞書が持つ。
#[test]
fn palette_is_twelve_hues_at_thirty_degree_steps() {
    assert_eq!(PALETTE_LEN, 12, "コア固定の 12 色");
    for (index, hue) in PALETTE_HUES.iter().enumerate() {
        assert_eq!(*hue, index as u32 * 30, "色相 = 添字 × 30");
        assert_eq!(palette_hue(index), Some(*hue));
    }
    assert_eq!(palette_hue(PALETTE_LEN), None, "範囲外は None");
}

/// 色相 → 最も近いパレット添字。境界 15° は上の色、負値・360 以上は wrap、NaN は 0。
#[test]
fn hue_to_palette_index_rounds_to_nearest_step() {
    for (hue, expected) in [
        (0.0, 0),
        (14.9, 0),
        (15.0, 1),
        (29.9, 1),
        (45.0, 2),
        (344.9, 11),
        (345.0, 0),
        (359.9, 0),
        (360.0, 0),
        (390.0, 1),
        (-30.0, 11),
        (-1.0, 0),
        (f32::NAN, 0),
    ] {
        assert_eq!(hue_to_palette_index(hue), expected, "hue = {hue}");
    }
}

/// 既定は全 12 色（何も絞っていない状態）。`none()` は 1 色も許可しない。
#[test]
fn color_set_default_allows_every_palette_color() {
    let all = ColorSet::default();
    assert_eq!(all, ColorSet::all(), "既定 = 全色");
    assert_eq!(all.len(), PALETTE_LEN);
    assert_eq!(all.indices(), (0..PALETTE_LEN).collect::<Vec<usize>>());
    assert!(!all.is_empty());
    assert!(ColorSet::none().is_empty());
    assert_eq!(ColorSet::none().len(), 0);
    for index in 0..PALETTE_LEN {
        assert!(all.contains(index), "添字 {index} は許可されている");
    }
}

/// 色相の列は最も近い色へ丸め、重複を畳み、色相順に正規化する。
#[test]
fn color_set_from_hues_normalizes() {
    // 29 は 30 へ、359 は 0 へ丸まる。240 の重複は 1 つに畳まれる。
    let set = ColorSet::from_hues([240.0, 0.0, 150.0, 359.0, 29.0, 240.0]);
    assert_eq!(set.indices(), [0, 1, 5, 8], "0/30/150/240 の 4 色");
    assert_eq!(
        set.hues().collect::<Vec<u32>>(),
        [0, 30, 150, 240],
        "色相順で戻る"
    );
    assert!(set.contains(5));
    assert!(!set.contains(2), "許可していない色は contains が false");
}

/// `set_index` は許可の on/off を切り替え、昇順・重複なしを保つ（パレット外は無視）。
#[test]
fn color_set_set_index_toggles_without_breaking_order() {
    let mut set = ColorSet::none();
    set.set_index(5, true);
    set.set_index(0, true);
    set.set_index(11, true);
    assert_eq!(set.indices(), [0, 5, 11], "昇順に保たれる");
    set.set_index(0, true);
    assert_eq!(set.indices(), [0, 5, 11], "既に許可済みでも重複しない");
    set.set_index(5, false);
    assert_eq!(set.indices(), [0, 11], "off で外れる");
    set.set_index(5, false);
    assert_eq!(set.indices(), [0, 11], "既に不許可でも壊れない");

    let mut all = ColorSet::default();
    all.set_index(PALETTE_LEN, false);
    all.set_index(usize::MAX, true);
    assert_eq!(all, ColorSet::default(), "パレット外の添字は無視する");
}

/// `insert_hue` は色相を最も近い色として加える（R21 の「この色を出す」）。
#[test]
fn color_set_insert_hue_rounds_and_keeps_sorted() {
    let mut set = ColorSet::from_hues([240.0]);
    set.insert_hue(150.0);
    set.insert_hue(29.0); // 30° へ丸まる
    set.insert_hue(240.0); // 既にある → 重複しない
    assert_eq!(set.indices(), [1, 5, 8], "30/150/240 の 3 色");
}

// =====================================================================
// 出現時の色を固定する（R19/R20 の個体）
// =====================================================================

/// `fixed_at` は宣言の sat / lum / glow / sweep を引き継ぎ、色と回転だけ差し替える。
#[test]
fn fixed_at_pins_the_color_and_stops_rotation() {
    let declared = TintStyle {
        mode: TintMode::Cycle,
        rotate: 150.0,
        sat: 35.0,
        lum: 78.0,
        glow: 0.5,
        sweep: Sweep::Steps,
    };
    let fixed = declared.fixed_at(210.0);

    assert_eq!(fixed.mode, TintMode::Fixed(210.0), "指定した色になる");
    assert_eq!(fixed.rotate, 0.0, "回してしまうと選んだ色が変わってしまう");
    assert_eq!(fixed.sat, 35.0, "彩度は宣言を引き継ぐ");
    assert_eq!(fixed.lum, 78.0, "明度は宣言を引き継ぐ");
    assert_eq!(fixed.glow, 0.5, "グローは宣言を引き継ぐ");
    assert_eq!(fixed.sweep, Sweep::Steps, "回転のしかたは宣言を引き継ぐ");
}
