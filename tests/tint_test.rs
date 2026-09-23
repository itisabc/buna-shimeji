//! ルート `<Mascot>` の色づけ宣言（`Tint` / `TintSpeed` / `TintStart` / `TintSat` /
//! `TintLum` / `TintGlow` と `<TintPalette>`）のパースと、色の変換のテスト。
//!
//! 設計: `docs/plans/design-gaming-color.md`（ローカル専用）

use std::collections::BTreeSet;
use std::path::PathBuf;

use shimeji::config::parse_actions;
use shimeji::tint::{hsl_to_rgb, id_allowed, Palette, PaletteColor, TintMode, TintStyle};

/// `<Mascot>` 直下に `body` を挟んだ一時 actions.xml を書く（`<TintPalette>` 用）。
fn temp_actions_with(tag: &str, root_attrs: &str, body: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("shimeji_tint_{}_{tag}.xml", std::process::id()));
    let content = format!(
        "<Mascot xmlns=\"http://www.group-finity.com/Mascot\"{root_attrs}>\n{body}\
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

fn temp_actions(tag: &str, root_attrs: &str) -> PathBuf {
    temp_actions_with(tag, root_attrs, "")
}

fn tint_of(tag: &str, root_attrs: &str) -> TintStyle {
    let path = temp_actions(tag, root_attrs);
    parse_actions(&path)
        .unwrap_or_else(|e| panic!("parse failed: {e}"))
        .tint
}

fn palette_of(tag: &str, root_attrs: &str, body: &str) -> Palette {
    let path = temp_actions_with(tag, root_attrs, body);
    parse_actions(&path)
        .unwrap_or_else(|e| panic!("parse failed: {e}"))
        .palette
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
        " Tint=\"cycle\" TintSpeed=\"200\" TintSat=\"35\" TintLum=\"78\" TintGlow=\"0.5\" TintStart=\"180\"",
    );
    assert_eq!(tint.mode, TintMode::Cycle);
    assert_eq!(tint.rotate, 200.0);
    assert_eq!(tint.sat, 35.0);
    assert_eq!(tint.lum, 78.0);
    assert_eq!(tint.glow, 0.5);
    assert_eq!(tint.start, 180.0);
}

/// `Tint="cycle"` で速度未指定なら既定 150°/s（`rainbow` は別名）。
#[test]
fn cycle_defaults_to_150_per_second() {
    let tint = tint_of("cycle-default", " Tint=\"cycle\"");
    assert_eq!(tint.mode, TintMode::Cycle);
    assert_eq!(tint.rotate, 150.0);
    assert_eq!(tint.sat, 100.0);
    assert_eq!(tint.lum, 62.0);
    assert_eq!(tint.glow, 1.0);
    assert_eq!(
        tint_of("rainbow", " Tint=\"rainbow\"").mode,
        TintMode::Cycle
    );
}

/// `random` は出現時に色を決めるので回さない。
#[test]
fn random_mode_does_not_rotate() {
    let random = tint_of("random", " Tint=\"random\"");
    assert_eq!(random.mode, TintMode::Random);
    assert_eq!(random.rotate, 0.0);
}

/// `off` / `none` / 未指定は色づけなし（警告なしで受ける）。
#[test]
fn off_and_none_mean_no_tinting() {
    for value in ["off", "none"] {
        let tint = tint_of(value, &format!(" Tint=\"{value}\""));
        assert_eq!(tint.mode, TintMode::Off, "Tint={value}");
        assert_eq!(tint.rotate, 0.0);
    }
}

/// 旧値（`#RRGGBB` / `within` / `steps`）と未知の値は警告 + 無色（起動は止めない）。
#[test]
fn legacy_and_unknown_tint_values_fall_back_to_no_tinting() {
    for value in ["#00ff00", "within", "steps", "glitter"] {
        let tint = tint_of(value, &format!(" Tint=\"{value}\""));
        assert_eq!(tint.mode, TintMode::Off, "Tint={value}");
        assert_eq!(tint.rotate, 0.0);
    }
}

