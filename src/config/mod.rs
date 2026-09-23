//! conf XML（actions.xml / behaviors.xml / tint.xml）の強型パース — Java `config` パッケージ相当。
//!
//! Java の Builder（ActionBuilder / AnimationBuilder / BehaviorBuilder）を介さず、
//! XML を強型データへ直接変換する（design.md §1.6: 構造は設計の最適形、
//! ロジックは Java を仕様として移植）。エラーはファイル名・行番号付きの
//! [`ConfigError`] で伝播する（AGENTS.md §5.5）。
//!
//! 資産 XML は UTF-8 BOM あり + CRLF。roxmltree が BOM を自動処理し、
//! 属性値内の改行は XML 仕様に従って空白へ正規化されるため、複数行の式も
//! 空白を許容する式評価器（script.rs）でそのまま評価できる。

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

use roxmltree::{Document, Node};
use thiserror::Error;

pub mod script;

use crate::tint::{Palette, PaletteColor, StartPhase, TintMode, TintStyle};

use script::Variable;

/// XML 属性値のマップ（属性名 → [`Variable`]。Java の params 相当）。
pub type VarMap = BTreeMap<String, Variable>;

/// XML 要素のネスト深さ上限（#21）。再帰パース関数（inline Action / 入れ子 Condition）の
/// スタックオーバーフローを防ぐ。超過時は ConfigError を返す（XML 破損 = Result 終了の
/// 既存契約）。正常な conf のネストは十分浅い。
const MAX_XML_NESTING: usize = 128;

/// 設定読み込みエラー（Java `ConfigurationException` 相当。ファイル・行番号・理由を保持）。
#[derive(Debug, Error)]
#[error("{file}:{line}: {reason}")]
pub struct ConfigError {
    pub file: String,
    /// 行番号（1 始まり）。特定できない場合は 0。
    pub line: u32,
    pub reason: String,
}

/// actions.xml のパース結果。
#[derive(Debug, Clone, Default)]
pub struct ActionsConfig {
    pub actions: BTreeMap<String, ActionDef>,
}

/// tint.xml のパース結果（set の色の宣言）。
///
/// ファイルが無い / 読めない / 壊れている / ルート要素が違う場合は**既定（無色）**を返す
/// （起動は止めない。設計: `docs/plans/design-gaming-color.md` §色の宣言と許可色）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TintConfig {
    /// モードと既定値（`Mode` / `Speed` / `Start` / `Sat` / `Lum` / `Glow`）。
    pub style: TintStyle,
    /// 宣言順の色（`<Color>`）。空 = 色なし。
    pub palette: Palette,
}

impl ActionsConfig {
    /// `(action 名, animation index)` の組に該当するアニメを定義から除去する
    /// （check_references が検出した欠落参照アニメの実無効化・#10b-1）。
    /// index は除去前の animations 内位置を指すため、組ごとに重複除去のうえ
    /// 降順で除去し、先に除去した index が後続 index をずらさないようにする。
    pub fn strip_animations(&mut self, disabled: &[(String, usize)]) {
        for (name, def) in self.actions.iter_mut() {
            let animations = match def {
                ActionDef::Embedded { animations, .. }
                | ActionDef::Stay { animations, .. }
                | ActionDef::Move { animations, .. }
                | ActionDef::Animate { animations, .. }
                | ActionDef::Sequence { animations, .. }
                | ActionDef::Select { animations, .. } => animations,
            };
            let mut indices: Vec<usize> = disabled
                .iter()
                .filter(|(action, _)| action == name)
                .map(|(_, index)| *index)
                .collect();
            if indices.is_empty() {
                continue;
            }
            indices.sort_unstable();
            indices.dedup();
            for index in indices.into_iter().rev() {
                if index < animations.len() {
                    animations.remove(index);
                }
            }
        }
    }
}

/// 境界種別（Action の BorderType 属性）。
/// Java `BorderedAction.java` L24/L40-48 逐語: 属性省略時は `None`（border 無効）。
/// 既知値（Floor / Wall / Ceiling）のみ `Some`。未知値はパースエラー（設計上の
/// fail-fast 強化・Java は未知値を黙って無視するが #3 で据え置き決定済み）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderType {
    Floor,
    Wall,
    Ceiling,
}

/// アクション定義（Java `ActionBuilder` 相当）。
/// 全バリアントが animations を持つ（Java buildAction が全型で animations を受け取るため）。
/// `border` は BorderType 属性の有無を保持する（省略 = None = Java では border 取得なし・
/// design §1.8(g)。Java は明示時に境界解決の結果（NotOnBorder 含む）をトークン保持する）。
#[derive(Debug, Clone)]
pub enum ActionDef {
    Embedded {
        class: String,
        border: Option<BorderType>,
        attrs: VarMap,
        animations: Vec<Animation>,
    },
    Stay {
        border: Option<BorderType>,
        attrs: VarMap,
        animations: Vec<Animation>,
    },
    Move {
        border: Option<BorderType>,
        attrs: VarMap,
        animations: Vec<Animation>,
    },
    Animate {
        border: Option<BorderType>,
        attrs: VarMap,
        animations: Vec<Animation>,
    },
    /// `is_loop` は Loop 属性（既定 false・Java Sequence.java L19-20。Rust 予約語
    /// `loop` のため `is_loop`）。資産 55 箇所使用・Dragged は Loop="true"。
    Sequence {
        border: Option<BorderType>,
        attrs: VarMap,
        is_loop: bool,
        animations: Vec<Animation>,
        children: Vec<SequenceChild>,
    },
    Select {
        border: Option<BorderType>,
        attrs: VarMap,
        is_loop: bool,
        animations: Vec<Animation>,
        children: Vec<SequenceChild>,
    },
}

/// Sequence / Select の子。ActionReference 参照 or 匿名 Action のインライン定義。
#[derive(Debug, Clone)]
pub enum SequenceChild {
    Ref { name: String, attrs: VarMap },
    Inline(Box<ActionDef>),
}

