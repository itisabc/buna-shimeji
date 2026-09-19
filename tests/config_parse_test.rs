//! タスク #3: src/config/mod.rs（XML 強型パース）の契約テスト。
//!
//! 検証対象は公開契約のみ:
//! - parse_actions / parse_behaviors が資産実物（UTF-8 BOM + CRLF）を全件ロードできる
//! - 構造: Action 92（型分布・Border・Embedded class・attrs 定数）/ Animation 39（Condition 10）/
//!   Pose 133（必須属性）/ ActionReference 198 / behaviors 57 + Condition グループ 10 +
//!   NextBehaviorList 7 + BehaviorReference 12
//! - XML 属性値 → Variable の構築ルール（${}/#{} → Script、true/false→Bool、数値→Number、他→Text）
//! - 入れ子 Condition の AND 積み上げ / Behavior 自身の Condition 属性の保持
//! - エラー処理: 破損 XML・重複名・必須 4 種欠落 → ConfigError（行番号は ConfigError 内部）
//!
//! 実装 (src/config/) は未存在のため cargo test は compile error = RED が正常。

mod common;

use std::path::{Path, PathBuf};

use common::{eval_ok, norm_ws, source_of, MockCtx};
use shimeji::config::script::{ConstantValue, EvalValue, Variable, Variables};
use shimeji::config::{
    parse_actions, parse_behaviors, validate_required_behaviors, ActionDef, ActionsConfig,
    Animation, BehaviorDef, BehaviorEntry, BehaviorsConfig, BorderType, Pose, SequenceChild,
};

// =====================================================================
// 共通ヘルパ
// =====================================================================

fn conf_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("conf")
        .join(name)
}

fn real_actions() -> ActionsConfig {
    parse_actions(&conf_path("actions.xml")).expect("conf/actions.xml をパースできる")
}

fn real_behaviors() -> BehaviorsConfig {
    parse_behaviors(&conf_path("behaviors.xml")).expect("conf/behaviors.xml をパースできる")
}

fn find_action<'a>(cfg: &'a ActionsConfig, name: &str) -> &'a ActionDef {
    cfg.actions
        .get(name)
        .unwrap_or_else(|| panic!("Action {name} が存在しない"))
}

/// Variable の説明文字列（assert 失敗時の表示用。派生トレイトに依存しない）。
fn var_desc(v: Option<&Variable>) -> String {
    match v {
        Some(v) => source_of(v),
        None => "<absent>".to_string(),
    }
}

fn expect_script<'a>(v: Option<&'a Variable>, where_: &str) -> (&'a String, bool) {
    match v {
        Some(Variable::Script {
            source,
            allow_value_reset,
        }) => (source, *allow_value_reset),
        _ => panic!("{where_} は Script のはずだが {}", var_desc(v)),
    }
}

fn expect_const_num(v: Option<&Variable>, where_: &str) -> f64 {
    match v {
        Some(Variable::Constant(ConstantValue::Number(n))) => *n,
        _ => panic!("{where_} は数値定数のはずだが {}", var_desc(v)),
    }
}

fn expect_const_bool(v: Option<&Variable>, where_: &str) -> bool {
    match v {
        Some(Variable::Constant(ConstantValue::Bool(b))) => *b,
        _ => panic!("{where_} はブール定数のはずだが {}", var_desc(v)),
    }
}

fn expect_const_text<'a>(v: Option<&'a Variable>, where_: &str) -> &'a str {
    match v {
        Some(Variable::Constant(ConstantValue::Text(t))) => t.as_str(),
        _ => panic!("{where_} はテキスト定数のはずだが {}", var_desc(v)),
    }
}

/// パース済みの Variable を評価する（注入変数を明示指定）。
fn eval_parsed(ctx: &MockCtx, var: &Variable, injected: &[(&str, f64)]) -> EvalValue {
    let mut vars = Variables::new();
    for (name, value) in injected {
        vars.inject(*name, *value);
    }
    match vars.eval(var, ctx) {
        Ok(v) => v,
        Err(_) => panic!("パース済み式 {:?} の評価に失敗", source_of(var)),
    }
}

fn eval_parsed_num(ctx: &MockCtx, var: &Variable, injected: &[(&str, f64)]) -> f64 {
    match eval_parsed(ctx, var, injected) {
        EvalValue::Number(n) => n,
        _ => panic!("パース済み式 {:?} は数値のはずが別の型", source_of(var)),
    }
}

fn eval_parsed_bool(ctx: &MockCtx, var: &Variable, injected: &[(&str, f64)]) -> bool {
    match eval_parsed(ctx, var, injected) {
        EvalValue::Bool(b) => b,
        _ => panic!("パース済み式 {:?} はブールのはずが別の型", source_of(var)),
    }
}

/// 子アクション列（Sequence/Select 以外は空）。
fn children_of(def: &ActionDef) -> &[SequenceChild] {
    match def {
        ActionDef::Sequence { children, .. } | ActionDef::Select { children, .. } => children,
        _ => &[],
    }
}

fn count_refs(def: &ActionDef) -> usize {
    children_of(def)
        .iter()
        .map(|c| match c {
            SequenceChild::Ref { .. } => 1,
            SequenceChild::Inline(inner) => count_refs(inner),
        })
        .sum()
}

/// 全 ActionDef 木から Animation を収集する。
///
/// 注: 契約上 `animations` は Animate にのみ明記されているが、Java ActionBuilder は
/// Embedded/Stay/Move/Animate の全型で animations を保持する（ActionBuilder.java の
/// buildAction: 全型が animations を受け取る）。資産の Animation 39 件の内訳は
/// Stay 11 / Move 7 / Animate 5 / Embedded 16 であり、全件検証（39/133）には
/// 全型での保持が前提になる。ここでは Java 準拠の形で検証する。
fn collect_animations<'a>(def: &'a ActionDef, out: &mut Vec<&'a Animation>) {
    match def {
        ActionDef::Embedded { animations, .. }
        | ActionDef::Stay { animations, .. }
        | ActionDef::Move { animations, .. }
        | ActionDef::Animate { animations, .. } => out.extend(animations.iter()),
        ActionDef::Sequence { children, .. } | ActionDef::Select { children, .. } => {
            for child in children {
                if let SequenceChild::Inline(inner) = child {
                    collect_animations(inner, out);
                }
            }
        }
    }
}

fn all_poses(cfg: &ActionsConfig) -> Vec<&Pose> {
    let mut anims = Vec::new();
    for def in cfg.actions.values() {
        collect_animations(def, &mut anims);
    }
    anims.iter().flat_map(|a| a.poses.iter()).collect()
}

fn border_of(def: &ActionDef) -> &Option<BorderType> {
    match def {
        ActionDef::Embedded { border, .. }
        | ActionDef::Stay { border, .. }
        | ActionDef::Move { border, .. }
        | ActionDef::Animate { border, .. }
        | ActionDef::Sequence { border, .. }
        | ActionDef::Select { border, .. } => border,
    }
}

/// 条件 Variable を比較可能な文字列へ（空白正規化）。
fn cond_source(v: &Variable) -> String {
    match v {
        Variable::Script { source, .. } => norm_ws(source),
        Variable::Constant(ConstantValue::Bool(b)) => b.to_string(),
        Variable::Constant(ConstantValue::Number(n)) => format!("num:{n}"),
        Variable::Constant(ConstantValue::Text(t)) => format!("text:{t}"),
    }
}

/// 全 Behavior を (所属条件群の参照, BehaviorDef) に展開。
/// Single は条件なし、Group はその conditions スライスを共有して返す。
fn walk_behaviors(cfg: &BehaviorsConfig) -> Vec<(Option<&[Variable]>, &BehaviorDef)> {
    let mut out = Vec::new();
    for entry in &cfg.entries {
        match entry {
            BehaviorEntry::Group {
                conditions,
                behaviors,
            } => {
                for b in behaviors {
                    out.push((Some(conditions.as_slice()), b));
                }
            }
            BehaviorEntry::Single(b) => out.push((None, b)),
        }
    }
    out
}

fn cond_sources(conds: &[Variable]) -> Vec<String> {
    conds.iter().map(cond_source).collect()
}

fn temp_conf(tag: &str, content: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("shimeji_t3_{}_{tag}.xml", std::process::id()));
    std::fs::write(&path, content).expect("一時 conf ファイルを書ける");
    path
}

const ACTIONS_XML_HEAD: &str = concat!(
    "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
    "<ActionList>\n"
);

// =====================================================================
// 実物 actions.xml: 構造と件数
// =====================================================================

#[test]
fn real_actions_load_with_92_unique_actions() {
    let cfg = real_actions();
    assert_eq!(
        cfg.actions.len(),
        92,
        "Action 定義数（2 つの ActionList 合計）"
    );
    // 重複名があると BTreeMap に潰れて件数が合わない（パーサは重複で Err になる別テストあり）
    for name in [
        "Look",
        "Offset",
        "Stand",
        "Walk",
        "Run",
        "Dash",
        "Sit",
        "Sprawl",
        "GrabWall",
        "GrabCeiling",
        "ClimbWall",
        "ClimbCeiling",
        "Fall",
        "Dragged",
        "Thrown",
        "ChaseMouse",
        "SplitIntoTwo",
        "PullUpShimeji",
        "Divided",
    ] {
        assert!(cfg.actions.contains_key(name), "Action {name} が存在する");
    }
}