/// 数値のパース失敗・非有限も既定へフォールバック。
#[test]
fn invalid_numbers_fall_back_to_defaults() {
    let tint = tint_of(
        "bad-numbers",
        " Tint=\"cycle\" TintSpeed=\"fast\" TintSat=\"NaN\"",
    );
    assert_eq!(tint.rotate, 150.0, "不正な速度は既定");
    assert_eq!(tint.sat, 100.0, "NaN は既定");
}

/// `TintStart` は初期色相（既定 0）。負値と 360 以上は wrap する。
#[test]
fn tint_start_defaults_to_zero_and_wraps() {
    assert_eq!(tint_of("start-default", " Tint=\"cycle\"").start, 0.0);
    assert_eq!(
        tint_of("start", " Tint=\"cycle\" TintStart=\"90\"").start,
        90.0
    );
    assert_eq!(
        tint_of("start-neg", " Tint=\"cycle\" TintStart=\"-30\"").start,
        330.0,
        "負の色相は wrap する"
    );
    assert_eq!(
        tint_of("start-wrap", " Tint=\"cycle\" TintStart=\"450\"").start,
        90.0,
        "360 以上は wrap する"
    );
    assert_eq!(
        tint_of("start-bad", " Tint=\"cycle\" TintStart=\"soon\"").start,
        0.0,
        "不正値は既定"
    );
}

// =====================================================================
// 色の変換
// =====================================================================

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
// 許可集合の判定と、出現時の確定色（R19/R20 の個体）
// =====================================================================

/// `id_allowed` は「キー欠落 = 全色 / `Some(空)` = 1 色も許可しない」を表す。
#[test]
fn id_allowed_distinguishes_missing_key_from_empty_list() {
    let allowed = BTreeSet::from(["mint".to_string(), "white".to_string()]);
    assert!(id_allowed(Some(&allowed), "mint"));
    assert!(!id_allowed(Some(&allowed), "strawberry"));
    assert!(id_allowed(None, "mint"), "キー欠落（None）は全色を許可");
    let empty = BTreeSet::new();
    assert!(
        !id_allowed(Some(&empty), "mint"),
        "colors = [] は 1 色も許可しない"
    );
}

/// `with_color` はパレットの色をそのまま確定色にする（回転は止め、glow も色の値）。
#[test]
fn with_color_pins_the_palette_colour() {
    let declared = TintStyle {
        mode: TintMode::Cycle,
        rotate: 150.0,
        sat: 35.0,
        lum: 78.0,
        glow: 0.5,
        start: 0.0,
    };
    let colour = PaletteColor {
        id: "white".to_string(),
        name: "白".to_string(),
        hue: 0.0,
        sat: 0.0,
        lum: 100.0,
        glow: 0.0,
    };
    let fixed = declared.with_color(&colour);

    assert_eq!(fixed.mode, TintMode::Fixed(0.0), "その色になる");
    assert_eq!(fixed.rotate, 0.0, "回してしまうと選んだ色が変わってしまう");
    assert_eq!(fixed.sat, 0.0, "彩度は色の値（宣言ではない）");
    assert_eq!(fixed.lum, 100.0, "明度は色の値");
    assert_eq!(
        fixed.glow, 0.0,
        "グローも色の値（白・黒は宣言値で光らない）"
    );
    assert_eq!(fixed.start, 0.0, "初期位相もその色");
}

/// `index_of_values` は hue / sat / lum の完全一致だけを返す（glow は見ない）。
#[test]
fn index_of_values_matches_the_exact_colour() {
    let palette = palette_of(
        "index-of-values",
        " Tint=\"random\"",
        "<TintPalette>\n\
         <Color Id=\"red\" Hue=\"0\"/>\n\
         <Color Id=\"white\" Sat=\"0\" Lum=\"100\" Glow=\"0\"/>\n\
         </TintPalette>\n",
    );
    assert_eq!(palette.index_of_values(0.0, 100.0, 62.0), Some(0));
    assert_eq!(palette.index_of_values(0.0, 0.0, 100.0), Some(1));
    assert_eq!(
        palette.index_of_values(0.0, 100.0, 50.0),
        None,
        "明度が違えば一致しない"
    );
    assert_eq!(palette.index_of_values(150.0, 100.0, 62.0), None);
}