/// アニメーション（Java `AnimationBuilder` 相当）。
/// `is_turn` は IsTurn 属性（既定 false・Java AnimationBuilder L89-90 契約・
/// 方向転換アニメ。資産使用 0 件）。
#[derive(Debug, Clone)]
pub struct Animation {
    pub condition: Option<Variable>,
    pub poses: Vec<Pose>,
    pub is_turn: bool,
}

/// ポーズ（画像パス・アンカー・速度・フレーム数・効果音。Java `Pose` 相当）。
/// 画像側のプリスケールは画像セット側（タスク #4）で行うため、本構造体は生値を保持する。
/// アンカー / velocity の scale 変換はアクション構築時（タスク #7）に
/// `imageset::scale_pose` / `scale_anchor` / `scale_velocity` で行う
/// （Java は `AnimationBuilder.loadPose` L206-211 がロード時に適用するのと同じ位置）。
/// `sound` / `volume` は Java `AnimationBuilder` L221-237 の `Sound` / `Volume` 属性。
/// Java はパース時に音声ファイルをロードして Clip のキーを保持するが、Rust は音声の
/// 実体を持たないため**生のファイル名と音量**を保持し、パス解決（`img/<set>/sound/` →
/// `sound/<set>/` → `sound/`・Java `Main.getSoundFilePath` L446-461）とロードは
/// 再生バックエンド（Phase 2）に残す（design §1.10 (z-12)）。
#[derive(Debug, Clone)]
pub struct Pose {
    pub image: String,
    pub anchor: (i32, i32),
    pub velocity: (i32, i32),
    pub duration: i32,
    /// XML `Sound` 属性（任意）。ファイル名のみ（パス解決は再生側）。
    pub sound: Option<String>,
    /// XML `Volume` 属性（任意・既定 0.0・Java L227-230）。
    pub volume: f32,
}

/// behaviors.xml のパース結果。
#[derive(Debug, Clone, Default)]
pub struct BehaviorsConfig {
    pub entries: Vec<BehaviorEntry>,
    /// Mascot 直下の `<Constant>` / `<定数>` の Name → Value（Java
    /// `Configuration.constants` 相当）。生文字列のまま保持し、行動条件の評価文脈へ
    /// 注入する（[`BehaviorTable::new`](crate::mascot::behavior::BehaviorTable::new)
    /// が [`Variable::parse`] する）。
    pub constants: BTreeMap<String, String>,
}

/// Behavior エントリ。Condition ノードで束ねられたグループ（AND 積み上げ）or 単体。
#[derive(Debug, Clone)]
pub enum BehaviorEntry {
    Group {
        conditions: Vec<Variable>,
        behaviors: Vec<BehaviorDef>,
    },
    Single(BehaviorDef),
}

/// Behavior 定義（Java `BehaviorBuilder` 相当）。
#[derive(Debug, Clone)]
pub struct BehaviorDef {
    pub name: String,
    pub frequency: i32,
    pub hidden: bool,
    /// Allowed Behaviours トグル対象（Java `BehaviorBuilder.toggleable` 相当）。
    /// 属性省略時 / 必須 4 種（ChaseMouse / Fall / Dragged / Thrown）は強制 false
    /// （Java BehaviorBuilder.java L169-176 逐語）。
    pub toggleable: bool,
    /// 対応アクション。既定は「Behavior 名（または Action 属性）と同名の参照」。
    /// attrs（残りの属性）は Gap 等の注入変数候補になる。
    pub action: SequenceChild,
    /// 子 `<NextBehaviorList>`（Java nextBehaviorBuilders 相当）。
    pub next: Option<NextBehaviorList>,
}

/// `<NextBehaviorList>`（Add 属性 + BehaviorReference 群）。
#[derive(Debug, Clone)]
pub struct NextBehaviorList {
    pub add: bool,
    pub references: Vec<BehaviorRef>,
}

/// `<BehaviorReference>`（次 Behavior 候補）。
#[derive(Debug, Clone)]
pub struct BehaviorRef {
    pub name: String,
    pub frequency: i32,
    pub condition: Option<Variable>,
}

/// actions.xml をパースする（Java: config/Configuration.java#load の ActionList 読み込み部分）。
/// ルート Mascot 配下の ActionList（複数可）の子 Action を順に登録する。重複 Name は Err。
pub fn parse_actions(path: &Path) -> Result<ActionsConfig, ConfigError> {
    let text = read_file(path)?;
    let doc = parse_document(&text, path)?;
    let cx = Cx::new(&doc, path);
    let root = doc.root_element();
    if root.tag_name().name() != "Mascot" {
        // Java: UnrecognizedRootTagNameErrorMessage 相当
        return cx.error(
            root,
            format!("unknown root tag: {}", root.tag_name().name()),
        );
    }

    let mut actions = BTreeMap::new();
    for list in element_children(root, "ActionList") {
        for node in element_children(list, "Action") {
            let (name, def) = parse_action_def(&cx, node, true, 0)?;
            if actions.contains_key(&name) {
                // Java: DuplicateActionErrorMessage 相当
                return cx.error(node, format!("duplicate Action `{name}`"));
            }
            actions.insert(name, def);
        }
    }
    warn_legacy_tint(&cx, root);
    Ok(ActionsConfig { actions })
}

/// 旧形式（`<Mascot>` の `Tint` 属性と、その子の `<TintPalette>`）を 1 回だけ知らせる。
///
/// 色の宣言は `conf/<set>/tint.xml` へ移った（2026-09-23・スライス 11）。旧形式は
/// **読めるが無視する**（無色になる）ので、黙って色が消えないように警告する。
fn warn_legacy_tint(cx: &Cx, root: Node) {
    let has_tint_attribute = root.attribute("Tint").is_some();
    let has_palette = element_children(root, "TintPalette").next().is_some();
    if has_tint_attribute || has_palette {
        log::warn!(
            "{}:{}: the colour declaration moved to conf/<set>/tint.xml: ignoring `Tint` / <TintPalette> here",
            cx.file,
            cx.line_of(root)
        );
    }
}

