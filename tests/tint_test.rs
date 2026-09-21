//! ルート `<Mascot>` の色づけ宣言（`Tint` / `TintSpeed` / `TintSat` / `TintLum` /
//! `TintGlow` / `TintSweep`）のパースと、色の変換のテスト。
//!
//! 設計: `.tmp/design-gaming-color.md`（ローカル専用）

use std::path::PathBuf;

use shimeji::config::parse_actions;
use shimeji::tint::{hex_to_hue, hsl_to_rgb, Sweep, TintMode, TintStyle};

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
