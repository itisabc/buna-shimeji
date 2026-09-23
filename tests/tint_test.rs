//! 色の宣言ファイル（`conf/<set>/tint.xml` / `img/<set>/conf/tint.xml`）のパースと、
//! 色の変換のテスト。
//!
//! 宣言はルート `<TintPalette>` の属性（`Mode` / `Speed` / `Start` / `Sat` / `Lum` /
//! `Glow`）と、子 `<Color>`（`Id` / `Name` / `Hue` / `Sat` / `Lum` / `Glow` / `Allowed`）。
//! 設計: `docs/plans/design-gaming-color.md`（ローカル専用）

use std::path::PathBuf;

use shimeji::config::{parse_actions, parse_tint};
use shimeji::tint::{hsl_to_rgb, Palette, PaletteColor, StartPhase, TintMode, TintStyle};

/// 一時 tint.xml を書く（`attrs` = ルート属性・`body` = `<Color>` 群）。
fn temp_tint(tag: &str, attrs: &str, body: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("shimeji_tint_{}_{tag}.xml", std::process::id()));
    let content = format!("<TintPalette{attrs}>\n{body}</TintPalette>\n");
    std::fs::write(&path, content).expect("一時 tint.xml を書ける");
    path
}

fn tint_of(tag: &str, attrs: &str) -> TintStyle {
    let path = temp_tint(tag, attrs, "");
    parse_tint(&path).style
}

fn palette_of(tag: &str, attrs: &str, body: &str) -> Palette {
    let path = temp_tint(tag, attrs, body);
    parse_tint(&path).palette
}

// =====================================================================
// 宣言のパース
// =====================================================================

/// `Mode` が無ければ色づけなし（パレットだけ置いた set は影響を受けない）。
#[test]
fn absent_mode_means_off() {
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
        " Mode=\"cycle\" Speed=\"200\" Sat=\"35\" Lum=\"78\" Glow=\"0.5\" Start=\"180\"",
    );
    assert_eq!(tint.mode, TintMode::Cycle);
    assert_eq!(tint.rotate, 200.0);
    assert_eq!(tint.sat, 35.0);
    assert_eq!(tint.lum, 78.0);
    assert_eq!(tint.glow, 0.5);
    assert_eq!(tint.start, StartPhase::Fixed(180.0));
}

/// `Mode="cycle"` で速度未指定なら既定 150°/s（`rainbow` は別名）。
#[test]
fn cycle_defaults_to_150_per_second() {
    let tint = tint_of("cycle-default", " Mode=\"cycle\"");
    assert_eq!(tint.mode, TintMode::Cycle);
    assert_eq!(tint.rotate, 150.0);
    assert_eq!(tint.sat, 100.0);
    assert_eq!(tint.lum, 62.0);
    assert_eq!(tint.glow, 1.0);
    assert_eq!(
        tint_of("rainbow", " Mode=\"rainbow\"").mode,
        TintMode::Cycle
    );
}

/// `random` は出現時に色を決めるので回さない。
#[test]
fn random_mode_does_not_rotate() {
    let random = tint_of("random", " Mode=\"random\"");
    assert_eq!(random.mode, TintMode::Random);
    assert_eq!(random.rotate, 0.0);
}

/// `off` / `none` / 未指定は色づけなし（警告なしで受ける）。
#[test]
fn off_and_none_mean_no_tinting() {
    for value in ["off", "none"] {
        let tint = tint_of(value, &format!(" Mode=\"{value}\""));
        assert_eq!(tint.mode, TintMode::Off, "Mode={value}");
        assert_eq!(tint.rotate, 0.0);
    }
}

/// 旧値（`#RRGGBB` / `within` / `steps`）と未知の値は警告 + 無色（起動は止めない）。
#[test]
fn legacy_and_unknown_mode_values_fall_back_to_no_tinting() {
    for value in ["#00ff00", "within", "steps", "glitter"] {
        let tint = tint_of(value, &format!(" Mode=\"{value}\""));
        assert_eq!(tint.mode, TintMode::Off, "Mode={value}");
        assert_eq!(tint.rotate, 0.0);
    }
}

/// 数値のパース失敗・非有限も既定へフォールバック。
#[test]
fn invalid_numbers_fall_back_to_defaults() {
    let tint = tint_of("bad-numbers", " Mode=\"cycle\" Speed=\"fast\" Sat=\"NaN\"");
    assert_eq!(tint.rotate, 150.0, "不正な速度は既定");
    assert_eq!(tint.sat, 100.0, "NaN は既定");
}