/// パレットの色は宣言順に列挙でき、`iter` がメニューの並びの正本になる。
#[test]
fn palette_iterates_in_declaration_order() {
    let palette = palette_of(
        "palette-iter",
        " Tint=\"random\"",
        "<TintPalette>\n\
         <Color Id=\"b\" Hue=\"240\"/>\n\
         <Color Id=\"a\" Hue=\"0\"/>\n\
         </TintPalette>\n",
    );
    let ids: Vec<&str> = palette.iter().map(|color| color.id.as_str()).collect();
    assert_eq!(ids, ["b", "a"], "宣言順（id 順に並べ替えない）");
}

// =====================================================================
// パレットの宣言（<TintPalette>）
// =====================================================================

/// 宣言順を保ち、省略した属性は `<Mascot>` の宣言値を継承し、書いた属性だけ上書きする。
#[test]
fn palette_keeps_declaration_order_and_inherits_defaults() {
    let palette = palette_of(
        "palette-order",
        " Tint=\"random\" TintSat=\"100\" TintLum=\"62\" TintGlow=\"1.0\"",
        "<TintPalette>\n\
         <Color Id=\"strawberry\" Name=\"いちご\" Hue=\"0\"/>\n\
         <Color Id=\"mint\" Name=\"ミント\" Hue=\"150\" Glow=\"1.4\"/>\n\
         <Color Id=\"white\" Name=\"白\" Sat=\"0\" Lum=\"100\" Glow=\"0\"/>\n\
         </TintPalette>\n",
    );
    assert!(!palette.is_empty());

    let first = palette.get(0).expect("1 色目");
    assert_eq!(first.id, "strawberry");
    assert_eq!(first.name, "いちご");
    assert_eq!(first.hue, 0.0);
    assert_eq!(first.sat, 100.0, "Sat 省略は宣言値");
    assert_eq!(first.lum, 62.0, "Lum 省略は宣言値");
    assert_eq!(first.glow, 1.0, "Glow 省略は宣言値");

    let second = palette.get(1).expect("2 色目");
    assert_eq!(second.id, "mint");
    assert_eq!(second.hue, 150.0, "宣言順（id 順や色相順に並べ替えない）");
    assert_eq!(second.glow, 1.4, "その色だけ上書きできる");

    let third = palette.get(2).expect("3 色目");
    assert_eq!(third.sat, 0.0);
    assert_eq!(third.lum, 100.0);
    assert_eq!(third.glow, 0.0);

    assert_eq!(palette.get(3), None);
    assert_eq!(palette.index_of_id("mint"), Some(1));
    assert_eq!(palette.index_of_id("nope"), None);
}

/// `Name` 省略は `Id` を表示名にする。`<TintPalette>` が無い / 空なら色なし。
#[test]
fn palette_name_defaults_to_id_and_absent_palette_is_empty() {
    let named = palette_of(
        "palette-name",
        " Tint=\"random\"",
        "<TintPalette><Color Id=\"soda\" Hue=\"180\"/></TintPalette>\n",
    );
    assert_eq!(named.get(0).expect("1 色目").name, "soda");

    let absent = palette_of("palette-absent", " Tint=\"random\"", "");
    assert!(absent.is_empty(), "宣言が無ければ色なし");
    assert_eq!(absent.get(0), None);

    let empty = palette_of("palette-empty", " Tint=\"random\"", "<TintPalette/>\n");
    assert!(empty.is_empty(), "<TintPalette> が空でも色なし");
}