/// `tint.xml`（set の色の宣言）を読む（設計: `docs/plans/design-gaming-color.md` §色の宣言と許可色）。
///
/// ルート `<TintPalette>` の属性がモードと既定値、子 `<Color>` が色（と出現の許可）。
/// ファイル不在 / 読み込み失敗 / パース失敗 / ルート要素違いは**警告 + 無色**で、
/// 起動は止めない（未対応の script 式と同じ方針）。ファイル不在は警告もしない
/// （探索側が「無ければ既定」を決める）。
pub fn parse_tint(path: &Path) -> TintConfig {
    if !path.is_file() {
        return TintConfig::default();
    }
    let text = match read_file(path) {
        Ok(text) => text,
        Err(err) => {
            log::warn!("{err}: treating as no colouring");
            return TintConfig::default();
        }
    };
    let doc = match parse_document(&text, path) {
        Ok(doc) => doc,
        Err(err) => {
            log::warn!("{err}: treating as no colouring");
            return TintConfig::default();
        }
    };
    let cx = Cx::new(&doc, path);
    let root = doc.root_element();
    if root.tag_name().name() != "TintPalette" {
        log::warn!(
            "{}:{}: unknown root tag `{}`: treating as no colouring",
            cx.file,
            cx.line_of(root),
            root.tag_name().name()
        );
        return TintConfig::default();
    }

    let style = parse_tint_style(&cx, root);
    // `<Color>` の省略値は宣言値に依存するため、宣言 → 色の順に読む
    let palette = parse_tint_colours(&cx, root, &style);
    TintConfig { style, palette }
}

/// ルート `<TintPalette>` の宣言（モードと既定値）を読む。
///
/// `Mode` の値は `random`（出現のたびに許可色から抽選）/ `cycle`（全色相を回す。別名 `rainbow`）。
/// **省略・`off`・`none` = 色づけなし**で、それ以外の値（`within` / `steps` / `#RRGGBB` など）は
/// **警告 + 無色**（起動は止めない）。`Start` / `Speed` / `Sat` / `Lum` / `Glow` は
/// 移行前に `<Mascot>` の `TintStart` / `TintSpeed` / `TintSat` / `TintLum` / `TintGlow` が
/// 持っていたのと同じ意味（接頭辞 `Tint` を落としただけ）。
fn parse_tint_style(cx: &Cx, root: Node) -> TintStyle {
    let mut style = TintStyle::default();
    let Some(text) = root.attribute("Mode") else {
        return style;
    };
    style.mode = match text {
        "off" | "none" => TintMode::Off,
        "rainbow" | "cycle" => TintMode::Cycle,
        "random" => TintMode::Random,
        other => {
            log::warn!(
                "{}:{}: unknown Mode value `{other}`: treating as no tinting",
                cx.file,
                cx.line_of(root)
            );
            TintMode::Off
        }
    };
    // 回転速度の既定は「回す宣言のときだけ 150」。抽選（random）では 0（出現時に固定される）
    let default_rotate = if style.mode == TintMode::Cycle {
        crate::tint::DEFAULT_ROTATE
    } else {
        0.0
    };
    style.rotate = tint_num_attr(cx, root, "Speed", default_rotate);
    style.sat = tint_num_attr(cx, root, "Sat", crate::tint::DEFAULT_SAT);
    style.lum = tint_num_attr(cx, root, "Lum", crate::tint::DEFAULT_LUM);
    style.glow = tint_num_attr(cx, root, "Glow", crate::tint::DEFAULT_GLOW);
    style.start = tint_start_attr(cx, root, "Start");
    style
}

/// ルート属性の初期位相（`Start`）。**未指定 = 抽選**（出現のたびに位相を決める）。
///
/// 値は 0 以上 360 未満へ wrap する（負値と 360 以上を許す）。不正値は警告して
/// **未指定と同じ扱い（抽選）**にする。
fn tint_start_attr(cx: &Cx, node: Node, name: &str) -> StartPhase {
    let Some(text) = node.attribute(name) else {
        return StartPhase::Random;
    };
    match text.trim().parse::<f32>() {
        Ok(value) if value.is_finite() => StartPhase::Fixed(value.rem_euclid(360.0)),
        _ => {
            log::warn!(
                "{}:{}: invalid {name} `{text}`: drawing the phase at spawn",
                cx.file,
                cx.line_of(node)
            );
            StartPhase::Random
        }
    }
}

/// ルート属性の数値。未指定は `default`、パース失敗・非有限は警告 + `default`。
fn tint_num_attr(cx: &Cx, node: Node, name: &str, default: f32) -> f32 {
    let Some(text) = node.attribute(name) else {
        return default;
    };
    match text.trim().parse::<f32>() {
        Ok(value) if value.is_finite() => value,
        _ => {
            log::warn!(
                "{}:{}: invalid {name} `{text}`: using {default}",
                cx.file,
                cx.line_of(node)
            );
            default
        }
    }
}

/// ルート `<TintPalette>` の `<Color>` を読む。無ければ空（= 色なし）。
///
/// `<Color>` は `Id` が必須。`Id` 欠落・不正な文字・重複は**その色だけ**捨てて警告する
/// （重複は先勝ち = 最初に書いた色が残る）。数値の不正も同じ扱い（起動は止めない）。
fn parse_tint_colours(cx: &Cx, root: Node, decl: &TintStyle) -> Palette {
    let mut colors: Vec<PaletteColor> = Vec::new();
    for color in element_children(root, "Color") {
        let Some(parsed) = parse_palette_color(cx, color, decl) else {
            continue;
        };
        if colors.iter().any(|kept| kept.id == parsed.id) {
            log::warn!(
                "{}:{}: duplicate colour Id `{}`: keeping the first",
                cx.file,
                cx.line_of(color),
                parsed.id
            );
            continue;
        }
        colors.push(parsed);
    }
    Palette::from_colors(colors)
}