#[test]
fn real_actions_type_distribution() {
    // asset-report §3-2: Sequence 59 / Embedded 12 / Stay 10 / Move 6 / Animate 5（Select はネストのみ）
    let cfg = real_actions();
    let (mut embedded, mut stay, mut mov, mut animate, mut sequence, mut select) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for def in cfg.actions.values() {
        match def {
            ActionDef::Embedded { .. } => embedded += 1,
            ActionDef::Stay { .. } => stay += 1,
            ActionDef::Move { .. } => mov += 1,
            ActionDef::Animate { .. } => animate += 1,
            ActionDef::Sequence { .. } => sequence += 1,
            ActionDef::Select { .. } => select += 1,
        }
    }
    assert_eq!(embedded, 12);
    assert_eq!(stay, 10);
    assert_eq!(mov, 6);
    assert_eq!(animate, 5);
    assert_eq!(sequence, 59);
    assert_eq!(select, 0);
}

#[test]
fn real_actions_border_types() {
    // asset-report §3-2: 明示 BorderType 23 定義（Floor 19 / Wall 2 / Ceiling 2）。
    // 設計補正 design §1.8(g)（Java BorderedAction.java L24/L40-48 逐語）:
    // 属性省略 = None（border 無効）で、省略時 Floor への折り畳みは廃止。
    // 省略 69 定義（Fall/Jump/Dragged 等の床スナップ不要アクション）は None にパースされる。
    let cfg = real_actions();
    let (mut floor, mut wall, mut ceiling, mut none) = (0usize, 0usize, 0usize, 0usize);
    for def in cfg.actions.values() {
        match border_of(def) {
            Some(BorderType::Floor) => floor += 1,
            Some(BorderType::Wall) => wall += 1,
            Some(BorderType::Ceiling) => ceiling += 1,
            None => none += 1,
        }
    }
    // 明示 Floor 19 箇所: conf/actions.xml L11 Stand / L17 Walk / L26 Run / L35 Dash /
    // L46 Sit / L52 SitAndLookUp / L58 SitAndLookAtMouse / L67 SitAndSpinHeadAction /
    // L80 SitWithLegsUp / L86 SitWithLegsDown / L92 SitAndDangleLegs / L103 Sprawl /
    // L109 Creep / L180 WalkWithIe / L189 RunWithIe / L198 ThrowIe /
    // L221 Bouncing / L228 Tripping / L718 HitGround
    assert_eq!(floor, 19, "明示 BorderType=\"Floor\" の定義数");
    // L142 GrabWall / L148 ClimbWall
    assert_eq!(wall, 2, "明示 BorderType=\"Wall\" の定義数");
    // L121 GrabCeiling / L127 ClimbCeiling
    assert_eq!(ceiling, 2, "明示 BorderType=\"Ceiling\" の定義数");
    assert_eq!(
        none, 69,
        "BorderType 省略の定義数（= None・Floor 折り畳みなし）"
    );
    // 明示指定の実例（行番号は conf/actions.xml）
    assert!(matches!(
        border_of(find_action(&cfg, "GrabWall")),
        Some(BorderType::Wall)
    ));
    assert!(matches!(
        border_of(find_action(&cfg, "ClimbWall")),
        Some(BorderType::Wall)
    ));
    assert!(matches!(
        border_of(find_action(&cfg, "GrabCeiling")),
        Some(BorderType::Ceiling)
    ));
    assert!(matches!(
        border_of(find_action(&cfg, "ClimbCeiling")),
        Some(BorderType::Ceiling)
    ));
    assert!(matches!(
        border_of(find_action(&cfg, "Stand")),
        Some(BorderType::Floor)
    ));
}

#[test]
fn real_actions_animations_39_and_poses_133() {
    let cfg = real_actions();
    let mut anims = Vec::new();
    for def in cfg.actions.values() {
        collect_animations(def, &mut anims);
    }
    assert_eq!(anims.len(), 39, "Animation 総数");
    assert_eq!(
        anims.iter().filter(|a| a.condition.is_some()).count(),
        10,
        "Condition 付き Animation 数"
    );
    let poses = all_poses(&cfg);
    assert_eq!(poses.len(), 133, "Pose 総数");
    // 全 Pose が必須属性（Image/ImageAnchor/Velocity/Duration）を持ってパース済み
    for pose in &poses {
        assert!(
            pose.image.starts_with("/shime"),
            "Pose の Image: {}",
            pose.image
        );
        assert!(
            pose.duration > 0,
            "Pose の Duration は正: {}",
            pose.duration
        );
    }
}

#[test]
fn real_actions_pose_fields_spot_check() {
    let cfg = real_actions();
    // Stand: 1 アニメ 1 ポーズ
    let stand = find_action(&cfg, "Stand");
    let mut anims = Vec::new();
    collect_animations(stand, &mut anims);
    assert_eq!(anims.len(), 1);
    assert_eq!(anims[0].poses.len(), 1);
    let pose = &anims[0].poses[0];
    assert_eq!(pose.image, "/shime1.png");
    assert_eq!(pose.anchor, (64, 128));
    assert_eq!(pose.velocity, (0, 0));
    assert_eq!(pose.duration, 250);
    // Walk: 速度 (-2,0)・Duration 6（lookRight=false 時は Rust 側で反転する前提の生値）
    let mut walk_anims = Vec::new();
    collect_animations(find_action(&cfg, "Walk"), &mut walk_anims);
    assert_eq!(walk_anims.len(), 1);
    assert_eq!(walk_anims[0].poses.len(), 4);
    assert_eq!(walk_anims[0].poses[0].velocity, (-2, 0));
    assert_eq!(walk_anims[0].poses[0].duration, 6);
    // ClimbWall: Condition 付きアニメ 2 件
    let mut climb_anims = Vec::new();
    collect_animations(find_action(&cfg, "ClimbWall"), &mut climb_anims);
    assert_eq!(climb_anims.len(), 2);
    assert!(climb_anims.iter().all(|a| a.condition.is_some()));
}

#[test]
fn real_actions_actionreference_count_198() {
    let cfg = real_actions();
    let total: usize = cfg.actions.values().map(count_refs).sum();
    assert_eq!(total, 198, "ActionReference 総数（ネスト含む）");
}

#[test]
fn real_actions_file_has_two_action_lists() {
    // asset-report §3-1: actions.xml は <ActionList> を 2 回持つ
    let text = std::fs::read_to_string(conf_path("actions.xml")).unwrap();
    let doc = roxmltree::Document::parse(&text).unwrap();
    let lists = doc
        .root_element()
        .children()
        .filter(|n| n.tag_name().name() == "ActionList")
        .count();
    assert_eq!(lists, 2);
}

#[test]
fn real_actions_embedded_classes() {
    let cfg = real_actions();
    let mut embedded = 0;
    for def in cfg.actions.values() {
        if let ActionDef::Embedded { class, .. } = def {
            embedded += 1;
            assert!(
                class.starts_with("com.group_finity.mascot.action."),
                "Embedded class は FQN: {class}"
            );
        }
    }
    assert_eq!(embedded, 12);
    match find_action(&cfg, "Look") {
        ActionDef::Embedded { class, .. } => {
            assert_eq!(class, "com.group_finity.mascot.action.Look");
        }
        _ => panic!("Look は Embedded"),
    }
    match find_action(&cfg, "Pinched") {
        ActionDef::Embedded { class, .. } => {
            assert_eq!(class, "com.group_finity.mascot.action.Dragged");
        }
        _ => panic!("Pinched は Embedded"),
    }
    match find_action(&cfg, "Divide1") {
        ActionDef::Embedded { class, .. } => {
            assert_eq!(class, "com.group_finity.mascot.action.Breed");
        }
        _ => panic!("Divide1 は Embedded"),
    }
}

// =====================================================================
// 実物 actions.xml: 属性 → Variable の構築ルール
// =====================================================================

