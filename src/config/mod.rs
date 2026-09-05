//! conf XML（actions.xml / behaviors.xml）の強型パース — Java `config` パッケージ相当。
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

use script::Variable;

/// XML 属性値のマップ（属性名 → [`Variable`]。Java の params 相当）。
pub type VarMap = BTreeMap<String, Variable>;

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
#[derive(Debug, Clone)]
pub struct ActionsConfig {
    pub actions: BTreeMap<String, ActionDef>,
}

/// 境界種別（Action の BorderType 属性。省略時は Floor）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorderType {
    Floor,
    Wall,
    Ceiling,
}

/// アクション定義（Java `ActionBuilder` 相当）。
/// 全バリアントが animations を持つ（Java buildAction が全型で animations を受け取るため）。
#[derive(Debug, Clone)]
pub enum ActionDef {
    Embedded {
        class: String,
        border: BorderType,
        attrs: VarMap,
        animations: Vec<Animation>,
    },
    Stay {
        border: BorderType,
        attrs: VarMap,
        animations: Vec<Animation>,
    },
    Move {
        border: BorderType,
        attrs: VarMap,
        animations: Vec<Animation>,
    },
    Animate {
        border: BorderType,
        attrs: VarMap,
        animations: Vec<Animation>,
    },
    Sequence {
        border: BorderType,
        attrs: VarMap,
        animations: Vec<Animation>,
        children: Vec<SequenceChild>,
    },
    Select {
        border: BorderType,
        attrs: VarMap,
        animations: Vec<Animation>,
        children: Vec<SequenceChild>,
    },
}

/// Sequence / Select の子。ActionReference 参照 or 匿名 Action のインライン定義。
#[derive(Debug, Clone)]
pub enum SequenceChild {
    Ref {
        name: String,
        attrs: VarMap,
    },
    Inline(Box<ActionDef>),
}

/// アニメーション（Java `AnimationBuilder` 相当）。
#[derive(Debug, Clone)]
pub struct Animation {
    pub condition: Option<Variable>,
    pub poses: Vec<Pose>,
}

/// ポーズ（画像パス・アンカー・速度・フレーム数。Java `Pose` 相当）。
/// scale 適用（画像 + アンカー dx/dy へのプリスケール）は画像セット側（タスク #4）で行うため生値を保持する。
#[derive(Debug, Clone)]
pub struct Pose {
    pub image: String,
    pub anchor: (i32, i32),
    pub velocity: (i32, i32),
    pub duration: i32,
}

/// behaviors.xml のパース結果。
#[derive(Debug, Clone)]
pub struct BehaviorsConfig {
    pub entries: Vec<BehaviorEntry>,
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
        return cx.error(root, format!("未知のルートタグ: {}", root.tag_name().name()));
    }

    let mut actions = BTreeMap::new();
    for list in element_children(root, "ActionList") {
        for node in element_children(list, "Action") {
            let (name, def) = parse_action_def(&cx, node, true)?;
            if actions.contains_key(&name) {
                // Java: DuplicateActionErrorMessage 相当
                return cx.error(node, format!("Action `{name}` が重複定義されています"));
            }
            actions.insert(name, def);
        }
    }
    Ok(ActionsConfig { actions })
}

/// behaviors.xml をパースする
/// （Java: config/Configuration.java#load の BehaviourList 部分 + #loadBehaviors）。
pub fn parse_behaviors(path: &Path) -> Result<BehaviorsConfig, ConfigError> {
    let text = read_file(path)?;
    let doc = parse_document(&text, path)?;
    let cx = Cx::new(&doc, path);
    let root = doc.root_element();
    if root.tag_name().name() != "Mascot" {
        return cx.error(root, format!("未知のルートタグ: {}", root.tag_name().name()));
    }

    let mut entries = Vec::new();
    let mut seen_names = HashSet::new();
    for list in root
        .children()
        .filter(|n| n.is_element() && matches!(n.tag_name().name(), "BehaviorList" | "BehaviourList"))
    {
        parse_behavior_list(&cx, list, &[], &mut entries, &mut seen_names)?;
    }
    Ok(BehaviorsConfig { entries })
}

/// 必須 4 種の Behavior（ChaseMouse / Fall / Dragged / Thrown）が揃っているか検証する
/// （Java: Configuration#validate の必須 Behavior チェック部分）。
pub fn validate_required_behaviors(config: &BehaviorsConfig) -> Result<(), ConfigError> {
    const REQUIRED: [&str; 4] = ["ChaseMouse", "Fall", "Dragged", "Thrown"];
    let names: HashSet<&str> = config
        .entries
        .iter()
        .flat_map(|entry| match entry {
            BehaviorEntry::Group { behaviors, .. } => {
                behaviors.iter().map(|b| b.name.as_str()).collect::<Vec<_>>()
            }
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
            reason: format!("必須 Behavior が欠落しています: {}", missing.join(", ")),
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
        reason: format!("conf ファイルを読めません: {e}"),
    })
}