/// `<Color>` 1 色。`Id` 欠落・不正な `Id` と不正な数値は警告 + `None`（その色を捨てる）。
fn parse_palette_color(cx: &Cx, node: Node, decl: &TintStyle) -> Option<PaletteColor> {
    let line = cx.line_of(node);
    let Some(id) = node
        .attribute("Id")
        .map(str::trim)
        .filter(|id| !id.is_empty())
    else {
        log::warn!("{}:{line}: <Color> without Id: skipped", cx.file);
        return None;
    };
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        log::warn!("{}:{line}: invalid colour Id `{id}`: skipped", cx.file);
        return None;
    }
    Some(PaletteColor {
        id: id.to_string(),
        // `Name` 省略時は `Id` を表示名にする（辞書は通さない = 作者データ）
        name: node
            .attribute("Name")
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(id)
            .to_string(),
        hue: palette_num_attr(cx, node, "Hue", 0.0)?.rem_euclid(360.0),
        sat: palette_num_attr(cx, node, "Sat", decl.sat)?,
        lum: palette_num_attr(cx, node, "Lum", decl.lum)?,
        glow: palette_num_attr(cx, node, "Glow", decl.glow)?,
        allowed: palette_bool_attr(cx, node, "Allowed", true),
    })
}

/// `<Color>` の真偽属性（`Allowed`）。未指定は `default`、`true` / `false` 以外は警告 + `default`。
///
/// 出現を許可するかの判定は `false` だけが「出さない」（省略 = 許可）なので、
/// タイポを黙って「出さない」に倒さないよう既定へ戻して知らせる。
fn palette_bool_attr(cx: &Cx, node: Node, name: &str, default: bool) -> bool {
    let Some(text) = node.attribute(name) else {
        return default;
    };
    match text.trim().to_ascii_lowercase().as_str() {
        "true" => true,
        "false" => false,
        _ => {
            log::warn!(
                "{}:{}: invalid {name} `{text}`: using {}",
                cx.file,
                cx.line_of(node),
                if default { "true" } else { "false" }
            );
            default
        }
    }
}

/// `<Color>` の数値属性。未指定は `default`、パース失敗・非有限は警告 + `None`（その色を捨てる）。
fn palette_num_attr(cx: &Cx, node: Node, name: &str, default: f32) -> Option<f32> {
    let Some(text) = node.attribute(name) else {
        return Some(default);
    };
    match text.trim().parse::<f32>() {
        Ok(value) if value.is_finite() => Some(value),
        _ => {
            log::warn!(
                "{}:{}: invalid {name} `{text}`: dropping this colour",
                cx.file,
                cx.line_of(node)
            );
            None
        }
    }
}

/// behaviors.xml をパースする
/// （Java: config/Configuration.java#load の BehaviourList 部分 + #loadBehaviors）。
pub fn parse_behaviors(path: &Path) -> Result<BehaviorsConfig, ConfigError> {
    let text = read_file(path)?;
    let doc = parse_document(&text, path)?;
    let cx = Cx::new(&doc, path);
    let root = doc.root_element();
    if root.tag_name().name() != "Mascot" {
        return cx.error(
            root,
            format!("unknown root tag: {}", root.tag_name().name()),
        );
    }

    let mut entries = Vec::new();
    let mut seen_names = HashSet::new();
    let mut constants = BTreeMap::new();
    // `<Constant>` / `<定数>`（Mascot 直下）。Java Configuration.java L158-171 逐語:
    // Name / Value は必須（欠落は ConfigurationException = fail-fast）。
    for node in root
        .children()
        .filter(|n| n.is_element() && matches!(n.tag_name().name(), "Constant" | "定数"))
    {
        let name = node
            .attribute("Name")
            .ok_or_else(|| cx.error_value(node, "Constant is missing the Name attribute"))?;
        let value = node
            .attribute("Value")
            .or_else(|| node.attribute("値"))
            .ok_or_else(|| cx.error_value(node, "Constant is missing the Value attribute"))?;
        constants.insert(name.to_string(), value.to_string());
    }
    for list in root.children().filter(|n| {
        n.is_element() && matches!(n.tag_name().name(), "BehaviorList" | "BehaviourList")
    }) {
        parse_behavior_list(&cx, list, &[], &mut entries, &mut seen_names, 0)?;
    }
    Ok(BehaviorsConfig { entries, constants })
}

/// 必須 4 種の Behavior（ChaseMouse / Fall / Dragged / Thrown）が揃っているか検証する
/// （Java: Configuration#validate の必須 Behavior チェック部分）。
pub fn validate_required_behaviors(config: &BehaviorsConfig) -> Result<(), ConfigError> {
    const REQUIRED: [&str; 4] = ["ChaseMouse", "Fall", "Dragged", "Thrown"];
    let names: HashSet<&str> = config
        .entries
        .iter()
        .flat_map(|entry| match entry {
            BehaviorEntry::Group { behaviors, .. } => behaviors
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>(),
            BehaviorEntry::Single(b) => vec![b.name.as_str()],
        })
        .collect();
    let missing: Vec<&str> = REQUIRED
        .iter()
        .copied()
        .filter(|name| !names.contains(name))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ConfigError {
            file: "(behaviors)".to_string(),
            line: 0,
            reason: format!("missing required behaviors: {}", missing.join(", ")),
        })
    }
}

// =============================================================================
// 内部: ファイル読み込みとパース進行の共有状態
// =============================================================================

fn read_file(path: &Path) -> Result<String, ConfigError> {
    fs::read_to_string(path).map_err(|e| ConfigError {
        file: path.display().to_string(),
        line: 0,
        reason: format!("failed to read conf file: {e}"),
    })
}

/// XML テキストを roxmltree ドキュメントへ。BOM（UTF-8）は roxmltree が自動処理する。
/// 先に要素ネスト深さを事前スキャンして過深入力を弾く（#21）。
fn parse_document<'a>(text: &'a str, path: &Path) -> Result<Document<'a>, ConfigError> {
    check_xml_nesting(text, path)?;
    Document::parse(text).map_err(|e| ConfigError {
        file: path.display().to_string(),
        line: e.pos().row,
        reason: format!("failed to parse XML: {e}"),
    })
}