/// `Start` は出現時の初期色相。**省略すると出現のたびに抽選**し、書けば 0 以上 360 未満へ wrap する。
#[test]
fn start_is_drawn_when_omitted_and_wraps_when_written() {
    assert_eq!(
        tint_of("start-default", " Mode=\"cycle\"").start,
        StartPhase::Random,
        "省略 = 個体ごとに抽選"
    );
    assert_eq!(
        tint_of("start", " Mode=\"cycle\" Start=\"90\"").start,
        StartPhase::Fixed(90.0)
    );
    assert_eq!(
        tint_of("start-neg", " Mode=\"cycle\" Start=\"-30\"").start,
        StartPhase::Fixed(330.0),
        "負の色相は wrap する"
    );
    assert_eq!(
        tint_of("start-wrap", " Mode=\"cycle\" Start=\"450\"").start,
        StartPhase::Fixed(90.0),
        "360 以上は wrap する"
    );
    assert_eq!(
        tint_of("start-bad", " Mode=\"cycle\" Start=\"soon\"").start,
        StartPhase::Random,
        "不正値は省略と同じ扱い（警告 + 抽選）"
    );
}

/// ファイルが無ければ無色（宣言なし扱い・警告なし）。読めない / 壊れた XML も無色で起動は止めない。
#[test]
fn missing_or_broken_file_means_no_colour() {
    let missing = std::env::temp_dir().join("shimeji_tint_does_not_exist.xml");
    let _ = std::fs::remove_file(&missing);
    let config = parse_tint(&missing);
    assert_eq!(config.style, TintStyle::default(), "無ければ無色");
    assert!(config.palette.is_empty());

    let mut broken = std::env::temp_dir();
    broken.push(format!("shimeji_tint_broken_{}.xml", std::process::id()));
    std::fs::write(&broken, "<TintPalette><Color Id=\"red\">\n").expect("一時ファイルを書ける");
    let config = parse_tint(&broken);
    assert_eq!(config.style, TintStyle::default(), "壊れた XML は無色");
    assert!(config.palette.is_empty());
}

/// 旧形式（`actions.xml` の `<Mascot Tint=...>` と、その子の `<TintPalette>`）は
/// **読めるが無視される**（警告は 1 回。色は `tint.xml` だけが持つ）。
#[test]
fn legacy_actions_declaration_is_ignored() {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "shimeji_tint_legacy_actions_{}.xml",
        std::process::id()
    ));
    std::fs::write(
        &path,
        "<Mascot xmlns=\"http://www.group-finity.com/Mascot\" Tint=\"random\" TintSat=\"100\">\n\
         <TintPalette><Color Id=\"red\" Hue=\"0\"/></TintPalette>\n\
         <ActionList>\n\
         <Action Name=\"Stand\" Type=\"Stay\" BorderType=\"Floor\">\n\
         <Animation><Pose Image=\"/shime1.png\" ImageAnchor=\"64,128\" Velocity=\"0,0\" Duration=\"10\"/></Animation>\n\
         </Action>\n\
         </ActionList>\n\
         </Mascot>\n",
    )
    .expect("一時 actions.xml を書ける");

    let config = parse_actions(&path).expect("旧形式でも actions.xml は読める");
    assert!(
        config.actions.contains_key("Stand"),
        "アクションは従来どおり読める（色の宣言だけを無視する）"
    );
    assert_eq!(
        config.actions.len(),
        1,
        "旧 <TintPalette> はアクションとして読まれない"
    );
}

