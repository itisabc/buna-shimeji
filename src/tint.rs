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
/// グロー強度に掛ける gain。加算するのは α をぼかした**シルエット**なので、そのまま
/// 加算すると面積ぶん白飛びする（プロトタイプの `gain` と同じ値）。
pub const GLOW_GAIN: f32 = 0.4;

/// 出現を許可する色の単位（コア固定の 12 色・色相 30° 刻み）。
///
/// 添字は色相順で、色相 = 添字 × 30。**表示名は辞書**（`conf/lang/<code>.toml`）が持ち、
/// ここには色相値だけを置く（設計: 「12 色の表示名は辞書に置く」）。
/// 既定の日本語名は いちご / みかん / レモン / メロン / マスカット / ミント / ソーダ /
/// そらいろ / ブルーベリー / ぶどう / カシス / もも、英語名は Strawberry / Tangerine /
/// Lemon / Melon / Muscat / Mint / Soda / Sky blue / Blueberry / Grape / Cassis / Peach。
pub const PALETTE_HUES: [u32; 12] = [0, 30, 60, 90, 120, 150, 180, 210, 240, 270, 300, 330];

/// パレットの色数（[`PALETTE_HUES`] の長さ）。
pub const PALETTE_LEN: usize = PALETTE_HUES.len();

/// パレット添字の色相（度）。範囲外は `None`。
pub fn palette_hue(index: usize) -> Option<u32> {
    PALETTE_HUES.get(index).copied()
}

/// 色相（度）を最も近いパレット色の添字へ丸める。
///
/// 丸めは 30° 刻みで、境界の 15° は上の色（`f32::round` の half away from zero）。
/// 負値と 360 以上は wrap し、NaN は 0 になる。宣言の `#RRGGBB` から得た色相も
/// この関数を通して「どの名前付き色か」を決める（設計: 最も近い名前付き色に丸める）。
pub fn hue_to_palette_index(hue: f32) -> usize {
    // NaN は `as usize` で 0 になる（丸めた値をそのまま使う。位相を進める関数ではない）
    ((hue.rem_euclid(360.0) / 30.0).round() as usize) % PALETTE_LEN
}

/// 出現を許可する色の集合（パレット添字・色相順・重複なし）。
///
/// この集合が「お気に入り」であり、同時にランダム出現（[`TintMode::Random`]）の
/// 抽選母集団、[`Sweep::Within`] / [`Sweep::Steps`] の回転範囲になる（別々に持たない）。
/// 空 = 1 色も許可しない。**オン/オフの切り替え UI は持たない**（トレイ・右クリックは別スライス）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorSet {
    /// 許可するパレット添字（昇順・重複なし）。
    slots: Vec<usize>,
}

impl Default for ColorSet {
    /// 既定は全 12 色（何も絞っていない状態。絞るのは利用者の操作）。
    fn default() -> Self {
        ColorSet {
            slots: (0..PALETTE_LEN).collect(),
        }
    }
}

impl ColorSet {
    /// 全 12 色を許可する集合。
    pub fn all() -> Self {
        Self::default()
    }

    /// 許可色なしの集合（`settings.toml` の `colors = []`）。
    pub fn none() -> Self {
        ColorSet { slots: Vec::new() }
    }

    /// 色相の列から作る（`settings.toml` の `[tint] colors`）。
    ///
    /// 各値は最も近いパレット色へ丸める。30° 刻みから外れた値は `log::warn` で
    /// 知らせる（起動は止めない＝宣言の不正値と同じ方針）。重複は 1 つに畳み、
    /// 並びは色相順に正規化する（抽選と回転が色相順を前提にするため）。
    pub fn from_hues(hues: impl IntoIterator<Item = f32>) -> Self {
        let mut slots: Vec<usize> = Vec::new();
        for hue in hues {
            let index = hue_to_palette_index(hue);
            let snapped = PALETTE_HUES[index] as f32;
            let wrapped = hue.rem_euclid(360.0);
            // 円環距離（359.9 は 0 へ丸まるので、差は 0.1 として見る）
            let delta = (wrapped - snapped)
                .abs()
                .min(360.0 - (wrapped - snapped).abs());
            if hue.is_finite() && delta > 0.5 {
                log::warn!("[tint] colors: {hue} is not a 30-degree palette hue; using {snapped}");
            }
            if !slots.contains(&index) {
                slots.push(index);
            }
        }
        slots.sort_unstable();
        ColorSet { slots }
    }

    /// その色を許可しているか（パレット添字）。
    pub fn contains(&self, index: usize) -> bool {
        self.slots.binary_search(&index).is_ok()
    }

    /// 許可色のパレット添字（色相順）。抽選母集団と回転範囲の正本。
    pub fn indices(&self) -> &[usize] {
        &self.slots
    }

    /// 許可色の色相（度・色相順）。`settings.toml` への書き出しに使う。
    pub fn hues(&self) -> impl Iterator<Item = u32> + '_ {
        self.slots.iter().map(|&index| PALETTE_HUES[index])
    }

    /// 許可色数。
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// 許可色が 1 つも無いか。
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

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

impl TintStyle {
    /// 描画時に加算合成するグローの α 倍率（0..=255）。`0` = グローなし。
    ///
    /// ブラー層は素の α ブラー（最大 255）で保持し、強度はここで掛ける
    /// （設計: 「ブラー層は素の α ブラーで保持し、強度は加算合成時に掛ける」）。
    /// 色づけなし（[`TintMode::Off`]）の個体は色を持たないため光らせない。
    pub fn glow_alpha(&self) -> u8 {
        if self.mode == TintMode::Off {
            return 0;
        }
        // NaN は `as u8` で 0 になる（負値と 255 超もこのクランプで吸収する）。
        (self.glow * GLOW_GAIN * 255.0).round().clamp(0.0, 255.0) as u8
    }

    /// 出現時の色を固定した見せ方（宣言の sat / lum / glow / sweep は引き継ぎ、回転は止める）。
    ///
    /// 手動で選んだ色（R19）とランダム抽選（R20）の個体に使う。回転を残すと選んだ色が
    /// 数秒で変わってしまい「その色の個体を出す」にならないため `rotate = 0` にする。
    /// 宣言で回している set（`Tint="rainbow"`）でも、色を指定して出した個体は固定色になる。
    pub fn fixed_at(self, hue: f32) -> Self {
        TintStyle {
            mode: TintMode::Fixed(hue),
            rotate: 0.0,
            ..self
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
