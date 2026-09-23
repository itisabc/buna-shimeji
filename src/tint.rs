//! 色づけ（tint）の宣言と型。
//!
//! 宣言は per-set conf（`conf/<set>/tint.xml` または `img/<set>/conf/tint.xml`）の
//! ルート `<TintPalette>` 属性で与える:
//!
//! ```xml
//! <TintPalette Mode="cycle" Speed="150" Start="0" Sat="100" Lum="62" Glow="1.4">
//! ```
//!
//! `Start` は出現時の位相（色相）で、**書かなければ出現のたびに抽選**する（個体ごとに違う色から
//! 始まる）。`Start="0"` のように書けば全個体が同じ色から始まる。
//!
//! 存在する色は同じファイルの `<Color>` が持つ（本体は色を知らない）:
//!
//! ```xml
//! <TintPalette>
//!   <Color Id="strawberry" Name="いちご" Hue="0"/>
//!   <Color Id="white" Sat="0" Lum="100" Glow="0" Allowed="false"/>
//! </TintPalette>
//! ```
//!
//! **出現を許可する色は [`PaletteColor::allowed`]**（省略 = 許可）が持つ。アプリは
//! 読むだけで書き戻さない（利用者がファイルに手書きする）。
//!
//! 色は「基準色相 + 回転速度」で表し、ゲーミング / パステル の違いは [`TintStyle`] の
//! 数値の組でしかない（設計: `docs/plans/design-gaming-color.md`）。
//! 個体が持つのは確定色（色相・彩度・明度・グロー）と位相だけで（[`crate::mascot::Mascot`] 側）、
//! 宣言の規則（選び方・速度・初期位相）と省略値は set 単位のこの型が持つ。

/// 色の出し方（`Mode` 属性の値）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum TintMode {
    /// 色づけなし（既定・`Mode` を書かない）。
    #[default]
    Off,
    /// 色相を回し続ける（`cycle` / `rainbow`）。パレットを参照しない。
    Cycle,
    /// 出現のたびに許可色から抽選する（`random`）。
    Random,
    /// 確定色（出現時に決まった色相・度）。宣言では使わず、解決済みの個体だけが持つ。
    Fixed(f32),
}

/// 出現時の位相の初期値（`Start` 属性）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum StartPhase {
    /// 宣言の色相（度・0 以上 360 未満）から始める（`Start="90"` など）。
    Fixed(f32),
    /// 出現のたびに抽選する（**`Start` を書かない既定**。個体ごとに違う色から始まる）。
    #[default]
    Random,
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
    /// 出現時の位相（色相・度）の決め方。`Speed="0"` と `Fixed` を組めば固定色。
    pub start: StartPhase,
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

/// パレットの 1 色。色（hue / sat / lum）と、その色でいるときのグロー（glow）、
/// 出現を許可するか（allowed）を持つ。
///
/// 省略した属性はルート `<TintPalette>` の宣言値（`Sat` / `Lum` / `Glow`。`Hue` は 0）を
/// 既定として**パース時に埋める**ので、利用側は解決済みの値だけを見る。
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteColor {
    /// 設定へ保存する安定キー（`[A-Za-z0-9_-]`・set 内で一意）。表示名ではない。
    pub id: String,
    /// 表示名（`Name` 省略時は `id`）。**辞書を通さない**（作者データ。AGENTS.md §6 の適用外）。
    pub name: String,
    /// 色相（度・0 以上 360 未満）。
    pub hue: f32,
    /// 彩度（%）。`0` = 無彩色。
    pub sat: f32,
    /// 明度（%）。
    pub lum: f32,
    /// この色でいるときのグロー強度（0 で光らない）。
    pub glow: f32,
    /// 出現を許可するか（`Allowed="false"` で false。省略 = true）。
    /// 「許可色の一覧（R19）」と「ランダム出現の抽選母集団（R20）」の両方を決める。
    pub allowed: bool,
}

/// set ごとのパレット（`tint.xml` の `<Color>` の宣言順）。空 = 色なし。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Palette {
    /// 宣言順の色（メニューの並びが作者の意図になるので、色相順へ並べ替えない）。
    colors: Vec<PaletteColor>,
}

impl Palette {
    /// 宣言順の色から作る（[`crate::config::parse_tint`] のパース結果）。
    pub fn from_colors(colors: Vec<PaletteColor>) -> Self {
        Palette { colors }
    }

    /// 色が 1 つも無いか（`tint.xml` が無い set も空）。
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

    /// `Id` から位置を引く。
    pub fn index_of_id(&self, id: &str) -> Option<usize> {
        self.colors.iter().position(|color| color.id == id)
    }

    /// **出現を許可されている色**を宣言順に返す（R19 の一覧と R20 の抽選母集団）。
    pub fn allowed_colors(&self) -> impl Iterator<Item = &PaletteColor> {
        self.colors.iter().filter(|color| color.allowed)
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
            start: StartPhase::Random,
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
            start: StartPhase::Fixed(color.hue),
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