/// ルート要素が `<TintPalette>` でなければ無色（警告のみ・起動は止めない）。
#[test]
fn unknown_root_tag_means_no_colour() {
    let mut path = std::env::temp_dir();
    path.push(format!("shimeji_tint_root_{}.xml", std::process::id()));
    std::fs::write(
        &path,
        "<Mascot Mode=\"cycle\"><Color Id=\"red\" Hue=\"0\"/></Mascot>\n",
    )
    .expect("一時ファイルを書ける");
    let config = parse_tint(&path);
    assert_eq!(config.style, TintStyle::default());
    assert!(config.palette.is_empty());
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
// 出現時の確定色（R19/R20 の個体）
// =====================================================================

/// `with_color` はパレットの色をそのまま確定色にする（回転は止め、glow も色の値）。
#[test]
fn with_color_pins_the_palette_colour() {
    let declared = TintStyle {
        mode: TintMode::Cycle,
        rotate: 150.0,
        sat: 35.0,
        lum: 78.0,
        glow: 0.5,
        start: StartPhase::Fixed(0.0),
    };
    let colour = PaletteColor {
        id: "white".to_string(),
        name: "白".to_string(),
        hue: 0.0,
        sat: 0.0,
        lum: 100.0,
        glow: 0.0,
        allowed: true,
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
    assert_eq!(fixed.start, StartPhase::Fixed(0.0), "初期位相もその色");
}

/// パレットの色は宣言順に列挙でき、`iter` がメニューの並びの正本になる。
#[test]
fn palette_iterates_in_declaration_order() {
    let palette = palette_of(
        "palette-iter",
        " Mode=\"random\"",
        "<Color Id=\"b\" Hue=\"240\"/>\n<Color Id=\"a\" Hue=\"0\"/>\n",
    );
    let ids: Vec<&str> = palette.iter().map(|color| color.id.as_str()).collect();
    assert_eq!(ids, ["b", "a"], "宣言順（id 順に並べ替えない）");
}

// =====================================================================
// 色の宣言（<Color>）
// =====================================================================

/// 宣言順を保ち、省略した属性はルートの宣言値を継承し、書いた属性だけ上書きする。
#[test]
fn colours_keep_declaration_order_and_inherit_defaults() {
    let palette = palette_of(
        "palette-order",
        " Mode=\"random\" Sat=\"100\" Lum=\"62\" Glow=\"1.0\"",
        "<Color Id=\"strawberry\" Name=\"いちご\" Hue=\"0\"/>\n\
         <Color Id=\"mint\" Name=\"ミント\" Hue=\"150\" Glow=\"1.4\"/>\n\
         <Color Id=\"white\" Name=\"白\" Sat=\"0\" Lum=\"100\" Glow=\"0\"/>\n",
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

/// `Name` 省略は `Id` を表示名にする。`<Color>` が無い / 空なら色なし。
#[test]
fn name_defaults_to_id_and_absent_colours_is_empty() {
    let named = palette_of(
        "palette-name",
        " Mode=\"random\"",
        "<Color Id=\"soda\" Hue=\"180\"/>\n",
    );
    assert_eq!(named.get(0).expect("1 色目").name, "soda");

    let absent = palette_of("palette-absent", " Mode=\"random\"", "");
    assert!(absent.is_empty(), "宣言が無ければ色なし");
    assert_eq!(absent.get(0), None);

    let empty = palette_of("palette-empty", " Mode=\"random\"", "\n");
    assert!(empty.is_empty(), "<Color> が無くても色なし");
}

/// `Id` 欠落・不正な文字・重複はその色だけ捨てる（重複は先勝ち・後続は残る）。
#[test]
fn colours_skip_bad_or_duplicate_ids() {
    let palette = palette_of(
        "palette-id",
        " Mode=\"random\"",
        "<Color Hue=\"60\"/>\n\
         <Color Id=\"ok\" Hue=\"60\"/>\n\
         <Color Id=\"ok\" Hue=\"240\"/>\n\
         <Color Id=\"has space\" Hue=\"120\"/>\n\
         <Color Id=\"sky_blue-2\" Hue=\"210\"/>\n\
         <Color Id=\"after\" Hue=\"300\"/>\n",
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
fn colours_drop_invalid_numbers_and_wrap_hue() {
    let palette = palette_of(
        "palette-bad-numbers",
        " Mode=\"random\" Sat=\"100\"",
        "<Color Id=\"bad-hue\" Hue=\"green\"/>\n\
         <Color Id=\"bad-sat\" Sat=\"NaN\"/>\n\
         <Color Id=\"wrapped\" Hue=\"-30\"/>\n\
         <Color Id=\"later\" Hue=\"330.5\" Glow=\"0.5\"/>\n",
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

// =====================================================================
// 出現を許可する色（Allowed）
// =====================================================================

/// `Allowed` は省略 = 許可、`false` だけが出さない（他の値は警告 + 許可）。
#[test]
fn allowed_defaults_to_true_and_only_false_disables() {
    let palette = palette_of(
        "allowed",
        " Mode=\"random\"",
        "<Color Id=\"strawberry\" Hue=\"0\"/>\n\
         <Color Id=\"white\" Sat=\"0\" Lum=\"100\" Glow=\"0\" Allowed=\"false\"/>\n\
         <Color Id=\"mint\" Hue=\"150\" Allowed=\"true\"/>\n\
         <Color Id=\"odd\" Hue=\"240\" Allowed=\"nope\"/>\n",
    );
    assert!(palette.get(0).expect("1 色目").allowed, "省略 = 許可");
    assert!(!palette.get(1).expect("2 色目").allowed, "false = 出さない");
    assert!(palette.get(2).expect("3 色目").allowed, "true = 許可");
    assert!(
        palette.get(3).expect("4 色目").allowed,
        "不正値は既定（許可）"
    );
}

/// 許可色の一覧は宣言順のまま `Allowed != false` の色だけ（R19 の一覧と R20 の抽選母集団）。
#[test]
fn allowed_colours_filter_in_declaration_order() {
    let palette = palette_of(
        "allowed-filter",
        " Mode=\"random\"",
        "<Color Id=\"b\" Hue=\"240\" Allowed=\"false\"/>\n\
         <Color Id=\"a\" Hue=\"0\"/>\n\
         <Color Id=\"c\" Hue=\"120\"/>\n",
    );
    let ids: Vec<&str> = palette
        .allowed_colors()
        .map(|color| color.id.as_str())
        .collect();
    assert_eq!(ids, ["a", "c"], "宣言順のまま・false だけ落ちる");

    let none = palette_of(
        "allowed-none",
        " Mode=\"random\"",
        "<Color Id=\"a\" Hue=\"0\" Allowed=\"false\"/>\n",
    );
    assert_eq!(none.allowed_colors().count(), 0, "全色 false は 0 色");
}