/// 生 XML テキストの要素ネスト深さを事前スキャンし、上限超過なら ConfigError を返す（#21）。
/// roxmltree 自身の `parse_element` / `parse_content` が要素ネストで再帰するため、
/// `Document::parse` に渡す前に生テキスト段階で拒否しないとスタックオーバーフローする。
/// コメント / CDATA / PI / DOCTYPE と引用符内は読み飛ばし、開始/終了タグで深さを数える。
fn check_xml_nesting(text: &str, path: &Path) -> Result<(), ConfigError> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut depth = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if bytes[i..].starts_with(b"<!--") {
            i += 4;
            while i < bytes.len() && !bytes[i..].starts_with(b"-->") {
                i += 1;
            }
            i = (i + 3).min(bytes.len());
            continue;
        }
        if bytes[i..].starts_with(b"<![CDATA[") {
            i += 9;
            while i < bytes.len() && !bytes[i..].starts_with(b"]]>") {
                i += 1;
            }
            i = (i + 3).min(bytes.len());
            continue;
        }
        if bytes[i..].starts_with(b"</") {
            depth = depth.saturating_sub(1);
            i = skip_tag(bytes, i + 2);
            continue;
        }
        if bytes[i..].starts_with(b"<?") || bytes[i..].starts_with(b"<!") {
            // 宣言 / PI / DOCTYPE は深さに数えない
            i = skip_tag(bytes, i + 2);
            continue;
        }
        // 開始タグ。`>` 直前の非空白が `/` なら自己閉じで深さは増えない。
        let after = skip_tag(bytes, i + 1);
        let mut k = after;
        let mut self_closing = false;
        while k > i + 1 {
            k -= 1;
            let b = bytes[k];
            if b == b'>' || b.is_ascii_whitespace() {
                continue;
            }
            self_closing = b == b'/';
            break;
        }
        if !self_closing {
            depth += 1;
            if depth > MAX_XML_NESTING {
                let line = bytes[..i].iter().filter(|b| **b == b'\n').count() as u32 + 1;
                return Err(ConfigError {
                    file: path.display().to_string(),
                    line,
                    reason: format!("XML element nesting too deep (limit {MAX_XML_NESTING})"),
                });
            }
        }
        i = after;
    }
    Ok(())
}

/// `from` から引用符内を飛ばしつつ `>` まで読み飛ばし、`>` の次を返す（#21 事前スキャン用）。
fn skip_tag(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                i += 1;
            }
            b'>' => return i + 1,
            _ => i += 1,
        }
    }
    i
}

/// 直接の子要素のうちローカル名が一致するものを順に返す（名前空間はローカル名比較）。
fn element_children<'a, 'input>(
    node: Node<'a, 'input>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    node.children()
        .filter(move |n| n.is_element() && n.tag_name().name() == name)
}

/// パース進行の共有状態（ドキュメント参照 + ファイル名）。エラーに行番号を付ける。
struct Cx<'doc, 'input> {
    doc: &'doc Document<'input>,
    file: String,
}

impl<'doc, 'input> Cx<'doc, 'input> {
    fn new(doc: &'doc Document<'input>, path: &Path) -> Self {
        Cx {
            doc,
            file: path.display().to_string(),
        }
    }

    /// 要素ノードの行番号。range 先端 = 開始タグ（`<`）の位置。
    fn line_of(&self, node: Node<'doc, 'input>) -> u32 {
        self.doc.text_pos_at(node.range().start).row
    }

    fn error_value(&self, node: Node<'doc, 'input>, reason: impl Into<String>) -> ConfigError {
        ConfigError {
            file: self.file.clone(),
            line: self.line_of(node),
            reason: reason.into(),
        }
    }

    fn error<T>(
        &self,
        node: Node<'doc, 'input>,
        reason: impl Into<String>,
    ) -> Result<T, ConfigError> {
        Err(self.error_value(node, reason))
    }

    /// #21: 要素ネスト深さの上限チェック。超過は ConfigError（panic しない）。
    fn check_nesting(&self, node: Node<'doc, 'input>, depth: usize) -> Result<(), ConfigError> {
        if depth > MAX_XML_NESTING {
            Err(self.error_value(
                node,
                format!("XML element nesting too deep (limit {MAX_XML_NESTING})"),
            ))
        } else {
            Ok(())
        }
    }
}

// =============================================================================
// actions.xml
// =============================================================================

/// Action の Type 属性値（Java ActionBuilder の TYPE_* 定数相当）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionKind {
    Embedded,
    Move,
    Stay,
    Animate,
    Sequence,
    Select,
}