/// XML テキストを roxmltree ドキュメントへ。BOM（UTF-8）は roxmltree が自動処理する。
fn parse_document<'a>(text: &'a str, path: &Path) -> Result<Document<'a>, ConfigError> {
    Document::parse(text).map_err(|e| ConfigError {
        file: path.display().to_string(),
        line: e.pos().row,
        reason: format!("XML として解釈できません: {e}"),
    })
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

    fn error<T>(&self, node: Node<'doc, 'input>, reason: impl Into<String>) -> Result<T, ConfigError> {
        Err(self.error_value(node, reason))
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
) -> Result<(String, ActionDef), ConfigError> {
    // Type 属性は必須。不正値は Err（Java: UnknownActionTypeErrorMessage 相当）
    let type_text = node
        .attribute("Type")
        .ok_or_else(|| cx.error_value(node, "Action に Type 属性がありません"))?;
    let kind = match type_text {
        "Embedded" => ActionKind::Embedded,
        "Move" => ActionKind::Move,
        "Stay" => ActionKind::Stay,
        "Animate" => ActionKind::Animate,
        "Sequence" => ActionKind::Sequence,
        "Select" => ActionKind::Select,
        _ => return cx.error(node, format!("未知の Action Type: {type_text}")),
    };

    // Name はトップレベルのみ必須（匿名アクションは名前を持たない）
    let name = if top_level {
        node.attribute("Name")
            .ok_or_else(|| cx.error_value(node, "Action に Name 属性がありません"))?
            .to_string()
    } else {
        String::new()
    };

    // Class は Embedded のみ必須。Java は Class.forName で存在確認するが、
    // Rust 版は FQN 文字列を保持するのみ（対応クラスはタスク #7 の ActionKind で判別）。
    let class = match kind {
        ActionKind::Embedded => Some(
            node.attribute("Class")
                .ok_or_else(|| cx.error_value(node, "Embedded Action に Class 属性がありません"))?
                .to_string(),
        ),
        _ => None,
    };

    // BorderType（省略時 Floor）
    let border = match node.attribute("BorderType") {
        None | Some("Floor") => BorderType::Floor,
        Some("Wall") => BorderType::Wall,
        Some("Ceiling") => BorderType::Ceiling,
        Some(other) => return cx.error(node, format!("BorderType の値が不正です: {other}")),
    };

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
                        format!("{type_text} 型の Action は子アクションを持てません"),
                    );
                }
                children.push(parse_action_ref(cx, child)?);
            }
            "Action" => {
                if !complex {
                    return cx.error(
                        child,
                        format!("{type_text} 型の Action は子アクションを持てません"),
                    );
                }
                let (_, inline) = parse_action_def(cx, child, false)?;
                children.push(SequenceChild::Inline(Box::new(inline)));
            }
            _ => {} // 未知の子要素は無視（Java 同様）
        }
    }
    if complex && children.is_empty() {
        // Java: NoChildActionsErrorMessage 相当
        return cx.error(
            node,
            format!("{type_text} 型の Action には子アクションが 1 つ以上必要です"),
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
            animations,
            children,
        },
        ActionKind::Select => ActionDef::Select {
            border,
            attrs,
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
        .ok_or_else(|| cx.error_value(node, "ActionReference に Name 属性がありません"))?
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
/// Condition 属性は任意。Pose が 1 つも無い場合は Err（Java: NoPosesInAnimationErrorMessage 相当）。
fn parse_animation(cx: &Cx, node: Node) -> Result<Animation, ConfigError> {
    let condition = node.attribute("Condition").map(Variable::parse);
    let mut poses = Vec::new();
    for child in element_children(node, "Pose") {
        poses.push(parse_pose(cx, child)?);
    }
    if poses.is_empty() {
        return cx.error(node, "Animation に Pose が 1 つもありません");
    }
    Ok(Animation { condition, poses })
}

/// Pose ノードをパースする（Java: config/AnimationBuilder.java#loadPose）。
/// 必須属性: Image / ImageAnchor / Velocity / Duration（欠落は Err）。
fn parse_pose(cx: &Cx, node: Node) -> Result<Pose, ConfigError> {
    let image = node
        .attribute("Image")
        .ok_or_else(|| cx.error_value(node, "Pose に Image 属性がありません"))?
        .to_string();
    let anchor = parse_xy2(cx, node, "ImageAnchor")?;
    let velocity = parse_xy2(cx, node, "Velocity")?;
    let duration_text = node
        .attribute("Duration")
        .ok_or_else(|| cx.error_value(node, "Pose に Duration 属性がありません"))?;
    let duration = duration_text.parse::<i32>().map_err(|_| {
        cx.error_value(
            node,
            format!("Duration が整数として解釈できません: {duration_text}"),
        )
    })?;
    Ok(Pose {
        image,
        anchor,
        velocity,
        duration,
    })
}

/// "x,y" 形式の Pose 属性を 2 つの i32 へ（Java: split(",") + Integer.parseInt 相当）。
/// 3 要素目以降は無視（Java も [0] / [1] のみ使用）。要素不足や整数化失敗は Err。
fn parse_xy2(cx: &Cx, node: Node, attr_name: &str) -> Result<(i32, i32), ConfigError> {
    let text = node
        .attribute(attr_name)
        .ok_or_else(|| cx.error_value(node, format!("Pose に {attr_name} 属性がありません")))?;
    let mut parts = text.split(',');
    let x_text = parts.next().unwrap_or_default();
    let y_text = parts.next().unwrap_or_default();
    match (x_text.parse::<i32>(), y_text.parse::<i32>()) {
        (Ok(x), Ok(y)) => Ok((x, y)),
        _ => cx.error(
            node,
            format!("{attr_name} は `x,y` 形式の整数である必要があります: {text}"),
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
) -> Result<(), ConfigError> {
    for node in list.children().filter(|c| c.is_element()) {
        match node.tag_name().name() {
            "Condition" => {
                let mut conditions = inherited.to_vec();
                if let Some(source) = node.attribute("Condition") {
                    conditions.push(parse_group_condition(source));
                }
                parse_behavior_list(cx, node, &conditions, entries, seen_names)?;
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
        .ok_or_else(|| cx.error_value(node, "Behavior に Name 属性がありません"))?
        .to_string();
    if !seen_names.insert(name.clone()) {
        // Java: DuplicateBehaviourErrorMessage 相当（グループをまたいで一意である必要がある）
        return cx.error(node, format!("Behavior `{name}` が重複定義されています"));
    }
    let frequency_text = node
        .attribute("Frequency")
        .ok_or_else(|| cx.error_value(node, format!("Behavior `{name}` に Frequency 属性がありません")))?;
    let frequency = frequency_text.parse::<i32>().map_err(|_| {
        cx.error_value(
            node,
            format!("Behavior `{name}` の Frequency が整数として解釈できません: {frequency_text}"),
        )
    })?;
    let hidden = node
        .attribute("Hidden")
        .is_some_and(|v| v.eq_ignore_ascii_case("true"));
    let action_attr = node.attribute("Action").map(|s| s.to_string());

    // 子要素: NextBehaviorList / ActionReference / 匿名 Action
    let mut next = None;
    let mut child_action: Option<SequenceChild> = None;
    for child in node.children().filter(|c| c.is_element()) {
        match child.tag_name().name() {
            "NextBehaviorList" | "NextBehaviourList" => {
                // 複数ある場合は後勝ち（Java は references を連結するが、契約は 1 リストのみ。
                // 資産では Behavior あたり 1 個しか現れない）
                next = Some(parse_next_behavior_list(cx, child)?);
            }
            "ActionReference" => child_action = Some(parse_action_ref(cx, child)?),
            "Action" => {
                let (_, inline) = parse_action_def(cx, child, false)?;
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
        action,
        next,
    })
}

/// NextBehaviorList ノードをパースする（Java: BehaviorBuilder#new の NextBehaviourList 読み込み部分）。
/// Add 属性は必須（Java 同様欠落は Err）。
fn parse_next_behavior_list(cx: &Cx, node: Node) -> Result<NextBehaviorList, ConfigError> {
    let add_text = node
        .attribute("Add")
        .ok_or_else(|| cx.error_value(node, "NextBehaviorList に Add 属性がありません"))?;
    let add = add_text.eq_ignore_ascii_case("true"); // Java Boolean.parseBoolean 相当
    let mut references = Vec::new();
    parse_next_list_children(cx, node, &[], &mut references)?;
    Ok(NextBehaviorList { add, references })
}

/// NextBehaviorList 配下を走査する（Java: BehaviorBuilder#loadBehaviors —
/// 入れ子 Condition を AND 積み上げして再帰する）。
fn parse_next_list_children(
    cx: &Cx,
    list: Node,
    inherited: &[(String, bool)],
    references: &mut Vec<BehaviorRef>,
) -> Result<(), ConfigError> {
    for node in list.children().filter(|c| c.is_element()) {
        match node.tag_name().name() {
            "Condition" => {
                let mut conditions = inherited.to_vec();
                if let Some(source) = node.attribute("Condition") {
                    // (生の式ソース, #{} かどうか) を保持する。
                    // 参照条件は資産の記法（${} / #{}）をそのまま反映する。
                    conditions.push((source.to_string(), source.starts_with("#{")));
                }
                parse_next_list_children(cx, node, &conditions, references)?;
            }
            "BehaviorReference" | "BehaviourReference" => {
                let name = node
                    .attribute("Name")
                    .ok_or_else(|| cx.error_value(node, "BehaviorReference に Name 属性がありません"))?
                    .to_string();
                let frequency_text = node
                    .attribute("Frequency")
                    .ok_or_else(|| {
                        cx.error_value(
                            node,
                            format!("BehaviorReference `{name}` に Frequency 属性がありません"),
                        )
                    })?;
                let frequency = frequency_text.parse::<i32>().map_err(|_| {
                    cx.error_value(
                        node,
                        format!("BehaviorReference `{name}` の Frequency が整数として解釈できません: {frequency_text}"),
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
                        let allow = conditions.iter().any(|(_, allow_value_reset)| *allow_value_reset);
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