/// `Id` 欠落・不正な文字・重複はその色だけ捨てる（重複は先勝ち・後続は残る）。
#[test]
fn palette_skips_colors_with_bad_or_duplicate_ids() {
    let palette = palette_of(
        "palette-id",
        " Tint=\"random\"",
        "<TintPalette>\n\
         <Color Hue=\"60\"/>\n\
         <Color Id=\"ok\" Hue=\"60\"/>\n\
         <Color Id=\"ok\" Hue=\"240\"/>\n\
         <Color Id=\"has space\" Hue=\"120\"/>\n\
         <Color Id=\"sky_blue-2\" Hue=\"210\"/>\n\
         <Color Id=\"after\" Hue=\"300\"/>\n\
         </TintPalette>\n",
    );
    assert_eq!(palette.get(0).map(|c| c.id.as_str()), Some("ok"));
    assert_eq!(palette.get(0).map(|c| c.hue), Some(60.0), "重複は先勝ち");
    assert_eq!(
        palette.get(1).map(|c| c.id.as_str()),
        Some("sky_blue-2"),
        "[A-Za-z0-9_-] は使える"
    );
    assert_eq!(
        palette.get(2).map(|c| c.id.as_str()),
        Some("after"),
        "捨てた色の後ろは残る"
    );
    assert_eq!(palette.get(3), None);
}

/// 数値が不正な色は捨てる（他の色は残る）。`Hue` は 0..360 へ wrap する。
#[test]
fn palette_drops_colors_with_invalid_numbers_and_wraps_hue() {
    let palette = palette_of(
        "palette-bad-numbers",
        " Tint=\"random\" TintSat=\"100\"",
        "<TintPalette>\n\
         <Color Id=\"bad-hue\" Hue=\"green\"/>\n\
         <Color Id=\"bad-sat\" Sat=\"NaN\"/>\n\
         <Color Id=\"wrapped\" Hue=\"-30\"/>\n\
         <Color Id=\"later\" Hue=\"330.5\" Glow=\"0.5\"/>\n\
         </TintPalette>\n",
    );
    assert_eq!(
        palette.get(0).map(|c| c.id.as_str()),
        Some("wrapped"),
        "不正値の色は捨てる"
    );
    assert_eq!(
        palette.get(0).map(|c| c.hue),
        Some(330.0),
        "負の色相は wrap"
    );
    assert_eq!(palette.get(1).map(|c| c.id.as_str()), Some("later"));
    assert_eq!(palette.get(1).map(|c| c.glow), Some(0.5));
    assert_eq!(palette.get(2), None);
}

/// `nearest_index` の候補は `sat > 0` の色だけ（無彩色へは丸まらない）。
#[test]
fn nearest_index_skips_achromatic_colors() {
    let palette = palette_of(
        "nearest",
        " Tint=\"random\"",
        "<TintPalette>\n\
         <Color Id=\"red\" Hue=\"0\"/>\n\
         <Color Id=\"white\" Sat=\"0\" Lum=\"100\" Glow=\"0\"/>\n\
         <Color Id=\"blue\" Hue=\"240\"/>\n\
         </TintPalette>\n",
    );
    assert_eq!(
        palette.nearest_index(10.0),
        Some(0),
        "白（sat 0）は候補にならない"
    );
    assert_eq!(palette.nearest_index(250.0), Some(2));
    assert_eq!(
        palette.nearest_index(350.0),
        Some(0),
        "色相環の距離で近い方へ丸める"
    );
}

/// 有彩色が 1 つも無ければ `nearest_index` は `None`（空も含む）。
#[test]
fn nearest_index_is_none_without_chromatic_candidates() {
    let gray = palette_of(
        "nearest-gray",
        " Tint=\"random\"",
        "<TintPalette><Color Id=\"gray\" Sat=\"0\" Lum=\"50\"/></TintPalette>\n",
    );
    assert_eq!(gray.nearest_index(0.0), None);

    let empty = palette_of("nearest-empty", " Tint=\"random\"", "");
    assert_eq!(empty.nearest_index(0.0), None);
}