/// Action / 匿名 Action ノードをパースする（Java: config/ActionBuilder.java#new）。
/// トップレベルと匿名（インライン）アクションの両方に対応する。
/// 戻り値は (Name 属性値, 構築した ActionDef)。匿名アクションの Name は空文字列。
fn parse_action_def(
    cx: &Cx,
    node: Node,
    top_level: bool,
    depth: usize,
) -> Result<(String, ActionDef), ConfigError> {
    cx.check_nesting(node, depth)?;
    // Type 属性は必須。不正値は Err（Java: UnknownActionTypeErrorMessage 相当）
    let type_text = node
        .attribute("Type")
        .ok_or_else(|| cx.error_value(node, "Action is missing the Type attribute"))?;
    let kind = match type_text {
        "Embedded" => ActionKind::Embedded,
        "Move" => ActionKind::Move,
        "Stay" => ActionKind::Stay,
        "Animate" => ActionKind::Animate,
        "Sequence" => ActionKind::Sequence,
        "Select" => ActionKind::Select,
        _ => return cx.error(node, format!("unknown Action Type: {type_text}")),
    };

    // Name はトップレベルのみ必須（匿名アクションは名前を持たない）
    let name = if top_level {
        node.attribute("Name")
            .ok_or_else(|| cx.error_value(node, "Action is missing the Name attribute"))?
            .to_string()
    } else {
        String::new()
    };

    // Class は Embedded のみ必須。Java は Class.forName で存在確認するが、
    // Rust 版は FQN 文字列を保持するのみ（対応クラスはタスク #7 の ActionKind で判別）。
    let class = match kind {
        ActionKind::Embedded => Some(
            node.attribute("Class")
                .ok_or_else(|| {
                    cx.error_value(node, "Embedded Action is missing the Class attribute")
                })?
                .to_string(),
        ),
        _ => None,
    };

    // BorderType（Java BorderedAction.java L24/L40-48 逐語: 省略時は None（border 無効）。
    // 既知値はそのまま Some。未知値はエラー（#3 で据え置きの fail-fast 強化））
    let border = match node.attribute("BorderType") {
        None => None,
        Some("Floor") => Some(BorderType::Floor),
        Some("Wall") => Some(BorderType::Wall),
        Some("Ceiling") => Some(BorderType::Ceiling),
        Some(other) => return cx.error(node, format!("invalid BorderType value: {other}")),
    };

    // Loop（Sequence / Select のみ使用される・Java Sequence.java L19-20。
    // Boolean.parseBoolean 相当 = "true" のみ true・省略 = false）
    let mut is_loop = false;
    if let Some(loop_text) = node.attribute("Loop") {
        is_loop = loop_text.eq_ignore_ascii_case("true");
    }

    // attrs: パーサが消費した属性（Type / Class / BorderType / トップレベルの Name）以外を
    // 全て Variable 化する（Java は全属性を params に入れるが、Name/Type/Class/Border は
    // 専用フィールドへ分離したため除く）。
    let mut attrs = VarMap::new();
    for attr in node.attributes() {
        let attr_name = attr.name();
        let consumed = attr_name == "Type"
            || attr_name == "Class"
            || attr_name == "BorderType"
            || (top_level && attr_name == "Name");
        if !consumed {
            attrs.insert(attr_name.to_string(), Variable::parse(attr.value()));
        }
    }

    // 子要素: Animation（全型で保持。Java buildAction も全型で animations を受け取る）
    // と ActionReference / 匿名 Action（ComplexAction 型のみ。それ以外は Java 同様エラー）。
    let complex = matches!(kind, ActionKind::Sequence | ActionKind::Select);
    let mut animations = Vec::new();
    let mut children = Vec::new();
    for child in node.children().filter(|c| c.is_element()) {
        match child.tag_name().name() {
            "Animation" => animations.push(parse_animation(cx, child)?),
            "ActionReference" => {
                if !complex {
                    return cx.error(
                        child,
                        format!("{type_text} Action cannot have child actions"),
                    );
                }
                children.push(parse_action_ref(cx, child)?);
            }
            "Action" => {
                if !complex {
                    return cx.error(
                        child,
                        format!("{type_text} Action cannot have child actions"),
                    );
                }
                let (_, inline) = parse_action_def(cx, child, false, depth + 1)?;
                children.push(SequenceChild::Inline(Box::new(inline)));
            }
            _ => {} // 未知の子要素は無視（Java 同様）
        }
    }
    if complex && children.is_empty() {
        // Java: NoChildActionsErrorMessage 相当
        return cx.error(
            node,
            format!("{type_text} Action requires at least one child action"),
        );
    }

    let def = match kind {
        ActionKind::Embedded => ActionDef::Embedded {
            class: class.unwrap_or_default(),
            border,
            attrs,
            animations,
        },
        ActionKind::Stay => ActionDef::Stay {
            border,
            attrs,
            animations,
        },
        ActionKind::Move => ActionDef::Move {
            border,
            attrs,
            animations,
        },
        ActionKind::Animate => ActionDef::Animate {
            border,
            attrs,
            animations,
        },
        ActionKind::Sequence => ActionDef::Sequence {
            border,
            attrs,
            is_loop,
            animations,
            children,
        },
        ActionKind::Select => ActionDef::Select {
            border,
            attrs,
            is_loop,
            animations,
            children,
        },
    };
    Ok((name, def))
}

/// ActionReference ノードをパースする（Java: config/ActionRef.java#new）。
/// Name 以外の全属性を attrs へ（Duration / Condition / TargetX / Gap 等の注入変数候補）。
fn parse_action_ref(cx: &Cx, node: Node) -> Result<SequenceChild, ConfigError> {
    let name = node
        .attribute("Name")
        .ok_or_else(|| cx.error_value(node, "ActionReference is missing the Name attribute"))?
        .to_string();
    let mut attrs = VarMap::new();
    for attr in node.attributes() {
        if attr.name() != "Name" {
            attrs.insert(attr.name().to_string(), Variable::parse(attr.value()));
        }
    }
    Ok(SequenceChild::Ref { name, attrs })
}

/// Animation ノードをパースする（Java: config/AnimationBuilder.java#new）。
/// Condition / IsTurn 属性は任意（IsTurn は "true" のみ true・Boolean.parseBoolean 相当）。
/// Pose が 1 つも無い場合は Err（Java: NoPosesInAnimationErrorMessage 相当）。
fn parse_animation(cx: &Cx, node: Node) -> Result<Animation, ConfigError> {
    let condition = node.attribute("Condition").map(Variable::parse);
    // Java AnimationBuilder L89-90: hasAttribute && Boolean.parseBoolean(attribute)
    let is_turn = node
        .attribute("IsTurn")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let mut poses = Vec::new();
    for child in element_children(node, "Pose") {
        poses.push(parse_pose(cx, child)?);
    }
    if poses.is_empty() {
        return cx.error(node, "Animation has no poses");
    }
    Ok(Animation {
        condition,
        poses,
        is_turn,
    })
}

