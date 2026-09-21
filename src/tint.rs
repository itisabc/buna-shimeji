//! 色づけ（tint）の宣言と型。
//!
//! 宣言は per-set conf（`conf/<set>/actions.xml` または `img/<set>/conf/actions.xml`）の
//! ルート `<Mascot>` 属性で与える:
//!
//! ```xml
//! <Mascot Tint="rainbow" TintSpeed="150" TintSat="100" TintLum="62" TintGlow="1.4" TintSweep="within">
//! ```
//!
//! 色は「基準色相 + 回転速度」で表し、ゲーミング / パステル の違いは [`TintStyle`] の
//! 数値の組でしかない（設計: `docs/plans/design-gaming-color.md`）。
//! 個体が持つのは位相だけ（[`crate::mascot::Mascot`] 側）で、速度・彩度・明度・グロー・
//! 回転のしかたは set 単位のこの型が持つ。

/// 回転のしかた。既定は [`Sweep::Within`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sweep {
    /// 全色相をなめらかに回る（許可色の絞り込みはランダムにだけ効く）。
    Full,
    /// 許可された色の範囲だけをなめらかに回る。
    #[default]
    Within,
    /// 許可された色を順に切り替える（補間なし）。
    Steps,
}

/// 色の出し方（`Tint` 属性の値）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TintMode {
    /// 色づけなし（既定）。
    #[default]
    Off,
    /// 色相を回し続ける（`rainbow`）。
    Cycle,
    /// 出現のたびに許可色から抽選する（`random`）。
    Random,
    /// 固定色（`#RRGGBB` の色相・度）。
    Fixed(f32),
}

/// set ごとの見せ方。宣言で決まり、個体では変わらない。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TintStyle {
    /// 色の出し方。
    pub mode: TintMode,
    /// 回転速度（°/秒）。0 なら固定色。
    pub rotate: f32,
    /// 彩度（%）。
    pub sat: f32,
    /// 明度（%）。
    pub lum: f32,
    /// グロー強度（0 で光らない）。
    pub glow: f32,
    /// 回転のしかた。
    pub sweep: Sweep,
}

/// 既定の彩度（%）。ゲーミングの値。
pub const DEFAULT_SAT: f32 = 100.0;
/// 既定の明度（%）。プロトタイプと同じ。
pub const DEFAULT_LUM: f32 = 62.0;
/// 既定の回転速度（°/秒）。
pub const DEFAULT_ROTATE: f32 = 150.0;
/// 既定のグロー強度。
pub const DEFAULT_GLOW: f32 = 1.0;

impl Default for TintStyle {
    fn default() -> Self {
        TintStyle {
            mode: TintMode::Off,
            rotate: 0.0,
            sat: DEFAULT_SAT,
            lum: DEFAULT_LUM,
            glow: DEFAULT_GLOW,
            sweep: Sweep::Within,
        }
    }
}

/// `#RRGGBB` / `RRGGBB` を HSL の色相（度・0 以上 360 未満）へ変換する。
/// 桁数違い・非 16 進は `None`。彩度 0 相当（R=G=B）は 0 を返す。
pub fn hex_to_hue(text: &str) -> Option<f32> {
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()? as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    if d == 0.0 {
        return Some(0.0);
    }
    let h = if max == r {
        60.0 * (((g - b) / d) % 6.0)
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    Some(if h < 0.0 { h + 360.0 } else { h })
}

/// `hsl(hue, sat%, lum%)` を sRGB の RGB へ変換する。
///
/// ここで返る RGB は「元の画素に掛ける乗算係数」であり、そのまま
/// [`crate::win::window::blit_argb_at`] の `tint` に渡せる。
/// プロトタイプ（`.tmp/demo/gaming-shimeji.html`）と同じ式で、
/// `hsl(0, 100%, 62%)` は `#ff3d3d` になる。
pub fn hsl_to_rgb(hue: f32, sat: f32, lum: f32) -> [u8; 3] {
    let s = (sat / 100.0).clamp(0.0, 1.0);
    let l = (lum / 100.0).clamp(0.0, 1.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h = hue.rem_euclid(360.0) / 60.0;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [to(r), to(g), to(b)]
}
