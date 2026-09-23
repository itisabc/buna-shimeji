//! 色づけ（tint）の宣言と型。
//!
//! 宣言は per-set conf（`conf/<set>/actions.xml` または `img/<set>/conf/actions.xml`）の
//! ルート `<Mascot>` 属性で与える:
//!
//! ```xml
//! <Mascot Tint="cycle" TintSpeed="150" TintStart="0" TintSat="100" TintLum="62" TintGlow="1.4">
//! ```
//!
//! 存在する色は同じ set の `<TintPalette>` が持つ（本体は色を知らない）:
//!
//! ```xml
//! <TintPalette>
//!   <Color Id="strawberry" Name="いちご" Hue="0"/>
//!   <Color Id="white" Sat="0" Lum="100" Glow="0"/>
//! </TintPalette>
//! ```
//!
//! 色は「基準色相 + 回転速度」で表し、ゲーミング / パステル の違いは [`TintStyle`] の
//! 数値の組でしかない（設計: `docs/plans/design-gaming-color.md`）。
//! 個体が持つのは確定色（色相・彩度・明度・グロー）と位相だけで（[`crate::mascot::Mascot`] 側）、
//! 宣言の規則（選び方・速度・初期位相）と省略値は set 単位のこの型が持つ。

use std::collections::BTreeSet;

/// 色の出し方（`Tint` 属性の値）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TintMode {
    /// 色づけなし（既定・`Tint` を書かない）。
    #[default]
    Off,
    /// 色相を回し続ける（`cycle` / `rainbow`）。パレットを参照しない。
    Cycle,
    /// 出現のたびに許可色から抽選する（`random`）。
    Random,
    /// 確定色（出現時に決まった色相・度）。宣言では使わず、解決済みの個体だけが持つ。
    Fixed(f32),
}

/// set ごとの見せ方。宣言で決まり、個体では変わらない。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TintStyle {
    /// 色の出し方。
    pub mode: TintMode,
    /// 回転速度（°/秒）。0 なら位相を進めない（`start` の色相で固定）。
    pub rotate: f32,
    /// 彩度（%）。
    pub sat: f32,
    /// 明度（%）。
    pub lum: f32,
    /// グロー強度（0 で光らない）。
    pub glow: f32,
    /// 出現時の位相の初期値（色相・度・0 以上 360 未満）。`TintSpeed="0"` と組めば固定色。
    pub start: f32,
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

/// パレットの 1 色。色（hue / sat / lum）と、その色でいるときのグロー（glow）を持つ。
///
/// 省略した属性は `<Mascot>` の宣言値（`TintSat` / `TintLum` / `TintGlow`。`Hue` は 0）を
/// 既定として**パース時に埋める**ので、利用側は解決済みの値だけを見る。
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteColor {
    /// 設定へ保存する安定キー（`[A-Za-z0-9_-]`・set 内で一意）。表示名ではない。
    pub id: String,
    /// 表示名（`Name` 省略時は `id`）。**辞書を通さない**（作者データ。AGENTS.md §6 の適用外）。
    pub name: String,
    /// 色相（度・0 以上 360 未満）。
    pub hue: f32,
    /// 彩度（%）。`0` = 無彩色で、R21 の写像先の候補から外れる。
    pub sat: f32,
    /// 明度（%）。
    pub lum: f32,
    /// この色でいるときのグロー強度（0 で光らない）。
    pub glow: f32,
}

/// set ごとのパレット（`<TintPalette>` の宣言順）。空 = 色なし。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Palette {
    /// 宣言順の色（メニューの並びが作者の意図になるので、色相順へ並べ替えない）。
    colors: Vec<PaletteColor>,
}

impl Palette {
    /// 宣言順の色から作る（[`crate::config::parse_actions`] のパース結果）。
    pub fn from_colors(colors: Vec<PaletteColor>) -> Self {
        Palette { colors }
    }

    /// 色が 1 つも無いか（`<TintPalette>` 未宣言も空）。
    pub fn is_empty(&self) -> bool {
        self.colors.is_empty()
    }

    /// 宣言順の位置で 1 色を取り出す。
    pub fn get(&self, index: usize) -> Option<&PaletteColor> {
        self.colors.get(index)
    }

    /// 宣言順の色を順に返す（トレイの一覧とセット別サブメニューの並びの正本）。
    pub fn iter(&self) -> impl Iterator<Item = &PaletteColor> {
        self.colors.iter()
    }

    /// `Id` から位置を引く（許可集合は id で保存される）。
    pub fn index_of_id(&self, id: &str) -> Option<usize> {
        self.colors.iter().position(|color| color.id == id)
    }

    /// その色（hue / sat / lum）と完全一致する位置を引く（R21 の写像の第 1 段）。
    ///
    /// R19（一覧から選ぶ）/ R20（抽選）の個体はパレットの値をそのまま写しているので一致する。
    /// `glow` は見ない（`cycle` の個体は宣言値を持ち、パレットのグローと一致しないため）。
    pub fn index_of_values(&self, hue: f32, sat: f32, lum: f32) -> Option<usize> {
        self.colors
            .iter()
            .position(|color| color.hue == hue && color.sat == sat && color.lum == lum)
    }

    /// 色相が最も近い色の位置（R21 の「この色を出す」の写像先）。
    ///
    /// 候補は**有彩色（`sat > 0`）だけ**にする（無彩色は色相を持たず、hue 0 の白と
    /// hue 0 の赤を区別できない）。距離は色相環の最短弧で見る。候補が無ければ `None`。
    pub fn nearest_index(&self, hue: f32) -> Option<usize> {
        let mut nearest: Option<(usize, f32)> = None;
        for (index, color) in self.colors.iter().enumerate() {
            if color.sat <= 0.0 {
                continue;
            }
            let distance = hue_distance(color.hue, hue);
            if nearest.is_none_or(|(_, best)| distance < best) {
                nearest = Some((index, distance));
            }
        }
        nearest.map(|(index, _)| index)
    }
}

/// 色相環の最短弧（度）。
fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// その色 id が許可されているか（`settings.toml` の set 別 `colors` の判定）。
///
/// `None`（キー欠落）= その set の全色を許可、`Some(空)` = 1 色も許可しない。
pub fn id_allowed(allowed: Option<&BTreeSet<String>>, id: &str) -> bool {
    allowed.is_none_or(|colors| colors.contains(id))
}

impl Default for TintStyle {
    fn default() -> Self {
        TintStyle {
            mode: TintMode::Off,
            rotate: 0.0,
            sat: DEFAULT_SAT,
            lum: DEFAULT_LUM,
            glow: DEFAULT_GLOW,
            start: 0.0,
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

    /// 出現時に決まった色を与えた見せ方（R19 の手動指定 / R20 の抽選 / パレットの色）。
    ///
    /// 個体の色は**確定色**（hue / sat / lum / glow をパレットの色からそのまま写す）になり、
    /// 回転は止める（[`TintStyle::rotate`] = 0）。回転を残すと選んだ色が数秒で変わってしまい
    /// 「その色の個体を出す」にならないため。`start` もその色に合わせる。
    pub fn with_color(self, color: &PaletteColor) -> Self {
        TintStyle {
            mode: TintMode::Fixed(color.hue),
            rotate: 0.0,
            sat: color.sat,
            lum: color.lum,
            glow: color.glow,
            start: color.hue,
        }
    }
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