/// Pose ノードをパースする（Java: config/AnimationBuilder.java#loadPose）。
/// 必須属性: Image / ImageAnchor / Velocity / Duration（欠落は Err）。
/// 任意属性: Sound（ファイル名）/ Volume（既定 0・Java L221-237）。
fn parse_pose(cx: &Cx, node: Node) -> Result<Pose, ConfigError> {
    let image = node
        .attribute("Image")
        .ok_or_else(|| cx.error_value(node, "Pose is missing the Image attribute"))?
        .to_string();
    let anchor = parse_xy2(cx, node, "ImageAnchor")?;
    let velocity = parse_xy2(cx, node, "Velocity")?;
    let duration_text = node
        .attribute("Duration")
        .ok_or_else(|| cx.error_value(node, "Pose is missing the Duration attribute"))?;
    let duration = duration_text.parse::<i32>().map_err(|_| {
        cx.error_value(
            node,
            format!("Duration is not a valid integer: {duration_text}"),
        )
    })?;
    // Java L221-237: Sound 属性があるときだけ音を扱う（無ければ null）。
    // 実体（デコード）は持たないためファイル名のまま保持する。
    let sound = node.attribute("Sound").map(str::to_string);
    // Java L227-230: Volume は任意で既定 0。数値化失敗は Java と同じくロード失敗。
    let volume = match node.attribute("Volume") {
        Some(text) => text
            .parse::<f32>()
            .map_err(|_| cx.error_value(node, format!("Volume is not a valid number: {text}")))?,
        None => 0.0,
    };
    Ok(Pose {
        image,
        anchor,
        velocity,
        duration,
        sound,
        volume,
    })
}

/// "x,y" 形式の Pose 属性を 2 つの i32 へ（Java: split(",") + Integer.parseInt 相当）。
/// 3 要素目以降は無視（Java も [0] / [1] のみ使用）。要素不足や整数化失敗は Err。
fn parse_xy2(cx: &Cx, node: Node, attr_name: &str) -> Result<(i32, i32), ConfigError> {
    let text = node.attribute(attr_name).ok_or_else(|| {
        cx.error_value(node, format!("Pose is missing the {attr_name} attribute"))
    })?;
    let mut parts = text.split(',');
    let x_text = parts.next().unwrap_or_default();
    let y_text = parts.next().unwrap_or_default();
    match (x_text.parse::<i32>(), y_text.parse::<i32>()) {
        (Ok(x), Ok(y)) => Ok((x, y)),
        _ => cx.error(
            node,
            format!("{attr_name} must be an integer pair in `x,y` format: {text}"),
        ),
    }
}

// =============================================================================
// behaviors.xml
// =============================================================================

/// BehaviorList（Condition / Behaviour ノードのリスト）を走査する
/// （Java: Configuration#loadBehaviors — Condition ノードで条件を積み上げて再帰する）。
fn parse_behavior_list(
    cx: &Cx,
    list: Node,
    inherited: &[Variable],
    entries: &mut Vec<BehaviorEntry>,
    seen_names: &mut HashSet<String>,
    depth: usize,
) -> Result<(), ConfigError> {
    cx.check_nesting(list, depth)?;
    for node in list.children().filter(|c| c.is_element()) {
        match node.tag_name().name() {
            "Condition" => {
                let mut conditions = inherited.to_vec();
                if let Some(source) = node.attribute("Condition") {
                    conditions.push(parse_group_condition(source));
                }
                parse_behavior_list(cx, node, &conditions, entries, seen_names, depth + 1)?;
            }
            "Behavior" | "Behaviour" => {
                // Behavior 自身の Condition 属性も Group.conditions へ AND 積み上げする
                // （Java: BehaviorBuilder.java 147-166 で条件リストに追加される）。
                // 条件が付いた Behavior は単体の Group として entries に積まれる。
                let mut conditions = inherited.to_vec();
                if let Some(source) = node.attribute("Condition") {
                    conditions.push(parse_group_condition(source));
                }
                let def = parse_behavior_def(cx, node, seen_names)?;
                if conditions.is_empty() {
                    entries.push(BehaviorEntry::Single(def));
                } else {
                    entries.push(BehaviorEntry::Group {
                        conditions,
                        behaviors: vec![def],
                    });
                }
            }
            _ => {} // 未知の子要素は無視（Java 同様）
        }
    }
    Ok(())
}

/// Behavior のグループ条件（Condition ノード / Behavior 自身の Condition 属性）。
/// Java はこれらを文字列のまま保持し、isEffective() ごとに Variable.parse で再評価する
/// （＝常に新規評価）。これは `#{}`（毎フレーム再評価）と同義であるため、
/// `${}` 記法であっても allow_value_reset=true に正規化して積む。
fn parse_group_condition(source: &str) -> Variable {
    match Variable::parse(source) {
        Variable::Script { source, .. } => Variable::Script {
            source,
            allow_value_reset: true,
        },
        other => other,
    }
}