#[test]
fn real_actions_attr_constants_bool_number_text() {
    let cfg = real_actions();
    // Fall（Sequence, Loop="false"）→ Constant(Bool(false))
    match find_action(&cfg, "Fall") {
        ActionDef::Sequence { attrs, .. } => {
            assert!(!expect_const_bool(attrs.get("Loop"), "Fall@Loop"));
        }
        _ => panic!("Fall は Sequence"),
    }
    // ThrowIe: InitialVX="32" InitialVY="-10" Gravity="0.5"
    match find_action(&cfg, "ThrowIe") {
        ActionDef::Embedded { attrs, .. } => {
            assert_eq!(
                expect_const_num(attrs.get("InitialVX"), "ThrowIe@InitialVX"),
                32.0
            );
            assert_eq!(
                expect_const_num(attrs.get("InitialVY"), "ThrowIe@InitialVY"),
                -10.0
            );
            assert_eq!(
                expect_const_num(attrs.get("Gravity"), "ThrowIe@Gravity"),
                0.5
            );
        }
        _ => panic!("ThrowIe は Embedded"),
    }
    // Falling: RegistanceX="0.05" RegistanceY="0.1" Gravity="2"
    match find_action(&cfg, "Falling") {
        ActionDef::Embedded { attrs, .. } => {
            assert_eq!(
                expect_const_num(attrs.get("RegistanceX"), "Falling@RegistanceX"),
                0.05
            );
            assert_eq!(
                expect_const_num(attrs.get("RegistanceY"), "Falling@RegistanceY"),
                0.1
            );
            assert_eq!(
                expect_const_num(attrs.get("Gravity"), "Falling@Gravity"),
                2.0
            );
        }
        _ => panic!("Falling は Embedded"),
    }
    // Jumping: VelocityParam="20"
    match find_action(&cfg, "Jumping") {
        ActionDef::Embedded { attrs, .. } => {
            assert_eq!(
                expect_const_num(attrs.get("VelocityParam"), "Jumping@VelocityParam"),
                20.0
            );
        }
        _ => panic!("Jumping は Embedded"),
    }
    // FallWithIe: IeOffsetX="6" IeOffsetY="-58"
    match find_action(&cfg, "FallWithIe") {
        ActionDef::Embedded { attrs, .. } => {
            assert_eq!(
                expect_const_num(attrs.get("IeOffsetX"), "FallWithIe@IeOffsetX"),
                6.0
            );
            assert_eq!(
                expect_const_num(attrs.get("IeOffsetY"), "FallWithIe@IeOffsetY"),
                -58.0
            );
        }
        _ => panic!("FallWithIe は Embedded"),
    }
    // PullUpShimeji1: BornX="-32" BornY="96" BornBehavior="PullUp"（数値にパース不能 → Text）
    match find_action(&cfg, "PullUpShimeji1") {
        ActionDef::Embedded { attrs, .. } => {
            assert_eq!(
                expect_const_num(attrs.get("BornX"), "PullUpShimeji1@BornX"),
                -32.0
            );
            assert_eq!(
                expect_const_num(attrs.get("BornY"), "PullUpShimeji1@BornY"),
                96.0
            );
            assert_eq!(
                expect_const_text(attrs.get("BornBehavior"), "PullUpShimeji1@BornBehavior"),
                "PullUp"
            );
        }
        _ => panic!("PullUpShimeji1 は Embedded"),
    }
    // ActionReference 上の bool 定数: <ActionReference Name="Look" LookRight="true"/>
    let walk_sit = find_action(&cfg, "WalkLeftAlongFloorAndSit");
    let look_ref = children_of(walk_sit)
        .iter()
        .find_map(|c| match c {
            SequenceChild::Ref { name, attrs } if name == "Look" => Some(attrs),
            _ => None,
        })
        .expect("WalkLeftAlongFloorAndSit の Look 参照");
    assert!(expect_const_bool(
        look_ref.get("LookRight"),
        "Look@LookRight"
    ));
}

#[test]
fn real_actions_ref_duration_script_evaluates() {
    // Fall の構造: [Ref(Falling), Inline(Select)]
    //   Select: [Inline(Sequence{condition=floor.isOn, children=[Ref(Bouncing), Ref(Stand Duration="${100+Math.random()*100}")]}),
    //            Ref(GrabWall Duration="100")]
    let cfg = real_actions();
    let fall = find_action(&cfg, "Fall");
    let select = match &children_of(fall)[1] {
        SequenceChild::Inline(inner) => inner,
        _ => panic!("Fall の第 2 子は Inline"),
    };
    let select_children = children_of(select);

    // Inline Sequence 内の Ref(Stand) の Duration は ${} スクリプト
    let inline_seq = match &select_children[0] {
        SequenceChild::Inline(inner) => inner,
        _ => panic!("Select[0] は Inline"),
    };
    let stand_ref = match &children_of(inline_seq)[1] {
        SequenceChild::Ref { name, attrs } => {
            assert_eq!(name, "Stand");
            attrs
        }
        _ => panic!("Inline Sequence[1] は Ref(Stand)"),
    };
    let (source, allow_reset) = expect_script(stand_ref.get("Duration"), "Stand@Duration");
    assert_eq!(norm_ws(source), "100+Math.random()*100");
    assert!(!allow_reset, "ドル記法は allow_value_reset=false");
    let ctx = MockCtx::new();
    for _ in 0..50 {
        let v = eval_parsed_num(&ctx, stand_ref.get("Duration").unwrap(), &[]);
        assert!(
            (100.0..200.0).contains(&v),
            "Duration 値 = {v} が [100,200) 外"
        );
    }

    // Select 直下の Ref(GrabWall) の Duration は数値定数
    let grab_ref = match &select_children[1] {
        SequenceChild::Ref { name, attrs } => {
            assert_eq!(name, "GrabWall");
            attrs
        }
        _ => panic!("Select[1] は Ref(GrabWall)"),
    };
    assert_eq!(
        expect_const_num(grab_ref.get("Duration"), "GrabWall@Duration"),
        100.0
    );
}

#[test]
fn real_actions_thrown_initial_velocity_evaluates() {
    // <ActionReference Name="Falling" InitialVX="${mascot.environment.cursor.dx}" InitialVY="${mascot.environment.cursor.dy}"/>
    let cfg = real_actions();
    let thrown = find_action(&cfg, "Thrown");
    let falling = match &children_of(thrown)[0] {
        SequenceChild::Ref { name, attrs } => {
            assert_eq!(name, "Falling");
            attrs
        }
        _ => panic!("Thrown の第 1 子は Ref(Falling)"),
    };
    let (vx_src, vx_reset) = expect_script(falling.get("InitialVX"), "Falling@InitialVX");
    assert_eq!(norm_ws(vx_src), "mascot.environment.cursor.dx");
    assert!(!vx_reset);
    let ctx = MockCtx::new(); // cursor.dx=5, cursor.dy=-2
    assert_eq!(
        eval_parsed_num(&ctx, falling.get("InitialVX").unwrap(), &[]),
        5.0
    );
    assert_eq!(
        eval_parsed_num(&ctx, falling.get("InitialVY").unwrap(), &[]),
        -2.0
    );
}

// =====================================================================
// 実物 actions.xml: ネスト構造と条件評価
// =====================================================================

#[test]
fn real_actions_fall_nested_sequence_select() {
    let cfg = real_actions();
    let fall = find_action(&cfg, "Fall");
    let children = children_of(fall);
    assert_eq!(children.len(), 2);
    // 第 1 子 = Ref(Falling)、第 2 子 = Inline(Select)
    match &children[0] {
        SequenceChild::Ref { name, .. } => assert_eq!(name, "Falling"),
        _ => panic!("Fall の第 1 子は Ref"),
    }
    let select = match &children[1] {
        SequenceChild::Inline(inner) => inner,
        _ => panic!("Fall の第 2 子は Inline"),
    };
    assert!(matches!(select.as_ref(), ActionDef::Select { .. }));
    let select_children = children_of(select);
    assert_eq!(select_children.len(), 2);
    // Select[0] = Inline(Sequence, Condition=...): 条件は attrs["Condition"] に保持される
    let inline_seq = match &select_children[0] {
        SequenceChild::Inline(inner) => inner,
        _ => panic!("Select[0] は Inline"),
    };
    match inline_seq.as_ref() {
        ActionDef::Sequence { attrs, .. } => {
            let (source, allow) =
                expect_script(attrs.get("Condition"), "Inline Sequence@Condition");
            assert_eq!(
                norm_ws(source),
                norm_ws("mascot.environment.floor.isOn(mascot.anchor)")
            );
            assert!(!allow, "資産はドル記法 → allow_value_reset=false");
        }
        _ => panic!("Inline は Sequence"),
    }
    // Inline Sequence の子 = [Ref(Bouncing), Ref(Stand)]
    let inline_children = children_of(inline_seq);
    assert_eq!(inline_children.len(), 2);
    match &inline_children[0] {
        SequenceChild::Ref { name, .. } => assert_eq!(name, "Bouncing"),
        _ => panic!(),
    }
    // 条件を評価: 床 on → true / 床 off → false
    let cond = match inline_seq.as_ref() {
        ActionDef::Sequence { attrs, .. } => attrs.get("Condition").unwrap(),
        _ => unreachable!(),
    };
    let ctx = MockCtx::new();
    assert!(eval_parsed_bool(&ctx, cond, &[]));
    let mut ctx_off = MockCtx::new();
    ctx_off.all_on = false;
    assert!(!eval_parsed_bool(&ctx_off, cond, &[]));
}