/// Behavior ノードをパースする（Java: config/BehaviorBuilder.java#new）。
/// Name / Frequency は必須（欠落は Err）。重複名は Err。
fn parse_behavior_def(
    cx: &Cx,
    node: Node,
    seen_names: &mut HashSet<String>,
) -> Result<BehaviorDef, ConfigError> {
    let name = node
        .attribute("Name")
        .ok_or_else(|| cx.error_value(node, "Behavior is missing the Name attribute"))?
        .to_string();
    if !seen_names.insert(name.clone()) {
        // Java: DuplicateBehaviourErrorMessage 相当（グループをまたいで一意である必要がある）
        return cx.error(node, format!("duplicate Behavior `{name}`"));
    }
    let frequency_text = node.attribute("Frequency").ok_or_else(|| {
        cx.error_value(
            node,
            format!("Behavior `{name}` is missing the Frequency attribute"),
        )
    })?;
    let frequency = frequency_text.parse::<i32>().map_err(|_| {
        cx.error_value(
            node,
            format!("Behavior `{name}` Frequency is not a valid integer: {frequency_text}"),
        )
    })?;
    let hidden = node
        .attribute("Hidden")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    // (C) Toggleable（Java BehaviorBuilder.java L169-176 逐語）:
    // 属性が無い場合 / 必須 4 種（ChaseMouse / Fall / Thrown / Dragged）のときは
    // 常に false。それ以外は Boolean.parseBoolean 相当（"true" の大小無視のみ true）。
    const TOGGLEABLE_RESERVED: [&str; 4] = ["ChaseMouse", "Fall", "Dragged", "Thrown"];
    let toggleable = match node.attribute("Toggleable") {
        Some(value) if !TOGGLEABLE_RESERVED.contains(&name.as_str()) => {
            value.eq_ignore_ascii_case("true")
        }
        _ => false,
    };
    let action_attr = node.attribute("Action").map(|s| s.to_string());

    // 子要素: NextBehaviorList（別名 NextBehavior / UK 綴り）/ ActionReference / 匿名 Action
    let mut next = None;
    let mut child_action: Option<SequenceChild> = None;
    for child in node.children().filter(|c| c.is_element()) {
        match child.tag_name().name() {
            "NextBehaviorList" | "NextBehaviourList" | "NextBehavior" | "NextBehaviour" => {
                // 複数ある場合は後勝ち（Java は references を連結するが、契約は 1 リストのみ。
                // 資産では Behavior あたり 1 個しか現れない）
                next = Some(parse_next_behavior_list(cx, child)?);
            }
            "ActionReference" => child_action = Some(parse_action_ref(cx, child)?),
            "Action" => {
                let (_, inline) = parse_action_def(cx, child, false, 0)?;
                child_action = Some(SequenceChild::Inline(Box::new(inline)));
            }
            _ => {}
        }
    }

    // 対応アクション。子要素で指定が無ければ「Behavior 名（または Action 属性）と同名の参照」。
    // attrs はプログラム用属性（Name / Action / Frequency / Hidden / Condition / Toggleable）
    // を除いた残り（Java BehaviorBuilder の params 相当）。
    let action = match child_action {
        Some(action) => action,
        None => {
            let mut attrs = VarMap::new();
            for attr in node.attributes() {
                let attr_name = attr.name();
                if matches!(
                    attr_name,
                    "Name" | "Action" | "Frequency" | "Hidden" | "Condition" | "Toggleable"
                ) {
                    continue;
                }
                attrs.insert(attr_name.to_string(), Variable::parse(attr.value()));
            }
            SequenceChild::Ref {
                name: action_attr.unwrap_or_else(|| name.clone()),
                attrs,
            }
        }
    };

    Ok(BehaviorDef {
        name,
        frequency,
        hidden,
        toggleable,
        action,
        next,
    })
}

/// NextBehaviorList ノードをパースする（Java: BehaviorBuilder#new の NextBehaviourList 読み込み部分）。
/// Add 属性は必須（Java 同様欠落は Err）。
fn parse_next_behavior_list(cx: &Cx, node: Node) -> Result<NextBehaviorList, ConfigError> {
    let add_text = node
        .attribute("Add")
        .ok_or_else(|| cx.error_value(node, "NextBehaviorList is missing the Add attribute"))?;
    let add = add_text.eq_ignore_ascii_case("true"); // Java Boolean.parseBoolean 相当
    let mut references = Vec::new();
    parse_next_list_children(cx, node, &[], &mut references, 0)?;
    Ok(NextBehaviorList { add, references })
}

/// NextBehaviorList 配下を走査する（Java: BehaviorBuilder#loadBehaviors —
/// 入れ子 Condition を AND 積み上げして再帰する）。
fn parse_next_list_children(
    cx: &Cx,
    list: Node,
    inherited: &[(String, bool)],
    references: &mut Vec<BehaviorRef>,
    depth: usize,
) -> Result<(), ConfigError> {
    cx.check_nesting(list, depth)?;
    for node in list.children().filter(|c| c.is_element()) {
        match node.tag_name().name() {
            "Condition" => {
                let mut conditions = inherited.to_vec();
                if let Some(source) = node.attribute("Condition") {
                    // (生の式ソース, #{} かどうか) を保持する。
                    // 参照条件は資産の記法（${} / #{}）をそのまま反映する。
                    conditions.push((source.to_string(), source.starts_with("#{")));
                }
                parse_next_list_children(cx, node, &conditions, references, depth + 1)?;
            }
            "BehaviorReference" | "BehaviourReference" => {
                let name = node
                    .attribute("Name")
                    .ok_or_else(|| {
                        cx.error_value(node, "BehaviorReference is missing the Name attribute")
                    })?
                    .to_string();
                let frequency_text = node.attribute("Frequency").ok_or_else(|| {
                    cx.error_value(
                        node,
                        format!("BehaviorReference `{name}` is missing the Frequency attribute"),
                    )
                })?;
                let frequency = frequency_text.parse::<i32>().map_err(|_| {
                    cx.error_value(
                        node,
                        format!("BehaviorReference `{name}` Frequency is not a valid integer: {frequency_text}"),
                    )
                })?;

                let mut conditions = inherited.to_vec();
                if let Some(source) = node.attribute("Condition") {
                    conditions.push((source.to_string(), source.starts_with("#{")));
                }
                // 契約上 BehaviorRef は条件を 1 個のみ保持するため、
                // 複数の Condition が積み上がった場合は AND 結合する。
                let condition = match conditions.len() {
                    0 => None,
                    1 => Some(Variable::parse(&conditions[0].0)),
                    _ => {
                        let joined = conditions
                            .iter()
                            .map(|(source, _)| format!("({source})"))
                            .collect::<Vec<_>>()
                            .join(" && ");
                        let allow = conditions
                            .iter()
                            .any(|(_, allow_value_reset)| *allow_value_reset);
                        Some(Variable::Script {
                            source: joined,
                            allow_value_reset: allow,
                        })
                    }
                };
                references.push(BehaviorRef {
                    name,
                    frequency,
                    condition,
                });
            }
            _ => {}
        }
    }
    Ok(())
}