#[test]
fn real_actions_chasemouse_structure_and_gap() {
    let cfg = real_actions();
    let chase = find_action(&cfg, "ChaseMouse");
    let children = children_of(chase);
    // 3 つの条件付き Inline Sequence + 1 つの Inline Select + 6 つの Ref = 10 子
    assert_eq!(children.len(), 10);
    let inline_count = children
        .iter()
        .filter(|c| matches!(c, SequenceChild::Inline(_)))
        .count();
    assert_eq!(inline_count, 4);
    // 条件付き Inline の条件を評価（真ん中 2 つ: leftBorder||rightBorder 系）
    let cond_sources_in_chase: Vec<String> = children
        .iter()
        .filter_map(|c| match c {
            SequenceChild::Inline(inner) => match inner.as_ref() {
                ActionDef::Sequence { attrs, .. } => attrs
                    .get("Condition")
                    .map(|v| norm_ws(&expect_script(Some(v), "ChaseMouse 条件").0.clone())),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert!(
        cond_sources_in_chase.contains(&norm_ws("mascot.environment.ceiling.isOn(mascot.anchor)"))
    );
    assert!(cond_sources_in_chase.contains(&norm_ws(
        "mascot.environment.workArea.leftBorder.isOn(mascot.anchor) || mascot.environment.activeIE.rightBorder.isOn(mascot.anchor)"
    )));
    // Gap 属性を持つ Dash 参照を見つける
    let gap_ref = children
        .iter()
        .filter_map(|c| match c {
            SequenceChild::Ref { name, attrs } if attrs.contains_key("Gap") => Some((name, attrs)),
            _ => None,
        })
        .next()
        .expect("Gap 属性付き ActionReference");
    assert_eq!(gap_ref.0, "Dash");
    // TargetX="#{mascot.environment.cursor.x+Gap}" → allow_value_reset=true
    let (tx_src, tx_reset) = expect_script(gap_ref.1.get("TargetX"), "Dash@TargetX");
    assert_eq!(norm_ws(tx_src), "mascot.environment.cursor.x+Gap");
    assert!(tx_reset, "ハッシュ記法は allow_value_reset=true");
    // Gap="${mascot.anchor.x < mascot.environment.cursor.x ? -Math.min(...) : Math.min(...)}"
    let (gap_src, gap_reset) = expect_script(gap_ref.1.get("Gap"), "Dash@Gap");
    assert!(gap_src.contains("Math.min"));
    assert!(!gap_reset, "資産の Gap はドル記法");
    // TargetX を評価（Gap 注入）
    let ctx = MockCtx::new(); // cursor.x=300
    assert_eq!(
        eval_parsed_num(&ctx, gap_ref.1.get("TargetX").unwrap(), &[("Gap", -50.0)]),
        250.0
    );
    // Gap 式自体を評価: anchor.x=100 < cursor.x=300 → [-200, 0]
    for _ in 0..50 {
        let v = eval_parsed_num(&ctx, gap_ref.1.get("Gap").unwrap(), &[]);
        assert!((-200.0..=0.0).contains(&v), "Gap 式 = {v}");
    }
    // Look 参照の LookRight 属性: ${mascot.anchor.x < mascot.environment.cursor.x}
    let look_ref = children
        .iter()
        .filter_map(|c| match c {
            SequenceChild::Ref { name, attrs } if name == "Look" => Some(attrs),
            _ => None,
        })
        .next()
        .expect("Look 参照");
    let (lr_src, _) = expect_script(look_ref.get("LookRight"), "Look@LookRight");
    assert_eq!(
        norm_ws(lr_src),
        "mascot.anchor.x < mascot.environment.cursor.x"
    );
    assert!(eval_parsed_bool(
        &ctx,
        look_ref.get("LookRight").unwrap(),
        &[]
    ));
}

#[test]
fn real_actions_animation_conditions_evaluate() {
    let cfg = real_actions();
    let ctx = MockCtx::new();
    // SitAndLookAtMouse: アニメ 2 件、第 1 のみ Condition
    let mut sit_anims = Vec::new();
    collect_animations(find_action(&cfg, "SitAndLookAtMouse"), &mut sit_anims);
    assert_eq!(sit_anims.len(), 2);
    assert!(sit_anims[0].condition.is_some());
    assert!(sit_anims[1].condition.is_none());
    assert!(eval_parsed_bool(
        &ctx,
        sit_anims[0].condition.as_ref().unwrap(),
        &[]
    ));
    let mut ctx_low = MockCtx::new();
    ctx_low.cursor_y = 900.0;
    assert!(!eval_parsed_bool(
        &ctx_low,
        sit_anims[0].condition.as_ref().unwrap(),
        &[]
    ));

    // Pinched: アニメ 7 件すべて Condition あり（FootX 注入）
    let mut pinched_anims = Vec::new();
    collect_animations(find_action(&cfg, "Pinched"), &mut pinched_anims);
    assert_eq!(pinched_anims.len(), 7);
    assert!(pinched_anims.iter().all(|a| a.condition.is_some()));
    let mut ctx_p = MockCtx::new();
    ctx_p.cursor_x = 200.0;
    assert!(eval_parsed_bool(
        &ctx_p,
        pinched_anims[0].condition.as_ref().unwrap(),
        &[("FootX", 0.0)]
    ));
    assert!(!eval_parsed_bool(
        &ctx_p,
        pinched_anims[0].condition.as_ref().unwrap(),
        &[("FootX", 200.0)]
    ));

    // ClimbWall: アニメ 2 件（TargetY 注入、上下で条件が入れ替わる）
    let mut climb_anims = Vec::new();
    collect_animations(find_action(&cfg, "ClimbWall"), &mut climb_anims);
    assert!(eval_parsed_bool(
        &ctx,
        climb_anims[0].condition.as_ref().unwrap(),
        &[("TargetY", 0.0)]
    ));
    assert!(!eval_parsed_bool(
        &ctx,
        climb_anims[1].condition.as_ref().unwrap(),
        &[("TargetY", 0.0)]
    ));
    assert!(eval_parsed_bool(
        &ctx,
        climb_anims[1].condition.as_ref().unwrap(),
        &[("TargetY", 600.0)]
    ));
}

#[test]
fn real_actions_climb_along_wall_nan_targetx() {
    // 資産 455 行 ClimbAlongWall の ClimbCeiling 参照: else 側が Math.random 括弧抜け → NaN
    let cfg = real_actions();
    let climb = find_action(&cfg, "ClimbAlongWall");
    let ceiling_ref = match children_of(climb).last() {
        Some(SequenceChild::Ref { name, attrs }) => {
            assert_eq!(name, "ClimbCeiling");
            attrs
        }
        _ => panic!("ClimbAlongWall の最終子は Ref(ClimbCeiling)"),
    };
    let (src, allow) = expect_script(ceiling_ref.get("TargetX"), "ClimbCeiling@TargetX");
    assert!(src.contains("Math.random*100"), "括弧抜け式を含む: {src}");
    assert!(!allow);
    let ctx = MockCtx::new();
    let mut ctx_false = MockCtx::new();
    ctx_false.look_right = false;
    // lookRight=false → else 側 → NaN（Java 挙動: (int)NaN = 0）
    let v = eval_parsed_num(&ctx_false, ceiling_ref.get("TargetX").unwrap(), &[]);
    assert!(v.is_nan(), "else 側は NaN になる（実測 {v}）");
    assert_eq!(shimeji::config::script::to_java_int(v), 0);
    // lookRight=true → 真側 → workArea.left + random*100 ∈ [0, 100)
    for _ in 0..50 {
        let v = eval_parsed_num(&ctx, ceiling_ref.get("TargetX").unwrap(), &[]);
        assert!((0.0..100.0).contains(&v), "真側 TargetX = {v}");
    }
}

// =====================================================================
// 実物 behaviors.xml: 構造と件数
// =====================================================================

#[test]
fn real_behaviors_load_with_57_behaviors() {
    let cfg = real_behaviors();
    let walked = walk_behaviors(&cfg);
    assert_eq!(walked.len(), 57, "Behavior 総数（Group 内含む）");
    let mut names: Vec<&str> = walked.iter().map(|(_, b)| b.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 57, "Behavior 名は一意");
}

#[test]
fn real_behaviors_required_four_are_present() {
    let cfg = real_behaviors();
    let walked = walk_behaviors(&cfg);
    for name in ["ChaseMouse", "Fall", "Dragged", "Thrown"] {
        let b = walked
            .iter()
            .find(|(_, def)| def.name == name)
            .unwrap_or_else(|| panic!("必須 Behavior {name} が存在する"));
        assert_eq!(b.1.frequency, 0, "{name} の Frequency");
        assert!(b.1.hidden, "{name} は Hidden");
    }
    // アクション参照は既定で Behavior 名と同名
    for b in &walked {
        if b.1.name == "ChaseMouse" {
            match &b.1.action {
                SequenceChild::Ref { name, .. } => assert_eq!(name, "ChaseMouse"),
                _ => panic!("ChaseMouse の action は Ref"),
            }
        }
        if b.1.name == "Fall" {
            match &b.1.action {
                SequenceChild::Ref { name, .. } => assert_eq!(name, "Fall"),
                _ => panic!("Fall の action は Ref"),
            }
        }
    }
}

#[test]
fn real_behaviors_validate_required_ok() {
    let cfg = real_behaviors();
    validate_required_behaviors(&cfg).expect("実物 behaviors は必須 4 種を含む");
}

#[test]
fn real_behaviors_have_no_toggleable_flags() {
    // 資産 behaviors.xml には Toggleable 属性が 0 件（asset-report・BehaviorBuilder.java
    // L169-176 省略時 false）→ 全 Behavior の toggleable は false
    let cfg = real_behaviors();
    for (_, def) in walk_behaviors(&cfg) {
        assert!(!def.toggleable, "Behavior {} の toggleable", def.name);
    }
}

#[test]
fn synthetic_behavior_toggleable_attribute_parses_and_stays_out_of_action_attrs() {
    let xml = concat!(
        "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
        "<BehaviorList>\n",
        "<Behavior Name=\"On\" Frequency=\"1\" Toggleable=\"true\"/>\n",
        "<Behavior Name=\"ExplicitOff\" Frequency=\"2\" Toggleable=\"false\"/>\n",
        "<Behavior Name=\"Unset\" Frequency=\"3\" Extra=\"7\"/>\n",
        "</BehaviorList>\n</Mascot>\n"
    );
    let path = temp_conf("toggleable", xml);
    let result = parse_behaviors(&path);
    let _ = std::fs::remove_file(&path);
    let cfg = result.expect("Toggleable 属性を含む behaviors をパースできる");
    let walked = walk_behaviors(&cfg);
    assert_eq!(walked.len(), 3);

    let toggle_of = |name: &str| -> bool {
        walked
            .iter()
            .find(|(_, b)| b.name == name)
            .unwrap_or_else(|| panic!("Behavior {name}"))
            .1
            .toggleable
    };
    // Toggleable="true" → true（Java BehaviorBuilder.java L169-176 相当）
    assert!(toggle_of("On"));
    // Toggleable="false" → false
    assert!(!toggle_of("ExplicitOff"));
    // 省略時は false（= Java 既定値）
    assert!(!toggle_of("Unset"));

    // Toggleable は行動パラメータ（既定参照の attrs）に入らない（プログラムパラメータ・
    // 除外リスト相当。Hidden/Frequency 等と同様に ActionReference に渡されない）
    let befs: Vec<&BehaviorDef> = walked.iter().map(|(_, b)| *b).collect();
    let on = befs.iter().find(|b| b.name == "On").unwrap();
    let unset = befs.iter().find(|b| b.name == "Unset").unwrap();
    for def in [on, unset] {
        match &def.action {
            SequenceChild::Ref { attrs, .. } => {
                assert!(
                    !attrs.contains_key("Toggleable"),
                    "{}: Toggleable は action attrs に入らない",
                    def.name
                );
            }
            _ => panic!("既定の action 参照"),
        }
    }
    // Unset の通常属性 Extra は従来どおり attrs に入る
    match &unset.action {
        SequenceChild::Ref { attrs, .. } => {
            assert_eq!(expect_const_num(attrs.get("Extra"), "Unset@Extra"), 7.0);
        }
        _ => panic!("Unset の action 参照"),
    }
}

#[test]
fn real_behaviors_ten_condition_groups() {
    // asset-report §3-4: <Condition Condition="..."> グループ ×10
    let cfg = real_behaviors();
    let groups: Vec<&Vec<Variable>> = cfg
        .entries
        .iter()
        .filter_map(|e| match e {
            BehaviorEntry::Group { conditions, .. } => Some(conditions),
            _ => None,
        })
        .collect();
    assert!(groups.len() >= 10, "Condition グループは 10 以上");

    let all_group_conds: Vec<String> = groups.iter().flat_map(|cs| cond_sources(cs)).collect();
    const GROUP_CONDITIONS: [&str; 10] = [
        "mascot.environment.floor.isOn(mascot.anchor)",
        "mascot.environment.wall.isOn(mascot.anchor)",
        "mascot.environment.ceiling.isOn(mascot.anchor)",
        "mascot.environment.workArea.bottomBorder.isOn(mascot.anchor)",
        "mascot.lookRight ? mascot.environment.workArea.rightBorder.isOn(mascot.anchor) : mascot.environment.workArea.leftBorder.isOn(mascot.anchor)",
        "mascot.environment.workArea.topBorder.isOn(mascot.anchor)",
        "mascot.environment.activeIE.topBorder.isOn(mascot.anchor)",
        "mascot.lookRight ? mascot.environment.activeIE.leftBorder.isOn(mascot.anchor) : mascot.environment.activeIE.rightBorder.isOn(mascot.anchor)",
        "mascot.environment.activeIE.bottomBorder.isOn(mascot.anchor)",
        "mascot.environment.activeIE.visible",
    ];
    for cond in GROUP_CONDITIONS {
        assert!(
            all_group_conds.iter().any(|c| c == cond),
            "グループ条件が見つからない: {cond}"
        );
    }
    // グループ条件は #{...} 記法 → allow_value_reset=true
    for cs in &groups {
        for c in cs.iter() {
            match c {
                Variable::Script {
                    allow_value_reset, ..
                } => {
                    assert!(*allow_value_reset, "グループ条件はハッシュ記法");
                }
                Variable::Constant(ConstantValue::Bool(_)) => {}
                other => panic!("グループ条件が想定外: {}", cond_source(other)),
            }
        }
    }
    // 床グループ条件を評価: all_on=true なら真
    let floor_cond = groups
        .iter()
        .find_map(|cs| {
            cs.iter()
                .find(|v| cond_source(v) == "mascot.environment.floor.isOn(mascot.anchor)")
        })
        .expect("床グループ条件");
    let ctx = MockCtx::new();
    assert!(eval_parsed_bool(&ctx, floor_cond, &[]));
    let mut ctx_off = MockCtx::new();
    ctx_off.all_on = false;
    assert!(!eval_parsed_bool(&ctx_off, floor_cond, &[]));
}

#[test]
fn real_behaviors_behavior_own_conditions_preserved() {
    // Behavior 要素自身の Condition 属性は Java では条件リストに積まれる
    // （BehaviorBuilder.java 147-166）。契約に condition フィールドがないため、
    // Group の conditions への AND 積み上げで保持されるはず。
    let cfg = real_behaviors();
    let walked = walk_behaviors(&cfg);

    // JumpFromLeftWall（トップレベル・複数行条件）: 条件は自分の 1 件のみ
    let (conds, def) = walked
        .iter()
        .find(|(_, b)| b.name == "JumpFromLeftWall")
        .expect("JumpFromLeftWall");
    assert_eq!(def.frequency, 50);
    assert!(def.hidden);
    let conds = conds.expect("JumpFromLeftWall は条件を持つ");
    let sources = cond_sources(conds);
    assert_eq!(
        sources,
        vec![norm_ws(
            "!mascot.environment.workArea.leftBorder.isOn(mascot.anchor) && mascot.anchor.x < mascot.environment.workArea.left+400 && Math.abs(mascot.environment.workArea.bottom-mascot.anchor.y) <mascot.environment.workArea.height/4"
        )],
        "JumpFromLeftWall の条件は自身の 1 件（複数行 1 式）"
    );

    // FallFromWall（壁グループ内 + 自分の条件）: 壁条件と自分の条件の AND
    let (conds, _) = walked
        .iter()
        .find(|(_, b)| b.name == "FallFromWall")
        .expect("FallFromWall");
    let sources = cond_sources(conds.expect("FallFromWall は条件を持つ"));
    assert!(
        sources.contains(&"mascot.environment.wall.isOn(mascot.anchor)".to_string()),
        "壁グループ条件が継承されている: {sources:?}"
    );
    assert!(
        sources.contains(&norm_ws("!mascot.environment.floor.isOn(mascot.anchor)")),
        "FallFromWall 自身の条件: {sources:?}"
    );

    // SplitIntoTwo（床グループ内 + totalCount 条件）
    let (conds, _) = walked
        .iter()
        .find(|(_, b)| b.name == "SplitIntoTwo")
        .expect("SplitIntoTwo");
    let sources = cond_sources(conds.expect("SplitIntoTwo は条件を持つ"));
    assert!(sources.contains(&"mascot.environment.floor.isOn(mascot.anchor)".to_string()));
    assert!(sources.contains(&"mascot.totalCount < 50".to_string()));
}

#[test]
fn real_behaviors_jump_from_left_wall_condition_evaluates() {
    // 複数行の実物条件を評価する（BOM/CRLF/複数行属性を含む実物経由）
    let cfg = real_behaviors();
    let walked = walk_behaviors(&cfg);
    let (conds, _) = walked
        .iter()
        .find(|(_, b)| b.name == "JumpFromLeftWall")
        .expect("JumpFromLeftWall");
    let cond = &conds.expect("条件")[0];
    // 床端でない・左端近く・床付近の高さ → 真
    let mut ctx = MockCtx::new();
    ctx.all_on = false;
    ctx.anchor_x = 100.0;
    ctx.anchor_y = 1000.0;
    ctx.wa_left = 0.0;
    ctx.wa_bottom = 1040.0;
    ctx.wa_height = 1040.0;
    assert!(eval_parsed_bool(&ctx, cond, &[]));
    // 壁に接しているなら偽（!isOn が false）
    let mut ctx_on = MockCtx::new();
    ctx_on.anchor_x = 100.0;
    ctx_on.anchor_y = 1000.0;
    assert!(!eval_parsed_bool(&ctx_on, cond, &[]));
}

#[test]
fn real_behaviors_next_lists_and_references() {
    // asset-report §3-4: NextBehaviorList 7 / BehaviorReference 12 / Add=true は SitDown のみ
    let cfg = real_behaviors();
    let walked = walk_behaviors(&cfg);
    let mut with_next = 0;
    let mut refs_total = 0;
    let mut add_true = 0;
    for (_, b) in &walked {
        if let Some(next) = &b.next {
            with_next += 1;
            refs_total += next.references.len();
            if next.add {
                add_true += 1;
            }
        }
    }
    assert_eq!(with_next, 7);
    assert_eq!(refs_total, 12);
    assert_eq!(add_true, 1);

    // SitDown: Add=true、参照 2 件
    let sitdown = walked
        .iter()
        .find(|(_, b)| b.name == "SitDown")
        .expect("SitDown")
        .1;
    assert_eq!(sitdown.frequency, 200);
    assert!(!sitdown.hidden);
    let next = sitdown
        .next
        .as_ref()
        .expect("SitDown は NextBehaviorList を持つ");
    assert!(next.add);
    assert_eq!(next.references.len(), 2);
    assert_eq!(next.references[0].name, "SitWhileDanglingLegs");
    assert_eq!(next.references[0].frequency, 100);
    assert!(next.references[0].condition.is_none());
    assert_eq!(next.references[1].name, "LieDown");
    assert_eq!(next.references[1].frequency, 100);

    // LieDown: Add=false、参照 3 件（後半 2 件は Condition 付き）
    let liedown = walked
        .iter()
        .find(|(_, b)| b.name == "LieDown")
        .expect("LieDown")
        .1;
    let next = liedown
        .next
        .as_ref()
        .expect("LieDown は NextBehaviorList を持つ");
    assert!(!next.add);
    assert_eq!(next.references.len(), 3);
    assert!(next.references[0].condition.is_none());
    let cond = next.references[1]
        .condition
        .as_ref()
        .expect("CrawlAlongIECeiling 参照は条件付き");
    // 資産: ${mascot.environment.activeIE.topBorder.isOn(mascot.anchor)} → ${} 記法
    match cond {
        Variable::Script {
            allow_value_reset, ..
        } => assert!(!allow_value_reset),
        _ => panic!("参照条件は Script"),
    }
    let ctx = MockCtx::new(); // all_on=true
    assert!(eval_parsed_bool(&ctx, cond, &[]));
    // 3 件目: ${mascot.environment.workArea.bottomBorder.isOn(mascot.anchor)}
    let cond2 = next.references[2]
        .condition
        .as_ref()
        .expect("CrawlAlongWorkAreaFloor 参照は条件付き");
    assert!(eval_parsed_bool(&ctx, cond2, &[]));
    let mut ctx_off = MockCtx::new();
    ctx_off.all_on = false;
    assert!(!eval_parsed_bool(&ctx_off, cond2, &[]));
}

// =====================================================================
// 合成 XML: 正常系・エラー系
// =====================================================================

// =====================================================================
// #32: `<NextBehavior>` 別名（デレマスしめじ v1.9 資産）
// =====================================================================

/// デレマスしめじ v1.9 の Behavior.xml は `<NextBehavior Add="false">` を 120 箇所で
/// 使う（Class 名ではなく短縮要素名）。パーサは `NextBehaviorList` /
/// `NextBehaviourList` しか見ないため、未知の子要素は `_ => {}` で無音に捨てられ、
/// 遷移が黙って壊れる。US 綴りの短縮形も list として読めること。
#[test]
fn synthetic_next_behavior_singular_alias_parses() {
    let xml = concat!(
        "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
        "<BehaviorList>\n",
        "  <Behavior Name=\"A\" Frequency=\"1\">\n",
        "    <NextBehavior Add=\"false\">\n",
        "      <BehaviorReference Name=\"B\" Frequency=\"7\" />\n",
        "    </NextBehavior>\n",
        "  </Behavior>\n",
        "  <Behavior Name=\"B\" Frequency=\"0\"/>\n",
        "</BehaviorList></Mascot>\n"
    );
    let path = temp_conf("next_behavior_alias", xml);
    let cfg = parse_behaviors(&path).expect("NextBehavior 別名を読める");
    let _ = std::fs::remove_file(&path);

    let a = walk_behaviors(&cfg)
        .into_iter()
        .find(|(_, b)| b.name == "A")
        .expect("A が存在する")
        .1;
    let next = a
        .next
        .as_ref()
        .expect("NextBehavior が遷移リストとして読まれる（無音破棄されない）");
    assert!(!next.add, "Add=\"false\" が反映される");
    assert_eq!(next.references.len(), 1);
    assert_eq!(next.references[0].name, "B");
    assert_eq!(next.references[0].frequency, 7);
}

#[test]
fn synthetic_bom_and_crlf_actions_parse() {
    // 資産と同じ UTF-8 BOM + CRLF でも壊れないこと
    let xml = "\u{FEFF}<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\r\n\
               <Mascot xmlns=\"http://www.group-finity.com/Mascot\">\r\n\
               <ActionList>\r\n\
               <Action Name=\"A\" Type=\"Stay\"><Animation><Pose Image=\"/x.png\" ImageAnchor=\"0,0\" Velocity=\"0,0\" Duration=\"1\"/></Animation></Action>\r\n\
               </ActionList>\r\n\
               </Mascot>\r\n";
    let path = temp_conf("bom", xml);
    let result = parse_actions(&path);
    let _ = std::fs::remove_file(&path);
    let cfg = result.expect("BOM + CRLF の actions.xml をパースできる");
    assert_eq!(cfg.actions.len(), 1);
    match cfg.actions.get("A").unwrap() {
        ActionDef::Stay { border, .. } => assert!(
            border.is_none(),
            "BorderType 省略 = None（border 無効・Java BorderedAction.java L24）"
        ),
        _ => panic!("A は Stay"),
    }
}

#[test]
fn synthetic_explicit_wall_and_omission_means_none() {
    let xml = format!(
        "{}\
         <Action Name=\"A\" Type=\"Stay\"><Animation><Pose Image=\"/x.png\" ImageAnchor=\"0,0\" Velocity=\"0,0\" Duration=\"1\"/></Animation></Action>\n\
         <Action Name=\"B\" Type=\"Stay\" BorderType=\"Wall\"><Animation><Pose Image=\"/x.png\" ImageAnchor=\"0,0\" Velocity=\"0,0\" Duration=\"1\"/></Animation></Action>\n\
         </ActionList>\n</Mascot>\n",
        ACTIONS_XML_HEAD
    );
    let path = temp_conf("borders", &xml);
    let result = parse_actions(&path);
    let _ = std::fs::remove_file(&path);
    let cfg = result.expect("Border 属性付き actions.xml をパースできる");
    // A = BorderType 省略 → None（省略時 Floor 折り畳み廃止・design §1.8(g)）
    assert!(border_of(cfg.actions.get("A").unwrap()).is_none());
    // B = 明示 BorderType="Wall" → Some(Wall)（未知値はエラーのまま・別契約）
    assert!(matches!(
        border_of(cfg.actions.get("B").unwrap()),
        Some(BorderType::Wall)
    ));
}

#[test]
fn synthetic_duplicate_action_names_error() {
    let xml = format!(
        "{}\
         <Action Name=\"Same\" Type=\"Stay\"><Animation><Pose Image=\"/x.png\" ImageAnchor=\"0,0\" Velocity=\"0,0\" Duration=\"1\"/></Animation></Action>\n\
         <Action Name=\"Same\" Type=\"Stay\"><Animation><Pose Image=\"/x.png\" ImageAnchor=\"0,0\" Velocity=\"0,0\" Duration=\"1\"/></Animation></Action>\n\
         </ActionList>\n</Mascot>\n",
        ACTIONS_XML_HEAD
    );
    let path = temp_conf("dup_action", &xml);
    let result = parse_actions(&path);
    let _ = std::fs::remove_file(&path);
    assert!(
        result.is_err(),
        "同名 Action は Err（ConfigError にソース位置を含む）"
    );
}

#[test]
fn synthetic_duplicate_behavior_names_error() {
    let xml = concat!(
        "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
        "<BehaviorList>\n",
        "<Behavior Name=\"Same\" Frequency=\"1\"/>\n",
        "<Behavior Name=\"Same\" Frequency=\"2\"/>\n",
        "</BehaviorList>\n</Mascot>\n"
    );
    let path = temp_conf("dup_behavior", xml);
    let result = parse_behaviors(&path);
    let _ = std::fs::remove_file(&path);
    assert!(result.is_err(), "同名 Behavior は Err");
}

#[test]
fn synthetic_malformed_xml_error() {
    // タグ未閉鎖
    let xml = format!("{}<Action Name=\"A\" Type=\"Stay\">", ACTIONS_XML_HEAD);
    let path = temp_conf("malformed", &xml);
    let result = parse_actions(&path);
    let _ = std::fs::remove_file(&path);
    assert!(result.is_err(), "破損 XML は Err");
}

#[test]
fn synthetic_unknown_action_type_error() {
    let xml = format!(
        "{}<Action Name=\"A\" Type=\"Bogus\"/>\n</ActionList>\n</Mascot>\n",
        ACTIONS_XML_HEAD
    );
    let path = temp_conf("bad_type", &xml);
    let result = parse_actions(&path);
    let _ = std::fs::remove_file(&path);
    assert!(
        result.is_err(),
        "未知の Type は Err（Java UnknownActionType 相当）"
    );
}

#[test]
fn synthetic_action_missing_name_error() {
    let xml = format!(
        "{}\
         <Action Type=\"Stay\"><Animation><Pose Image=\"/x.png\" ImageAnchor=\"0,0\" Velocity=\"0,0\" Duration=\"1\"/></Animation></Action>\n\
         </ActionList>\n</Mascot>\n",
        ACTIONS_XML_HEAD
    );
    let path = temp_conf("no_name", &xml);
    let result = parse_actions(&path);
    let _ = std::fs::remove_file(&path);
    assert!(result.is_err(), "Name 属性欠落は Err（Java 必須属性相当）");
}

#[test]
fn synthetic_pose_missing_required_attrs_error() {
    // ImageAnchor 欠落（asset-report §4-9: Image があるとき ImageAnchor は必須）
    let xml1 = format!(
        "{}\
         <Action Name=\"A\" Type=\"Stay\"><Animation><Pose Image=\"/x.png\" Velocity=\"0,0\" Duration=\"1\"/></Animation></Action>\n\
         </ActionList>\n</Mascot>\n",
        ACTIONS_XML_HEAD
    );
    let path = temp_conf("pose_no_anchor", &xml1);
    let result = parse_actions(&path);
    let _ = std::fs::remove_file(&path);
    assert!(result.is_err(), "ImageAnchor 欠落の Pose は Err");

    // Velocity 欠落
    let xml2 = format!(
        "{}\
         <Action Name=\"A\" Type=\"Stay\"><Animation><Pose Image=\"/x.png\" ImageAnchor=\"0,0\" Duration=\"1\"/></Animation></Action>\n\
         </ActionList>\n</Mascot>\n",
        ACTIONS_XML_HEAD
    );
    let path = temp_conf("pose_no_velocity", &xml2);
    let result = parse_actions(&path);
    let _ = std::fs::remove_file(&path);
    assert!(result.is_err(), "Velocity 欠落の Pose は Err");
}

#[test]
fn synthetic_constant_definitions_parse_ja_and_en() {
    let xml = concat!(
        "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
        "  <Constant Name=\"speed\" Value=\"3\"/>\n",
        "  <定数 Name=\"maxCount\" 値=\"5\" />\n",
        "  <Constant Name=\"flag\" Value=\"true\"/>\n",
        "<BehaviorList>\n",
        "<Behavior Name=\"A\" Frequency=\"1\"/>\n",
        "</BehaviorList>\n</Mascot>\n"
    );
    let path = temp_conf("constants", xml);
    let result = parse_behaviors(&path);
    let _ = std::fs::remove_file(&path);
    let cfg = result.expect("Constant / 定数 を含む behaviors をパースできる");
    assert_eq!(cfg.constants.get("speed").map(String::as_str), Some("3"));
    assert_eq!(cfg.constants.get("maxCount").map(String::as_str), Some("5"));
    assert_eq!(cfg.constants.get("flag").map(String::as_str), Some("true"));
}

#[test]
fn synthetic_constant_missing_value_error() {
    let xml = concat!(
        "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
        "<定数 Name=\"maxCount\" />\n",
        "<BehaviorList><Behavior Name=\"A\" Frequency=\"1\"/></BehaviorList>\n</Mascot>\n"
    );
    let path = temp_conf("constant_no_value", xml);
    let result = parse_behaviors(&path);
    let _ = std::fs::remove_file(&path);
    assert!(result.is_err(), "Value 属性欠落は Err（Java 必須属性相当）");
}

#[test]
fn synthetic_behavior_missing_frequency_error() {
    let xml = concat!(
        "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
        "<BehaviorList>\n",
        "<Behavior Name=\"X\"/>\n",
        "</BehaviorList>\n</Mascot>\n"
    );
    let path = temp_conf("no_frequency", xml);
    let result = parse_behaviors(&path);
    let _ = std::fs::remove_file(&path);
    assert!(result.is_err(), "Frequency 欠落は Err（Java 必須属性相当）");
}

#[test]
fn synthetic_nested_condition_groups_accumulate() {
    // Java loadBehaviors の再帰相当: 入れ子 Condition は AND 積み上げになる
    let xml = concat!(
        "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
        "<BehaviorList>\n",
        "<Condition Condition=\"#{mascot.totalCount &lt; 50}\">\n",
        "<Behavior Name=\"Outer\" Frequency=\"10\"/>\n",
        "<Condition Condition=\"${mascot.lookRight}\">\n",
        "<Behavior Name=\"Inner\" Frequency=\"20\"/>\n",
        "</Condition>\n",
        "<Behavior Name=\"AfterInner\" Frequency=\"30\"/>\n",
        "</Condition>\n",
        "</BehaviorList>\n</Mascot>\n"
    );
    let path = temp_conf("nested_cond", xml);
    let result = parse_behaviors(&path);
    let _ = std::fs::remove_file(&path);
    let cfg = result.expect("入れ子 Condition を含む behaviors をパースできる");
    let walked = walk_behaviors(&cfg);
    assert_eq!(walked.len(), 3, "Outer / Inner / AfterInner");

    let conds_of = |name: &str| -> Vec<String> {
        walked
            .iter()
            .find(|(_, b)| b.name == name)
            .unwrap_or_else(|| panic!("Behavior {name}"))
            .0
            .map(cond_sources)
            .unwrap_or_default()
    };
    // Inner は外側 + 内側の両方（AND 積み上げ、順序保持）
    assert_eq!(
        conds_of("Inner"),
        vec![
            "mascot.totalCount < 50".to_string(),
            "mascot.lookRight".to_string()
        ]
    );
    // 外側のみ
    assert_eq!(
        conds_of("Outer"),
        vec!["mascot.totalCount < 50".to_string()]
    );
    // 内側 Condition が閉じた後の Behavior は外側の条件のみ
    assert_eq!(
        conds_of("AfterInner"),
        vec!["mascot.totalCount < 50".to_string()]
    );
}

#[test]
fn synthetic_validate_required_behaviors_missing_thrown() {
    // Fall/Dragged/Thrown のうち Thrown が欠落 → Err
    let xml = concat!(
        "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
        "<BehaviorList>\n",
        "<Behavior Name=\"ChaseMouse\" Frequency=\"0\" Hidden=\"true\"/>\n",
        "<Behavior Name=\"Fall\" Frequency=\"0\" Hidden=\"true\"/>\n",
        "<Behavior Name=\"Dragged\" Frequency=\"0\" Hidden=\"true\"/>\n",
        "</BehaviorList>\n</Mascot>\n"
    );
    let path = temp_conf("missing_thrown", xml);
    let result = parse_behaviors(&path);
    let _ = std::fs::remove_file(&path);
    let cfg = result.expect("XML 自体は妥当");
    assert!(
        validate_required_behaviors(&cfg).is_err(),
        "Thrown 欠落は validate_required_behaviors で Err"
    );

    // 4 種すべて揃っていれば Ok
    let xml_ok = concat!(
        "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
        "<BehaviorList>\n",
        "<Behavior Name=\"ChaseMouse\" Frequency=\"0\" Hidden=\"true\"/>\n",
        "<Behavior Name=\"Fall\" Frequency=\"0\" Hidden=\"true\"/>\n",
        "<Behavior Name=\"Dragged\" Frequency=\"0\" Hidden=\"true\"/>\n",
        "<Behavior Name=\"Thrown\" Frequency=\"0\" Hidden=\"true\"/>\n",
        "</BehaviorList>\n</Mascot>\n"
    );
    let path = temp_conf("all_required", xml_ok);
    let result = parse_behaviors(&path);
    let _ = std::fs::remove_file(&path);
    let cfg = result.expect("XML 自体は妥当");
    validate_required_behaviors(&cfg).expect("必須 4 種が揃っていれば Ok");
}

// =====================================================================
// #21: XML 要素ネスト深さガード（パーサ再帰で abort しない）
//
// 要素階層ごとに再帰する parse_action_def（Inline Action）/
// parse_behavior_list（Condition）/ parse_next_list_children（NextBehaviorList 配下
// の Condition）は、病的に深い XML でスタックオーバーフローする。深さ上限超過時は
// ConfigError（Result）を返し、プロセスを落とさない（XML 破損 = Result 終了の既存契約）。
// 正常な実 conf は depth が十分浅く従来どおりパースできる（既存テストで担保）。
// =====================================================================

/// 契約 4a: 深くネストした Inline Action は abort せず Err。
#[test]
fn deeply_nested_inline_actions_error_not_stack_overflow() {
    const DEPTH: usize = 50_000;
    let mut xml = String::with_capacity(DEPTH * 64);
    xml.push_str(ACTIONS_XML_HEAD);
    xml.push_str("<Action Name=\"A0\" Type=\"Sequence\">");
    for _ in 0..DEPTH {
        xml.push_str("<Action Type=\"Sequence\">");
    }
    xml.push_str("<ActionReference Name=\"X\"/>");
    for _ in 0..DEPTH {
        xml.push_str("</Action>");
    }
    xml.push_str("</Action></ActionList></Mascot>");

    let path = temp_conf("deep_inline_actions", &xml);
    let result = parse_actions(&path);
    let _ = std::fs::remove_file(&path);
    assert!(
        result.is_err(),
        "5 万階層の Inline Action ネストは深さ上限超過で Err（abort しない）"
    );
}

/// 契約 4b: 深くネストした Condition（behaviors）は abort せず Err。
#[test]
fn deeply_nested_behavior_conditions_error_not_stack_overflow() {
    const DEPTH: usize = 50_000;
    let mut xml = String::with_capacity(DEPTH * 40);
    xml.push_str("<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n<BehaviorList>\n");
    for _ in 0..DEPTH {
        xml.push_str("<Condition>");
    }
    xml.push_str("<Behavior Name=\"B\" Frequency=\"1\"/>");
    for _ in 0..DEPTH {
        xml.push_str("</Condition>");
    }
    xml.push_str("</BehaviorList></Mascot>");

    let path = temp_conf("deep_behavior_conditions", &xml);
    let result = parse_behaviors(&path);
    let _ = std::fs::remove_file(&path);
    assert!(
        result.is_err(),
        "5 万階層の Condition ネストは深さ上限超過で Err（abort しない）"
    );
}

/// 契約 4c: 深くネストした Condition（NextBehaviorList 配下）は abort せず Err。
#[test]
fn deeply_nested_next_list_conditions_error_not_stack_overflow() {
    const DEPTH: usize = 50_000;
    let mut xml = String::with_capacity(DEPTH * 30);
    xml.push_str("<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n<BehaviorList>\n");
    xml.push_str("<Behavior Name=\"B\" Frequency=\"1\">\n<NextBehaviorList Add=\"true\">\n");
    for _ in 0..DEPTH {
        xml.push_str("<Condition>");
    }
    xml.push_str("<BehaviorReference Name=\"R\" Frequency=\"1\"/>");
    for _ in 0..DEPTH {
        xml.push_str("</Condition>");
    }
    xml.push_str("</NextBehaviorList></Behavior></BehaviorList></Mascot>");

    let path = temp_conf("deep_next_list_conditions", &xml);
    let result = parse_behaviors(&path);
    let _ = std::fs::remove_file(&path);
    assert!(
        result.is_err(),
        "NextBehaviorList 配下の 5 万階層 Condition ネストは Err（abort しない）"
    );
}

#[test]
fn parsed_expressions_are_evaluable_via_public_api() {
    // パース結果の Variable がそのまま Variables::eval に流せること（config ↔ script の接続）
    let cfg = real_actions();
    let throwing = find_action(&cfg, "ThrowIEFromLeft");
    // <ActionReference Name="Jumping" TargetX="${mascot.environment.activeIE.left+6}" TargetY="${mascot.environment.activeIE.bottom+58}"/>
    let jumping = match &children_of(throwing)[0] {
        SequenceChild::Ref { name, attrs } => {
            assert_eq!(name, "Jumping");
            attrs
        }
        _ => panic!("ThrowIEFromLeft の第 1 子は Ref(Jumping)"),
    };
    let ctx = MockCtx::new(); // activeIE.left=200, bottom=900
    assert_eq!(
        eval_parsed_num(&ctx, jumping.get("TargetX").unwrap(), &[]),
        206.0
    );
    assert_eq!(
        eval_parsed_num(&ctx, jumping.get("TargetY").unwrap(), &[]),
        958.0
    );
    // 評価結果の EvalValue を as_number で取り出せる
    match eval_ok(&ctx, jumping.get("TargetX").unwrap()) {
        EvalValue::Number(n) => assert_eq!(n, 206.0),
        _ => panic!("TargetX は数値のはずが別の型"),
    }
}

// =====================================================================
// タスク #7b: Loop 属性（Sequence/Select）と IsTurn 属性（Animation）のパース
// =====================================================================

/// 資産 actions.xml の Loop 属性 55 箇所が ActionDef::{Sequence,Select}.is_loop に
/// パースされる。Dragged は Loop="true"・Fall は "false"（資産 L315/L303）。
#[test]
fn real_actions_loop_parsed_55_total_with_dragged_true_fall_false() {
    // ファイル内の Loop= 総数（asset-report §3-2・55 箇所）
    let text = std::fs::read_to_string(conf_path("actions.xml")).unwrap();
    assert_eq!(
        text.matches("Loop=").count(),
        55,
        "資産の Loop 属性は 55 箇所"
    );

    let cfg = real_actions();
    let mut loop_total = 0usize;
    for def in cfg.actions.values() {
        let is_loop = match def {
            ActionDef::Sequence { is_loop, .. } => *is_loop,
            ActionDef::Select { is_loop, .. } => *is_loop,
            _ => continue,
        };
        let _ = is_loop;
        loop_total += 1;
    }
    assert_eq!(
        loop_total, 59,
        "全 Sequence/Select 定義が Loop（明示 55 + 省略 4）と無関係なく is_loop を持つ"
    );

    match find_action(&cfg, "Dragged") {
        ActionDef::Sequence { is_loop, .. } => assert!(*is_loop, "Dragged は Loop true"),
        _ => panic!("Dragged は Sequence"),
    }
    match find_action(&cfg, "Fall") {
        ActionDef::Sequence { is_loop, .. } => assert!(!*is_loop, "Fall は Loop false"),
        _ => panic!("Fall は Sequence"),
    }
}

/// 合成 XML: Sequence/Select の Loop 属性と Animation の IsTurn 属性
/// （Animation.java L89-90 契約相当・資産使用 0 件のため合成で pin）。
#[test]
fn synthetic_loop_and_is_turn_parse_sequence_select() {
    let xml = format!(
        "{}\
         <Action Name=\"S\" Type=\"Sequence\" Loop=\"true\">\
         <Action Name=\"I\" Type=\"Animate\" Loop=\"false\">\
         <Animation IsTurn=\"true\"><Pose Image=\"/x.png\" ImageAnchor=\"0,0\" Velocity=\"0,0\" Duration=\"1\"/></Animation>\
         </Action>\
         <ActionReference Name=\"I\"/>\
         </Action>\
         <Action Name=\"Q\" Type=\"Select\"><Animation><Pose Image=\"/y.png\" ImageAnchor=\"0,0\" Velocity=\"0,0\" Duration=\"1\"/></Animation><ActionReference Name=\"I\"/></Action>\
         </ActionList>\n</Mascot>\n",
        ACTIONS_XML_HEAD
    );
    let path = temp_conf("loop_isturn", &xml);
    let result = parse_actions(&path);
    let _ = std::fs::remove_file(&path);
    let cfg = result.expect("Loop / IsTurn 属性でパースできる");
    match cfg.actions.get("S").unwrap() {
        ActionDef::Sequence { is_loop, .. } => assert!(
            *is_loop,
            "Sequence の Loop 属性は ActionDef.is_loop に保持される"
        ),
        _ => panic!("S は Sequence"),
    }
    match cfg.actions.get("Q").unwrap() {
        ActionDef::Select { is_loop, .. } => {
            assert!(!*is_loop, "Loop 属性が無い Select は既定 false");
        }
        _ => panic!("Q は Select"),
    }
    // Inline Action（匿名 Action）の Animation.is_turn: IsTurn="true" → true
    match cfg.actions.get("S").unwrap() {
        ActionDef::Sequence { children, .. } => match &children[0] {
            SequenceChild::Inline(inner) => match inner.as_ref() {
                ActionDef::Animate { animations, .. } => assert!(
                    animations[0].is_turn,
                    "Animation の IsTurn 属性は is_turn へ（AnimationBuilder.java L89-90 契約）"
                ),
                _ => panic!("Inline は Animate"),
            },
            _ => panic!("S[0] は Inline"),
        },
        _ => panic!(),
    }
}

// =====================================================================
// #5 の stub アクション参照検出は #35（全アクション本体実装）で撤去した
// =====================================================================
